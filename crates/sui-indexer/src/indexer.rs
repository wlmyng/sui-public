// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeSet;
use std::env;
use std::time::Duration;

use anyhow::Result;
use backoff::backoff::Backoff as _;
use prometheus::Registry;
use sui_types::full_checkpoint_content::CheckpointData;
use sui_types::messages_checkpoint::CheckpointSequenceNumber;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::info;

use mysten_metrics::spawn_monitored_task;
use sui_data_ingestion_core::{
    create_remote_store_client, DataIngestionMetrics, IndexerExecutor, ReaderOptions,
    ShimProgressStore, WorkerPool,
};

use crate::errors::IndexerError;
use crate::handlers::checkpoint_handler::{new_handlers, CheckpointHandler};
use crate::metrics::IndexerMetrics;
use crate::store::IndexerStore;
use crate::Config;

pub struct Indexer;

impl Indexer {
    pub async fn start_writer<S: IndexerStore + Sync + Send + Clone + 'static>(
        store: S,
        metrics: IndexerMetrics,
        config: &Config,
    ) -> Result<(), IndexerError> {
        Indexer::start_writer_with_config::<S>(store, metrics, CancellationToken::new(), config)
            .await
    }

    pub async fn start_writer_with_config<S: IndexerStore + Sync + Send + Clone + 'static>(
        store: S,
        metrics: IndexerMetrics,
        cancel: CancellationToken,
        config: &Config,
    ) -> Result<(), IndexerError> {
        info!(
            "Sui Indexer Writer (version {:?}) started...",
            env!("CARGO_PKG_VERSION")
        );

        // Start from the latest checkpoint number between the config and the last checkpoint
        // persisted in the Db.
        let watermark = std::cmp::max(
            config
                .data_ingestion
                .starting_checkpoint
                .unwrap_or_default(),
            store
                .get_latest_checkpoint_sequence_number()
                .await
                .expect("Failed to get latest tx checkpoint sequence number from DB")
                .map(|seq| seq + 1)
                .unwrap_or_default(),
        );

        let cancel_clone = cancel.clone();
        let (exit_sender, exit_receiver) = oneshot::channel();
        // Spawn a task that links the cancellation token to the exit sender
        spawn_monitored_task!(async move {
            cancel_clone.cancelled().await;
            let _ = exit_sender.send(());
        });

        let mut executor = IndexerExecutor::new(
            ShimProgressStore(watermark),
            1,
            DataIngestionMetrics::new(&Registry::new()),
        );
        let worker = new_handlers::<S>(store, metrics, watermark, cancel.clone(), config).await?;
        let worker_pool = WorkerPool::new(
            worker,
            "workflow".to_string(),
            config.data_ingestion.batch_size,
        );
        let extra_reader_options = ReaderOptions {
            batch_size: config.data_ingestion.batch_size,
            timeout_secs: config.data_ingestion.timeout_secs,
            data_limit: config.data_ingestion.data_limit,
            ..Default::default()
        };
        executor.register(worker_pool).await?;
        executor
            .run(
                config
                    .data_ingestion
                    .path
                    .clone()
                    .unwrap_or(tempfile::tempdir().unwrap().into_path()),
                config.data_ingestion.remote_store_url.clone(),
                vec![],
                extra_reader_options,
                exit_receiver,
            )
            .await?;
        Ok(())
    }

    pub async fn index_checkpoints<S: IndexerStore + Sync + Send + Clone + 'static>(
        sequence_numbers: Vec<u64>,
        store: S,
        metrics: IndexerMetrics,
        config: &Config,
    ) -> Result<(), IndexerError> {
        let object_store = create_remote_store_client(
            config
                .data_ingestion
                .remote_store_url
                .clone()
                .expect("remote-store-url not set"),
            vec![],
            config.data_ingestion.timeout_secs,
        )
        .expect("failed to create remote store client");

        let deduped_and_ordered = BTreeSet::from_iter(sequence_numbers);
        for sequence_number in deduped_and_ordered {
            let (checkpoint, _size) =
                remote_fetch_checkpoint(&object_store, sequence_number).await?;
            let data = CheckpointHandler::data_to_commit(checkpoint, store.clone(), &metrics, None)
                .await?;
            let epoch = data.epoch.clone();
            crate::handlers::committer::commit_checkpoints(&store, vec![data], epoch, &metrics)
                .await;
        }
        Ok(())
    }
}

pub(crate) async fn remote_fetch_checkpoint(
    store: &impl object_store::ObjectStore,
    checkpoint_number: CheckpointSequenceNumber,
) -> Result<(CheckpointData, usize)> {
    let mut backoff = backoff::ExponentialBackoff::default();
    backoff.max_elapsed_time = Some(Duration::from_secs(60));
    backoff.initial_interval = Duration::from_millis(100);
    backoff.current_interval = backoff.initial_interval;
    backoff.multiplier = 1.0;
    loop {
        match remote_fetch_checkpoint_internal(store, checkpoint_number).await {
            Ok(data) => return Ok(data),
            Err(err) => match backoff.next_backoff() {
                Some(duration) => {
                    if !err.to_string().contains("404") {
                        tracing::debug!(
                            "remote reader retry in {} ms. Error is {:?}",
                            duration.as_millis(),
                            err
                        );
                    }
                    tokio::time::sleep(duration).await
                }
                None => return Err(err),
            },
        }
    }
}

pub(crate) async fn remote_fetch_checkpoint_internal(
    store: &impl object_store::ObjectStore,
    checkpoint_number: CheckpointSequenceNumber,
) -> Result<(CheckpointData, usize)> {
    let path = object_store::path::Path::from(format!("{}.chk", checkpoint_number));
    let response = store.get(&path).await?;
    let bytes = response.bytes().await?;
    Ok((
        sui_storage::blob::Blob::from_bytes::<CheckpointData>(&bytes)?,
        bytes.len(),
    ))
}
