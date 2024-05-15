use once_cell::sync::Lazy;

//*************************************************************************************************#
// DB
//*************************************************************************************************#

pub static DB_POOL_SIZE: Lazy<u32> = Lazy::new(|| {
    std::env::var("DB_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100)
});
pub static DB_CONNECTION_TIMEOUT: Lazy<u64> = Lazy::new(|| {
    std::env::var("DB_CONNECTION_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600)
});
pub static DB_STATEMENT_TIMEOUT: Lazy<u64> = Lazy::new(|| {
    std::env::var("DB_STATEMENT_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600)
});

//*************************************************************************************************#
// Indexer
//*************************************************************************************************#

pub static DOWNLOAD_QUEUE_SIZE: Lazy<Option<usize>> = Lazy::new(|| {
    std::env::var("DOWNLOAD_QUEUE_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
});

pub static INGESTION_READER_TIMEOUT_SECS: Lazy<Option<u64>> = Lazy::new(|| {
    std::env::var("INGESTION_READER_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
});

/// Limit indexing parallelism on big checkpoints to avoid OOM,
/// by limiting the total size of batch checkpoints to ~20MB.
/// On testnet, most checkpoints are < 200KB, some can go up to 50MB.
pub static CHECKPOINT_PROCESSING_BATCH_DATA_LIMIT: Lazy<Option<usize>> = Lazy::new(|| {
    std::env::var("CHECKPOINT_PROCESSING_BATCH_DATA_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
});

pub static CHECKPOINT_PROCESSING_BATCH_SIZE: Lazy<Option<usize>> = Lazy::new(|| {
    std::env::var("CHECKPOINT_PROCESSING_BATCH_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
});

//*************************************************************************************************#
// Objects Snapshot Processor
//*************************************************************************************************#

// The objects_snapshot table maintains a delayed snapshot of the objects table,
// controlled by object_snapshot_max_checkpoint_lag (max lag) and
// object_snapshot_min_checkpoint_lag (min lag). For instance, with a max lag of 900
// and a min lag of 300 checkpoints, the objects_snapshot table will lag behind the
// objects table by 300 to 900 checkpoints. The snapshot is updated when the lag
// exceeds the max lag threshold, and updates continue until the lag is reduced to
// the min lag threshold. Then, we have a consistent read range between
// latest_snapshot_cp and latest_cp based on objects_snapshot and objects_history,
// where the size of this range varies between the min and max lag values.

pub static OBJECTS_SNAPSHOT_MIN_CHECKPOINT_LAG: Lazy<Option<usize>> = Lazy::new(|| {
    std::env::var("OBJECTS_SNAPSHOT_MIN_CHECKPOINT_LAG")
        .ok()
        .and_then(|s| s.parse().ok())
});

pub static OBJECTS_SNAPSHOT_MAX_CHECKPOINT_LAG: Lazy<Option<usize>> = Lazy::new(|| {
    std::env::var("OBJECTS_SNAPSHOT_MAX_CHECKPOINT_LAG")
        .ok()
        .and_then(|s| s.parse().ok())
});

//*************************************************************************************************#
// Checkpoint Handler
//*************************************************************************************************#

pub static CHECKPOINT_QUEUE_SIZE: Lazy<Option<usize>> = Lazy::new(|| {
    std::env::var("CHECKPOINT_QUEUE_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
});

//*************************************************************************************************#
// PG Indexer Store
//*************************************************************************************************#

pub static PG_COMMIT_PARALLEL_CHUNK_SIZE: Lazy<Option<usize>> = Lazy::new(|| {
    std::env::var("PG_COMMIT_PARALLEL_CHUNK_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
});
pub static PG_COMMIT_OBJECTS_PARALLEL_CHUNK_SIZE: Lazy<Option<usize>> = Lazy::new(|| {
    std::env::var("PG_COMMIT_OBJECTS_PARALLEL_CHUNK_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
});
pub static EPOCHS_TO_KEEP: Lazy<Option<u64>> = Lazy::new(|| {
    std::env::var("EPOCHS_TO_KEEP")
        .ok()
        .and_then(|s| s.parse().ok())
});
pub static SKIP_OBJECT_HISTORY: Lazy<Option<bool>> = Lazy::new(|| {
    std::env::var("SKIP_OBJECT_HISTORY")
        .ok()
        .and_then(|s| s.parse().ok())
});
pub static SKIP_OBJECT_SNAPSHOT: Lazy<Option<bool>> = Lazy::new(|| {
    std::env::var("SKIP_OBJECT_SNAPSHOT")
        .ok()
        .and_then(|s| s.parse().ok())
});

//*************************************************************************************************#
// Checkpoint Handler
//*************************************************************************************************#

pub static CHECKPOINT_COMMIT_BATCH_SIZE: Lazy<Option<usize>> = Lazy::new(|| {
    std::env::var("CHECKPOINT_COMMIT_BATCH_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
});

//*************************************************************************************************#
// Fetcher
//*************************************************************************************************#

pub static CHECKPOINT_FETCH_INTERVAL_MS: Lazy<u64> = Lazy::new(|| {
    std::env::var("CHECKPOINT_FETCH_INTERVAL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500)
});
