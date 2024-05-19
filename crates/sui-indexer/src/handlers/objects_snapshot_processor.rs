// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use tokio_util::sync::CancellationToken;
use tracing::info;

use sui_rest_api::Client;

use crate::types::IndexerResult;
use crate::CONFIG;
use crate::{metrics::IndexerMetrics, store::IndexerStore};

#[derive(Clone, Debug)]
pub struct SnapshotLagConfig {
    pub snapshot_min_lag: usize,
    pub snapshot_max_lag: usize,
    pub sleep_duration: u64,
}

impl SnapshotLagConfig {
    pub fn new(
        min_lag: Option<usize>,
        max_lag: Option<usize>,
        sleep_duration: Option<u64>,
    ) -> Self {
        let default_config = Self::default();
        Self {
            snapshot_min_lag: min_lag.unwrap_or(default_config.snapshot_min_lag),
            snapshot_max_lag: max_lag.unwrap_or(default_config.snapshot_max_lag),
            sleep_duration: sleep_duration.unwrap_or(default_config.sleep_duration),
        }
    }
}

impl Default for SnapshotLagConfig {
    fn default() -> Self {
        let snapshot_min_lag = CONFIG.object_snapshot.objects_snapshot_min_checkpoint_lag();
        let snapshot_max_lag = CONFIG.object_snapshot.objects_snapshot_max_checkpoint_lag();

        SnapshotLagConfig {
            snapshot_min_lag,
            snapshot_max_lag,
            sleep_duration: 5,
        }
    }
}

pub struct ObjectsSnapshotProcessor<S> {
    pub client: Client,
    pub store: S,
    metrics: IndexerMetrics,
    pub config: SnapshotLagConfig,
    cancel: CancellationToken,
}

// NOTE: "handler"
impl<S> ObjectsSnapshotProcessor<S>
where
    S: IndexerStore + Clone + Sync + Send + 'static,
{
    #[tracing::instrument(skip_all)]
    pub fn new_with_config(
        client: Client,
        store: S,
        metrics: IndexerMetrics,
        config: SnapshotLagConfig,
        cancel: CancellationToken,
    ) -> ObjectsSnapshotProcessor<S> {
        tracing::info!("Using config {config:?}");
        Self {
            client,
            store,
            metrics,
            config,
            cancel,
        }
    }

    // The `objects_snapshot` table maintains a delayed snapshot of the `objects` table,
    // controlled by `object_snapshot_max_checkpoint_lag` (max lag) and
    // `object_snapshot_min_checkpoint_lag` (min lag). For instance, with a max lag of 900
    // and a min lag of 300 checkpoints, the `objects_snapshot` table will lag behind the
    // `objects` table by 300 to 900 checkpoints. The snapshot is updated when the lag
    // exceeds the max lag threshold, and updates continue until the lag is reduced to
    // the min lag threshold. Then, we have a consistent read range between
    // `latest_snapshot_cp` and `latest_cp` based on `objects_snapshot` and `objects_history`,
    // where the size of this range varies between the min and max lag values.
    pub async fn start(&self) -> IndexerResult<()> {
        info!("Starting object snapshot processor...");
        let latest_snapshot_cp = self
            .store
            .get_latest_object_snapshot_checkpoint_sequence_number()
            .await?
            .unwrap_or_default();
        // make sure cp 0 is handled
        let mut start_cp = if latest_snapshot_cp == 0 {
            0
        } else {
            latest_snapshot_cp + 1
        };
        // with MAX and MIN, the CSR range will vary from MIN cps to MAX cps
        let snapshot_window =
            self.config.snapshot_max_lag as u64 - self.config.snapshot_min_lag as u64;
        let mut latest_fn_cp = self.client.get_latest_checkpoint().await?.sequence_number;

        // While the below is true, we are in backfill mode, and so `ObjectsSnapshotProcessor` will
        // no-op. Once we exit the loop, this task will then be responsible for updating the
        // `objects_snapshot` table.
        while latest_fn_cp > start_cp + self.config.snapshot_max_lag as u64 {
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    info!("Shutdown signal received, terminating object snapshot processor");
                    return Ok(());
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(self.config.sleep_duration)) => {
                    latest_fn_cp = self.client.get_latest_checkpoint().await?.sequence_number;
                    start_cp = self
                        .store
                        .get_latest_object_snapshot_checkpoint_sequence_number()
                        .await?
                        .unwrap_or_default();
                }
            }
        }

        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    info!("Shutdown signal received, terminating object snapshot processor");
                    return Ok(());
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(self.config.sleep_duration)) => {
                    let latest_cp = self
                        .store
                        .get_latest_checkpoint_sequence_number()
                        .await?
                        .unwrap_or_default();

                    if latest_cp > start_cp + self.config.snapshot_max_lag as u64 {
                        self.store
                            .update_objects_snapshot(start_cp, start_cp + snapshot_window)
                            .await?;
                        start_cp += snapshot_window;
                        self.metrics
                            .latest_object_snapshot_sequence_number
                            .set(start_cp as i64);
                    }
                }
            }
        }
    }
}
