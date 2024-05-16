use once_cell::sync::Lazy;

macro_rules! env {
    ($($name:ident: $ty:ty;)*) => {
        $(
            pub static $name: Lazy<Option<$ty>> = Lazy::new(|| {
                std::env::var(stringify!($name))
                    .ok()
                    .and_then(|s| s.parse().ok())
            });
        )*
    };
}

//*************************************************************************************************#
// DB
//*************************************************************************************************#

env! {
    DB_POOL_SIZE: u32;
    DB_CONNECTION_TIMEOUT: u64;
    DB_STATEMENT_TIMEOUT: u64;
}

//*************************************************************************************************#
// Indexer
//*************************************************************************************************#

env! {
    DOWNLOAD_QUEUE_SIZE: usize;
    INGESTION_READER_TIMEOUT_SECS: u64;
}

//*************************************************************************************************#
// Runner
//*************************************************************************************************#

env! {
    CHECKPOINT_PROCESSING_BATCH_DATA_LIMIT: usize;
    CHECKPOINT_PROCESSING_BATCH_SIZE: usize;
}

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

env! {
    OBJECTS_SNAPSHOT_MIN_CHECKPOINT_LAG: usize;
    OBJECTS_SNAPSHOT_MAX_CHECKPOINT_LAG: usize;
}

//*************************************************************************************************#
// Checkpoint Handler
//*************************************************************************************************#

env! {
    CHECKPOINT_QUEUE_SIZE: usize;
    CHECKPOINT_COMMIT_BATCH_SIZE: usize;
}

//*************************************************************************************************#
// PG Indexer Store
//*************************************************************************************************#

env! {
    PG_COMMIT_PARALLEL_CHUNK_SIZE: usize;
    PG_COMMIT_OBJECTS_PARALLEL_CHUNK_SIZE: usize;
    EPOCHS_TO_KEEP: u64;
    SKIP_OBJECT_HISTORY: bool;
    SKIP_OBJECT_SNAPSHOT: bool;
}

//*************************************************************************************************#
// Fetcher
//*************************************************************************************************#

env! {
    CHECKPOINT_FETCH_INTERVAL_MS: u64;
}
