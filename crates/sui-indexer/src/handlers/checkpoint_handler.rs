// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::handlers::committer::start_tx_checkpoint_commit_task;
use crate::models::display::StoredDisplay;
use crate::{CheckpointHandlerConfig, Config};
use async_trait::async_trait;
use itertools::Itertools;
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::TypeTag;
use mysten_metrics::{get_metrics, spawn_monitored_task};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use sui_rest_api::{CheckpointData, CheckpointTransaction, Client};
use sui_types::base_types::{ExecutionDigests, ObjectRef};
use sui_types::dynamic_field::DynamicFieldInfo;
use sui_types::dynamic_field::DynamicFieldType;
use sui_types::messages_checkpoint::{
    CertifiedCheckpointSummary, CheckpointContents, CheckpointSequenceNumber,
};
use sui_types::object::Object;
use sui_types::signature::GenericSignature;
use tokio_util::sync::CancellationToken;

use std::collections::hash_map::Entry;
use std::collections::HashSet;
use sui_data_ingestion_core::Worker;
use sui_types::base_types::SequenceNumber;
use sui_types::effects::{TransactionEffects, TransactionEffectsAPI};
use sui_types::event::SystemEpochInfoEvent;
use sui_types::object::Owner;
use sui_types::transaction::TransactionDataAPI;
use tracing::{info, warn};

use sui_types::base_types::ObjectID;
use sui_types::sui_system_state::sui_system_state_summary::SuiSystemStateSummary;
use sui_types::sui_system_state::{get_sui_system_state, SuiSystemStateTrait};

use crate::errors::IndexerError;
use crate::metrics::IndexerMetrics;

use crate::store::IndexerStore;
use crate::types::{
    DynamicFieldInfoShort, IndexedCheckpoint, IndexedDeletedObject, IndexedEpochInfo, IndexedEvent,
    IndexedObject, IndexedPackage, IndexedTransaction, IndexerResult, TransactionKind, TxIndex,
};

use super::tx_processor::EpochEndIndexingObjectStore;
use super::tx_processor::TxChangesProcessor;
use super::CheckpointDataToCommit;
use super::EpochToCommit;
use super::TransactionObjectChangesToCommit;

pub async fn new_handlers<S>(
    state: S,
    client: Client,
    metrics: IndexerMetrics,
    next_checkpoint_sequence_number: CheckpointSequenceNumber,
    cancel: CancellationToken,
    config: &Config,
) -> Result<CheckpointHandler<S>, IndexerError>
where
    S: IndexerStore + Clone + Sync + Send + 'static,
{
    let checkpoint_queue_size = config.checkpoint_handler.queue_size;
    let global_metrics = get_metrics().unwrap();
    let (indexed_checkpoint_sender, indexed_checkpoint_receiver) =
        mysten_metrics::metered_channel::channel(
            checkpoint_queue_size,
            &global_metrics
                .channels
                .with_label_values(&["checkpoint_indexing"]),
        );

    let state_clone = state.clone();
    let client_clone = client.clone();
    let metrics_clone = metrics.clone();
    let config_clone = config.clone();
    spawn_monitored_task!(start_tx_checkpoint_commit_task(
        state_clone,
        client_clone,
        metrics_clone,
        indexed_checkpoint_receiver,
        next_checkpoint_sequence_number,
        cancel.clone(),
        config_clone,
    ));

    Ok(CheckpointHandler::new(
        state,
        metrics,
        indexed_checkpoint_sender,
        &config.checkpoint_handler,
    ))
}

pub struct CheckpointHandler<S> {
    state: S,
    metrics: IndexerMetrics,
    filter_packages: Option<Vec<ObjectID>>,
    indexed_checkpoint_sender: mysten_metrics::metered_channel::Sender<CheckpointDataToCommit>,
}

#[async_trait]
impl<S> Worker for CheckpointHandler<S>
where
    S: IndexerStore + Clone + Sync + Send + 'static,
{
    async fn process_checkpoint(&self, checkpoint: CheckpointData) -> anyhow::Result<()> {
        let checkpoint_data = Self::data_to_commit(
            checkpoint,
            self.state.clone(),
            &self.metrics,
            self.filter_packages.as_ref().map(|v| v.as_ref()),
        )
        .await?;
        self.indexed_checkpoint_sender.send(checkpoint_data).await?;
        Ok(())
    }

    fn preprocess_hook(&self, checkpoint: CheckpointData) -> anyhow::Result<()> {
        let package_objects = Self::get_package_objects(&[checkpoint]);
        for (
            IndexedPackage {
                package_id,
                checkpoint_sequence_number,
                ..
            },
            _,
        ) in package_objects
        {
            info!("Package {package_id} published in checkpoint {checkpoint_sequence_number}");
        }
        Ok(())
    }
}

impl<S> CheckpointHandler<S>
where
    S: IndexerStore + Clone + Sync + Send + 'static,
{
    fn new(
        state: S,
        metrics: IndexerMetrics,
        indexed_checkpoint_sender: mysten_metrics::metered_channel::Sender<CheckpointDataToCommit>,
        config: &CheckpointHandlerConfig,
    ) -> Self {
        Self {
            state,
            metrics,
            filter_packages: config.filter_packages.clone(),
            indexed_checkpoint_sender,
        }
    }

    pub(crate) async fn data_to_commit(
        checkpoint: CheckpointData,
        state: S,
        metrics: &IndexerMetrics,
        filter: Option<&[ObjectID]>,
    ) -> Result<CheckpointDataToCommit, IndexerError> {
        let checkpoint = if let Some(filter) = filter {
            Self::filter_transactions(checkpoint, filter)
        } else {
            checkpoint
        };
        let index_packages = Self::index_packages(&checkpoint, metrics);
        Self::index_checkpoint(
            state.into(),
            checkpoint,
            Arc::new(metrics.clone()),
            index_packages,
        )
        .await
    }

    #[tracing::instrument(skip_all)]
    async fn index_epoch(
        state: Arc<S>,
        data: &CheckpointData,
    ) -> Result<Option<EpochToCommit>, IndexerError> {
        let checkpoint_object_store = EpochEndIndexingObjectStore::new(data);

        let CheckpointData {
            transactions,
            checkpoint_summary,
            checkpoint_contents: _,
        } = data;

        // Genesis epoch
        if *checkpoint_summary.sequence_number() == 0 {
            info!("Processing genesis epoch");
            let system_state: SuiSystemStateSummary =
                get_sui_system_state(&checkpoint_object_store)?.into_sui_system_state_summary();
            return Ok(Some(EpochToCommit {
                last_epoch: None,
                new_epoch: IndexedEpochInfo::from_new_system_state_summary(
                    system_state,
                    0, //first_checkpoint_id
                    None,
                ),
            }));
        }

        // If not end of epoch, return
        if checkpoint_summary.end_of_epoch_data.is_none() {
            return Ok(None);
        }

        let system_state: SuiSystemStateSummary =
            get_sui_system_state(&checkpoint_object_store)?.into_sui_system_state_summary();

        let epoch_event = transactions
            .iter()
            .flat_map(|t| t.events.as_ref().map(|e| &e.data))
            .flatten()
            .find(|ev| ev.is_system_epoch_info_event())
            .unwrap_or_else(|| {
                panic!(
                    "Can't find SystemEpochInfoEvent in epoch end checkpoint {}",
                    checkpoint_summary.sequence_number()
                )
            });

        let event = bcs::from_bytes::<SystemEpochInfoEvent>(&epoch_event.contents)?;

        // Now we just entered epoch X, we want to calculate the diff between
        // TotalTransactionsByEndOfEpoch(X-1) and TotalTransactionsByEndOfEpoch(X-2)
        let network_tx_count_prev_epoch = match system_state.epoch {
            // If first epoch change, this number is 0
            1 => Ok(0),
            _ => {
                let last_epoch = system_state.epoch - 2;
                state
                    .get_network_total_transactions_by_end_of_epoch(last_epoch)
                    .await
            }
        }?;

        Ok(Some(EpochToCommit {
            last_epoch: Some(IndexedEpochInfo::from_end_of_epoch_data(
                &system_state,
                checkpoint_summary,
                &event,
                network_tx_count_prev_epoch,
            )),
            new_epoch: IndexedEpochInfo::from_new_system_state_summary(
                system_state,
                checkpoint_summary.sequence_number + 1, // first_checkpoint_id
                Some(&event),
            ),
        }))
    }

    #[tracing::instrument(skip_all)]
    pub(crate) async fn index_checkpoint(
        state: Arc<S>,
        data: CheckpointData,
        metrics: Arc<IndexerMetrics>,
        packages: Vec<IndexedPackage>,
    ) -> Result<CheckpointDataToCommit, IndexerError> {
        let checkpoint_seq = data.checkpoint_summary.sequence_number;
        info!(checkpoint_seq, "Indexing checkpoint data blob");

        // Index epoch
        let epoch = Self::index_epoch(state, &data).await?;

        // Index Objects
        let object_changes: TransactionObjectChangesToCommit =
            Self::index_objects(data.clone(), &metrics).await?;
        let object_history_changes: TransactionObjectChangesToCommit =
            Self::index_objects_history(data.clone()).await?;

        let (checkpoint, db_transactions, db_events, db_indices, db_displays) = {
            let CheckpointData {
                transactions,
                checkpoint_summary,
                checkpoint_contents,
            } = data;

            let (db_transactions, db_events, db_indices, db_displays) = Self::index_transactions(
                transactions,
                &checkpoint_summary,
                &checkpoint_contents,
                &metrics,
            )
            .await?;

            let successful_tx_num: u64 = db_transactions.iter().map(|t| t.successful_tx_num).sum();
            (
                IndexedCheckpoint::from_sui_checkpoint(
                    &checkpoint_summary,
                    &checkpoint_contents,
                    successful_tx_num as usize,
                ),
                db_transactions,
                db_events,
                db_indices,
                db_displays,
            )
        };
        info!(checkpoint_seq, "Indexed one checkpoint.");
        metrics
            .index_lag_ms
            .set(chrono::Utc::now().timestamp_millis() - checkpoint.timestamp_ms as i64);

        Ok(CheckpointDataToCommit {
            checkpoint,
            transactions: db_transactions,
            events: db_events,
            tx_indices: db_indices,
            display_updates: db_displays,
            object_changes,
            object_history_changes,
            packages,
            epoch,
        })
    }

    #[tracing::instrument(skip_all)]
    async fn index_transactions(
        transactions: Vec<CheckpointTransaction>,
        checkpoint_summary: &CertifiedCheckpointSummary,
        checkpoint_contents: &CheckpointContents,
        metrics: &IndexerMetrics,
    ) -> IndexerResult<(
        Vec<IndexedTransaction>,
        Vec<IndexedEvent>,
        Vec<TxIndex>,
        BTreeMap<String, StoredDisplay>,
    )> {
        let checkpoint_seq = checkpoint_summary.sequence_number();

        let mut tx_seq_num_iter = checkpoint_contents
            .enumerate_transactions(checkpoint_summary)
            .map(|(seq, execution_digest)| (execution_digest.transaction, seq));

        if checkpoint_contents.size() != transactions.len() {
            return Err(IndexerError::FullNodeReadingError(format!(
                "CheckpointContents has different size {} compared to Transactions {} for checkpoint {}",
                checkpoint_contents.size(),
                transactions.len(),
                checkpoint_seq
            )));
        }

        let mut db_transactions = Vec::new();
        let mut db_events = Vec::new();
        let mut db_displays = BTreeMap::new();
        let mut db_indices = Vec::new();

        for tx in transactions {
            let CheckpointTransaction {
                transaction: sender_signed_data,
                effects: fx,
                events,
                input_objects,
                output_objects,
            } = tx;
            // Unwrap safe - we checked they have equal length above
            let (tx_digest, tx_sequence_number) = tx_seq_num_iter.next().unwrap();
            if tx_digest != *sender_signed_data.digest() {
                return Err(IndexerError::FullNodeReadingError(format!(
                    "Transactions has different ordering from CheckpointContents, for checkpoint {}, Mismatch found at {} v.s. {}",
                    checkpoint_seq, tx_digest, sender_signed_data.digest()
                )));
            }
            let tx = sender_signed_data.transaction_data();
            let events = events
                .as_ref()
                .map(|events| events.data.clone())
                .unwrap_or_default();

            let transaction_kind = if tx.is_system_tx() {
                TransactionKind::SystemTransaction
            } else {
                TransactionKind::ProgrammableTransaction
            };

            db_events.extend(events.iter().enumerate().map(|(idx, event)| {
                IndexedEvent::from_event(
                    tx_sequence_number,
                    idx as u64,
                    *checkpoint_seq,
                    tx_digest,
                    event,
                    checkpoint_summary.timestamp_ms,
                )
            }));

            db_displays.extend(
                events
                    .iter()
                    .flat_map(StoredDisplay::try_from_event)
                    .map(|display| (display.object_type.clone(), display)),
            );

            let objects = input_objects
                .iter()
                .chain(output_objects.iter())
                .collect::<Vec<_>>();

            let (balance_change, object_changes) =
                TxChangesProcessor::new(&objects, metrics.clone())
                    .get_changes(tx, &fx, &tx_digest)
                    .await?;

            let db_txn = IndexedTransaction {
                tx_sequence_number,
                tx_digest,
                checkpoint_sequence_number: *checkpoint_summary.sequence_number(),
                timestamp_ms: checkpoint_summary.timestamp_ms,
                sender_signed_data: sender_signed_data.data().clone(),
                effects: fx.clone(),
                object_changes,
                balance_change,
                events,
                transaction_kind,
                successful_tx_num: if fx.status().is_ok() {
                    tx.kind().tx_count() as u64
                } else {
                    0
                },
            };

            db_transactions.push(db_txn);

            // Input Objects
            let input_objects = tx
                .input_objects()
                .expect("committed txns have been validated")
                .into_iter()
                .map(|obj_kind| obj_kind.object_id())
                .collect::<Vec<_>>();

            // Changed Objects
            let changed_objects = fx
                .all_changed_objects()
                .into_iter()
                .map(|(object_ref, _owner, _write_kind)| object_ref.0)
                .collect::<Vec<_>>();

            // Payers
            let payers = vec![tx.gas_owner()];

            // Senders
            let senders = vec![tx.sender()];

            // Recipients
            let recipients = fx
                .all_changed_objects()
                .into_iter()
                .filter_map(|(_object_ref, owner, _write_kind)| match owner {
                    Owner::AddressOwner(address) => Some(address),
                    _ => None,
                })
                .unique()
                .collect::<Vec<_>>();

            // Move Calls
            let move_calls = tx
                .move_calls()
                .iter()
                .map(|(p, m, f)| (*<&ObjectID>::clone(p), m.to_string(), f.to_string()))
                .collect();

            db_indices.push(TxIndex {
                tx_sequence_number,
                transaction_digest: tx_digest,
                checkpoint_sequence_number: *checkpoint_seq,
                input_objects,
                changed_objects,
                senders,
                payers,
                recipients,
                move_calls,
            });
        }
        Ok((db_transactions, db_events, db_indices, db_displays))
    }

    #[tracing::instrument(skip_all)]
    async fn index_objects(
        data: CheckpointData,
        metrics: &IndexerMetrics,
    ) -> Result<TransactionObjectChangesToCommit, IndexerError> {
        let _timer = metrics.indexing_objects_latency.start_timer();
        let checkpoint_seq = data.checkpoint_summary.sequence_number;
        let deleted_objects = data
            .transactions
            .iter()
            .flat_map(|tx| get_deleted_objects(&tx.effects))
            .collect::<Vec<_>>();
        let deleted_object_ids = deleted_objects
            .iter()
            .map(|o| (o.0, o.1))
            .collect::<HashSet<_>>();
        let indexed_deleted_objects = deleted_objects
            .into_iter()
            .map(|o| IndexedDeletedObject {
                object_id: o.0,
                object_version: o.1.value(),
                checkpoint_sequence_number: checkpoint_seq,
            })
            .collect();

        let (latest_objects, intermediate_versions) = get_latest_objects(data.output_objects());

        let live_objects: Vec<Object> = data
            .transactions
            .iter()
            .flat_map(|tx| {
                let CheckpointTransaction {
                    transaction: tx,
                    effects: fx,
                    ..
                } = tx;
                fx.all_changed_objects()
                    .into_iter()
                    .filter_map(|(oref, _owner, _kind)| {
                        // We don't care about objects that are deleted or updated more than once
                        if intermediate_versions.contains(&(oref.0, oref.1))
                            || deleted_object_ids.contains(&(oref.0, oref.1))
                        {
                            return None;
                        }
                        let object = latest_objects.get(&(oref.0)).unwrap_or_else(|| {
                            panic!(
                                "object {:?} not found in CheckpointData (tx_digest: {})",
                                oref.0,
                                tx.digest()
                            )
                        });
                        assert_eq!(oref.1, object.version());
                        Some(object.clone())
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        let changed_objects = live_objects
            .into_iter()
            .map(|o| {
                try_create_dynamic_field_info(&o)
                    .map(|info| IndexedObject::from_object(checkpoint_seq, o, info))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(TransactionObjectChangesToCommit {
            changed_objects,
            deleted_objects: indexed_deleted_objects,
        })
    }

    // similar to index_objects, but objects_history keeps all versions of objects
    #[tracing::instrument(skip_all)]
    async fn index_objects_history(
        data: CheckpointData,
    ) -> Result<TransactionObjectChangesToCommit, IndexerError> {
        let checkpoint_seq = data.checkpoint_summary.sequence_number;
        let deleted_objects = data
            .transactions
            .iter()
            .flat_map(|tx| get_deleted_objects(&tx.effects))
            .collect::<Vec<_>>();
        let indexed_deleted_objects: Vec<IndexedDeletedObject> = deleted_objects
            .into_iter()
            .map(|o| IndexedDeletedObject {
                object_id: o.0,
                object_version: o.1.value(),
                checkpoint_sequence_number: checkpoint_seq,
            })
            .collect();

        let history_object_map = data
            .output_objects()
            .into_iter()
            .map(|o| ((o.id(), o.version()), o.clone()))
            .collect::<HashMap<_, _>>();

        let history_objects: Vec<Object> = data
            .transactions
            .iter()
            .flat_map(|tx| {
                let CheckpointTransaction {
                    transaction: tx,
                    effects: fx,
                    ..
                } = tx;
                fx.all_changed_objects()
                    .into_iter()
                    .map(|(oref, _owner, _kind)| {
                        let history_object = history_object_map.get(&(oref.0, oref.1)).unwrap_or_else(|| {
                            panic!(
                                "object {:?} version {:?} not found in CheckpointData (tx_digest: {})",
                                oref.0,
                                oref.1,
                                tx.digest()
                            )
                        });
                        assert_eq!(oref.2, history_object.digest());
                        history_object.clone()
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        let changed_objects = history_objects
            .into_iter()
            .map(|o| {
                try_create_dynamic_field_info(&o)
                    .map(|info| IndexedObject::from_object(checkpoint_seq, o, info))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(TransactionObjectChangesToCommit {
            changed_objects,
            deleted_objects: indexed_deleted_objects,
        })
    }

    #[tracing::instrument(skip_all)]
    fn filter_transactions(
        CheckpointData {
            checkpoint_summary,
            checkpoint_contents,
            transactions,
        }: CheckpointData,
        filter_packages: &[ObjectID],
    ) -> CheckpointData {
        let ((digests, sigs), transactions): ((Vec<_>, _), _) = checkpoint_contents
            .into_iter_with_signatures()
            .zip(transactions)
            .filter(|d| Self::keep_transaction(filter_packages, d))
            .unzip();
        let checkpoint_contents =
            CheckpointContents::new_with_digests_and_signatures(digests, sigs);
        CheckpointData {
            checkpoint_summary,
            checkpoint_contents,
            transactions,
        }
    }

    #[tracing::instrument(skip_all)]
    fn keep_transaction(
        filter: &[ObjectID],
        (_, tx): &(
            (ExecutionDigests, Vec<GenericSignature>),
            CheckpointTransaction,
        ),
    ) -> bool {
        use sui_types::object::Data;
        use sui_types::transaction::TransactionKind;
        if !matches!(
            tx.transaction.data().transaction_data().kind(),
            TransactionKind::ProgrammableTransaction(_)
        ) {
            tracing::info!("Committing non-PTB: {}", tx.effects.transaction_digest());
            return true;
        }
        tx.output_objects.iter().any(|o| match &o.as_inner().data {
            Data::Move(m) => {
                let object_pkg = m.type_().address();
                let touches_pkgs = filter
                    .iter()
                    .any(|pkg_id| Self::is_object_from_package(&object_pkg, pkg_id));
                if touches_pkgs {
                    tracing::info!(
                        "Found transaction {} touching a filtered package",
                        tx.effects.transaction_digest()
                    );
                }
                touches_pkgs
            }
            _ => false,
        })
    }

    fn is_object_from_package(object_pkg: &AccountAddress, pkg_id: &AccountAddress) -> bool {
        object_pkg == pkg_id
    }

    pub(crate) fn index_packages(
        checkpoint_data: &CheckpointData,
        metrics: &IndexerMetrics,
    ) -> Vec<IndexedPackage> {
        let _timer = metrics.indexing_packages_latency.start_timer();
        let checkpoint_sequence_number = checkpoint_data.checkpoint_summary.sequence_number;
        checkpoint_data
            .output_objects()
            .iter()
            .filter_map(|o| {
                if let sui_types::object::Data::Package(p) = &o.data {
                    Some(IndexedPackage {
                        package_id: o.id(),
                        move_package: p.clone(),
                        checkpoint_sequence_number,
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    fn get_package_objects(checkpoint_data: &[CheckpointData]) -> Vec<(IndexedPackage, Object)> {
        checkpoint_data
            .iter()
            .flat_map(|data| {
                let checkpoint_sequence_number = data.checkpoint_summary.sequence_number;
                data.output_objects()
                    .iter()
                    .filter_map(|o| {
                        if let sui_types::object::Data::Package(p) = &o.data {
                            let indexed_pkg = IndexedPackage {
                                package_id: o.id(),
                                move_package: p.clone(),
                                checkpoint_sequence_number,
                            };
                            Some((indexed_pkg, (**o).clone()))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

pub fn get_deleted_objects(effects: &TransactionEffects) -> Vec<ObjectRef> {
    let deleted = effects.deleted().into_iter();
    let wrapped = effects.wrapped().into_iter();
    let unwrapped_then_deleted = effects.unwrapped_then_deleted().into_iter();
    deleted
        .chain(wrapped)
        .chain(unwrapped_then_deleted)
        .collect::<Vec<_>>()
}

pub fn get_latest_objects(
    objects: Vec<&Object>,
) -> (
    HashMap<ObjectID, Object>,
    HashSet<(ObjectID, SequenceNumber)>,
) {
    let mut latest_objects = HashMap::new();
    let mut discarded_versions = HashSet::new();
    for object in objects {
        match latest_objects.entry(object.id()) {
            Entry::Vacant(e) => {
                e.insert(object.clone());
            }
            Entry::Occupied(mut e) => {
                if object.version() > e.get().version() {
                    discarded_versions.insert((e.get().id(), e.get().version()));
                    e.insert(object.clone());
                }
            }
        }
    }
    (latest_objects, discarded_versions)
}

fn try_create_dynamic_field_info(o: &Object) -> IndexerResult<Option<DynamicFieldInfoShort>> {
    // Skip if not a move object
    let Some(move_object) = o.data.try_as_move().cloned() else {
        return Ok(None);
    };

    if !move_object.type_().is_dynamic_field() {
        return Ok(None);
    }

    let (type_, object_id) = match &move_object.type_().type_params()[0] {
        TypeTag::Struct(tag) if DynamicFieldInfo::is_dynamic_object_field_wrapper(tag) => {
            let contents = move_object.into_contents();
            (
                DynamicFieldType::DynamicObject,
                // HACK: we explore the fact that BCS:
                // * serializes struct fields in order
                // * keeps bytes as bytes
                // Hence, we know the last 32 bytes of
                // ```
                // Field<Name, 0x2::object::ID> {
                //     id: 0x2::object::UID,
                //     name: Name,
                //     value: 0x2::object::ID
                // }`
                // ```
                // is the object id
                ObjectID::new(contents[(contents.len() - 32)..].try_into().unwrap()),
            )
        }
        // Safe to assume dynamic field here since we checked above that `o` is a DF.
        _ => (DynamicFieldType::DynamicField, o.id()),
    };

    Ok(Some(DynamicFieldInfoShort { type_, object_id }))
}
