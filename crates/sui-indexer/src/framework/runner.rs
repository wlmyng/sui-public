// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use tokio_util::sync::CancellationToken;

use crate::metrics::IndexerMetrics;
use crate::CONFIG;

use super::fetcher::CheckpointDownloadData;
use super::interface::Handler;

pub async fn run<S>(
    stream: S,
    mut handlers: Vec<Box<dyn Handler>>,
    metrics: IndexerMetrics,
    cancel: CancellationToken,
) where
    S: futures::Stream<Item = CheckpointDownloadData> + std::marker::Unpin,
{
    use futures::StreamExt;
    let batch_size = CONFIG.runner.checkpoint_processing_batch_size();
    let data_limit = CONFIG.runner.checkpoint_processing_batch_data_limit();
    tracing::info!("Indexer runner is starting with {batch_size}");
    let mut chunks: futures::stream::ReadyChunks<S> = stream.ready_chunks(batch_size);
    while let Some(checkpoints) = tokio::select! {
       _ = cancel.cancelled() => {
        tracing::info!("Indexer runner is cancelled");
        return
       }
       chunk = chunks.next() => chunk,
    } {
        //TODO create tracing spans for processing
        let mut cp_batch = vec![];
        let mut cp_batch_total_size = 0;
        for checkpoint in checkpoints.iter() {
            cp_batch_total_size += checkpoint.size;
            cp_batch.push(checkpoint.data.clone());
            if cp_batch_total_size >= data_limit {
                futures::future::join_all(handlers.iter_mut().map(|handler| async {
                    handler.process_checkpoints(&cp_batch).await.unwrap()
                }))
                .await;

                metrics.indexing_batch_size.set(cp_batch_total_size as i64);
                cp_batch = vec![];
                cp_batch_total_size = 0;
            }
        }
        if !cp_batch.is_empty() {
            futures::future::join_all(
                handlers
                    .iter_mut()
                    .map(|handler| async { handler.process_checkpoints(&cp_batch).await.unwrap() }),
            )
            .await;
            metrics.indexing_batch_size.set(cp_batch_total_size as i64);
        }
    }
}
