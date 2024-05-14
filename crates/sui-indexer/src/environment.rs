use once_cell::sync::Lazy;

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

pub static CHECKPOINT_FETCH_INTERVAL_MS: Lazy<u64> = Lazy::new(|| {
    std::env::var("CHECKPOINT_FETCH_INTERVAL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500)
});

pub static CHECKPOINT_PROCESSING_BATCH_SIZE: Lazy<Option<usize>> = Lazy::new(|| {
    std::env::var("CHECKPOINT_PROCESSING_BATCH_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
});

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

pub static CHECKPOINT_PROCESSING_BATCH_DATA_LIMIT: Lazy<Option<usize>> = Lazy::new(|| {
    std::env::var("CHECKPOINT_PROCESSING_BATCH_DATA_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
});

pub static CHECKPOINT_QUEUE_SIZE: Lazy<Option<usize>> = Lazy::new(|| {
    std::env::var("CHECKPOINT_QUEUE_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
});

pub static CHECKPOINT_COMMIT_BATCH_SIZE: Lazy<Option<usize>> = Lazy::new(|| {
    std::env::var("CHECKPOINT_COMMIT_BATCH_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
});

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
