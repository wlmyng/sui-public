// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::env;

use anyhow::Result;
use diesel::r2d2::R2D2Connection;
use prometheus::Registry;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::info;

use mysten_metrics::spawn_monitored_task;
use sui_data_ingestion_core::{
    DataIngestionMetrics, IndexerExecutor, ReaderOptions, ShimProgressStore, WorkerPool,
};

use crate::build_json_rpc_server;
use crate::errors::IndexerError;
use crate::handlers::checkpoint_handler::new_handlers;
use crate::handlers::objects_snapshot_processor::{ObjectsSnapshotProcessor, SnapshotLagConfig};
use crate::indexer_reader::IndexerReader;
use crate::metrics::IndexerMetrics;
use crate::store::IndexerStore;
use crate::{IndexerConfig, CONFIG};

pub struct Indexer;

impl Indexer {
    pub async fn start_writer<
        S: IndexerStore + Sync + Send + Clone + 'static,
        T: R2D2Connection + 'static,
    >(
        config: &IndexerConfig,
        store: S,
        metrics: IndexerMetrics,
    ) -> Result<(), IndexerError> {
        let snapshot_config = SnapshotLagConfig::default();
        Indexer::start_writer_with_config::<S, T>(
            config,
            store,
            metrics,
            snapshot_config,
            CancellationToken::new(),
        )
        .await
    }

    pub async fn start_writer_with_config<
        S: IndexerStore + Sync + Send + Clone + 'static,
        T: R2D2Connection + 'static,
    >(
        config: &IndexerConfig,
        store: S,
        metrics: IndexerMetrics,
        snapshot_config: SnapshotLagConfig,
        cancel: CancellationToken,
    ) -> Result<(), IndexerError> {
        info!(
            "Sui Indexer Writer (version {:?}) started...",
            env!("CARGO_PKG_VERSION")
        );

        let watermark = store
            .get_latest_checkpoint_sequence_number()
            .await
            .expect("Failed to get latest tx checkpoint sequence number from DB")
            .map(|seq| seq + 1)
            .unwrap_or_default();

        let download_queue_size = CONFIG.indexer.download_queue_size();
        let ingestion_reader_timeout_secs = CONFIG.indexer.ingestion_reader_timeout_secs();
        let data_limit = CONFIG.runner.checkpoint_processing_batch_data_limit();

        let rest_client = sui_rest_api::Client::new(format!("{}/rest", config.rpc_client_url));

        let objects_snapshot_processor = ObjectsSnapshotProcessor::new_with_config(
            rest_client.clone(),
            store.clone(),
            metrics.clone(),
            snapshot_config,
            cancel.clone(),
        );
        spawn_monitored_task!(objects_snapshot_processor.start());

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
        let worker =
            new_handlers::<S, T>(store, rest_client, metrics, watermark, cancel.clone()).await?;
        let worker_pool = WorkerPool::new(worker, "workflow".to_string(), download_queue_size);
        let extra_reader_options = ReaderOptions {
            batch_size: download_queue_size,
            timeout_secs: ingestion_reader_timeout_secs,
            data_limit,
            ..Default::default()
        };
        executor.register(worker_pool).await?;
        executor
            .run(
                config
                    .data_ingestion_path
                    .clone()
                    .unwrap_or(tempfile::tempdir().unwrap().into_path()),
                config.remote_store_url.clone(),
                vec![],
                extra_reader_options,
                exit_receiver,
            )
            .await?;
        Ok(())
    }

    pub async fn start_reader<T: R2D2Connection + 'static>(
        config: &IndexerConfig,
        registry: &Registry,
        db_url: String,
    ) -> Result<(), IndexerError> {
        info!(
            "Sui Indexer Reader (version {:?}) started...",
            env!("CARGO_PKG_VERSION")
        );
        let indexer_reader = IndexerReader::<T>::new(db_url)?;
        let handle = build_json_rpc_server(registry, indexer_reader, config, None)
            .await
            .expect("Json rpc server should not run into errors upon start.");
        tokio::spawn(async move { handle.stopped().await })
            .await
            .expect("Rpc server task failed");

        Ok(())
    }
}
