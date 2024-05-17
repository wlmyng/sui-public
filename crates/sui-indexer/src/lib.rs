// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0
#![recursion_limit = "256"]

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{anyhow, Result};
use clap::Parser;
use diesel::r2d2::R2D2Connection;
use jsonrpsee::http_client::{HeaderMap, HeaderValue, HttpClient, HttpClientBuilder};
use metrics::IndexerMetrics;
use mysten_metrics::spawn_monitored_task;
use once_cell::sync::Lazy;
use prometheus::Registry;
use secrecy::{ExposeSecret, Secret};
use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr};
use std::path::PathBuf;
use sui_types::base_types::{ObjectID, SuiAddress};
use system_package_task::SystemPackageTask;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use url::Url;

use sui_json_rpc::ServerType;
use sui_json_rpc::{JsonRpcServerBuilder, ServerHandle};
use sui_json_rpc_api::CLIENT_SDK_TYPE_HEADER;

use crate::apis::{
    CoinReadApi, ExtendedApi, GovernanceReadApi, IndexerApi, MoveUtilsApi, ReadApi,
    TransactionBuilderApi, WriteApi,
};
use crate::indexer_reader::IndexerReader;
use errors::IndexerError;

pub mod apis;
pub mod db;
pub mod environment;
pub mod errors;
pub mod framework;
pub mod handlers;
pub mod indexer;
pub mod indexer_reader;
pub mod metrics;
pub mod models;
pub mod schema;
pub mod store;
pub mod system_package_task;
pub mod test_utils;
pub mod types;

pub static CONFIG: Lazy<Config> = Lazy::new(|| {
    let CliArgs {
        mut config,
        db_url,
        db_user_name,
        db_password,
        db_host,
        db_port,
        db_name,
    } = CliArgs::parse();

    config.indexer.db_url = db_url;
    config.indexer.db_user_name = db_user_name;
    config.indexer.db_password = db_password;
    config.indexer.db_host = db_host;
    config.indexer.db_port = db_port;
    config.indexer.db_name = db_name;

    config
});

#[derive(Parser, Clone, Debug)]
#[clap(
    name = "Sui indexer",
    about = "An off-fullnode service serving data from Sui protocol",
    rename_all = "kebab-case"
)]
pub struct CliArgs {
    /// Path to the TOML configuration file.
    #[arg(long)]
    pub config: Config,

    #[clap(long)]
    pub db_url: Option<Secret<String>>,
    #[clap(long)]
    pub db_user_name: Option<String>,
    #[clap(long)]
    pub db_password: Option<Secret<String>>,
    #[clap(long)]
    pub db_host: Option<String>,
    #[clap(long)]
    pub db_port: Option<u16>,
    #[clap(long)]
    pub db_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    #[serde(default)]
    pub indexer: IndexerConfig,
    #[serde(default)]
    pub database: DbConfig,
    #[serde(default)]
    pub runner: RunnerConfig,
    #[serde(default)]
    pub object_snapshot: ObjectSnapshotConfig,
    #[serde(default)]
    pub checkpoint_handler: CheckpointHandlerConfig,
    #[serde(default)]
    pub postgres_store: PgIndexerStoreConfig,
    #[serde(default)]
    pub fetcher: FetcherConfig,
}

impl std::str::FromStr for Config {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let toml_str = std::fs::read_to_string(s)?;
        Ok(toml::from_str(&toml_str)?)
    }
}

macro_rules! get_with_env_override {
    ($($attr:ident = $ENV:ident -> $ty:ty;)*) => ($(
        pub fn $attr(&self) -> $ty {
            environment::$ENV.unwrap_or(self.$attr)
        }
    )*)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct DbConfig {
    db_pool_size: u32,
    db_connection_timeout: u64,
    db_statement_timeout: u64,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            db_pool_size: 100,
            db_connection_timeout: 3600,
            db_statement_timeout: 3600,
        }
    }
}

impl DbConfig {
    get_with_env_override! {
        db_pool_size = DB_POOL_SIZE -> u32;
        db_connection_timeout = DB_CONNECTION_TIMEOUT -> u64;
        db_statement_timeout = DB_STATEMENT_TIMEOUT -> u64;
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RunnerConfig {
    /// Limit indexing parallelism on big checkpoints to avoid OOM,
    /// by limiting the total size of batch checkpoints to ~20MB.
    /// On testnet, most checkpoints are < 200KB, some can go up to 50MB.
    checkpoint_processing_batch_data_limit: usize,
    checkpoint_processing_batch_size: usize,
}

impl RunnerConfig {
    get_with_env_override! {
        checkpoint_processing_batch_data_limit = CHECKPOINT_PROCESSING_BATCH_DATA_LIMIT -> usize;
        checkpoint_processing_batch_size = CHECKPOINT_PROCESSING_BATCH_SIZE -> usize;
    }
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            checkpoint_processing_batch_data_limit: 20000000,
            checkpoint_processing_batch_size: 100,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ObjectSnapshotConfig {
    objects_snapshot_min_checkpoint_lag: usize,
    objects_snapshot_max_checkpoint_lag: usize,
}

impl ObjectSnapshotConfig {
    get_with_env_override! {
        objects_snapshot_min_checkpoint_lag = OBJECTS_SNAPSHOT_MIN_CHECKPOINT_LAG -> usize;
        objects_snapshot_max_checkpoint_lag = OBJECTS_SNAPSHOT_MAX_CHECKPOINT_LAG -> usize;
    }
}

impl Default for ObjectSnapshotConfig {
    fn default() -> Self {
        Self {
            objects_snapshot_min_checkpoint_lag: 300,
            objects_snapshot_max_checkpoint_lag: 900,
        }
    }
}

#[serde_as]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct CheckpointHandlerConfig {
    checkpoint_queue_size: usize,
    checkpoint_commit_batch_size: usize,
    /// If ommitted ([`None`]), disables filtering. If `Some(vec![])`, commit only non-PTB
    /// transactions.
    #[serde_as(as = "Option<Vec<DisplayFromStr>>")]
    pub filter_packages: Option<Vec<ObjectID>>,
}

impl CheckpointHandlerConfig {
    get_with_env_override! {
        checkpoint_queue_size = CHECKPOINT_QUEUE_SIZE -> usize;
        checkpoint_commit_batch_size = CHECKPOINT_COMMIT_BATCH_SIZE -> usize;
    }
}

impl Default for CheckpointHandlerConfig {
    fn default() -> Self {
        Self {
            checkpoint_queue_size: 100,
            checkpoint_commit_batch_size: 100,
            filter_packages: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PgIndexerStoreConfig {
    /// The amount of rows to update in one DB transcation
    pg_commit_parallel_chunk_size: usize,
    /// The amount of rows to update in one DB transcation, for objects particularly
    /// Having this number too high may cause many db deadlocks because of
    /// optimistic locking.
    pg_commit_objects_parallel_chunk_size: usize,
    epochs_to_keep: Option<u64>,
    skip_object_history: bool,
    skip_object_snapshot: bool,
}

impl PgIndexerStoreConfig {
    get_with_env_override! {
        pg_commit_parallel_chunk_size = PG_COMMIT_PARALLEL_CHUNK_SIZE -> usize;
        pg_commit_objects_parallel_chunk_size = PG_COMMIT_OBJECTS_PARALLEL_CHUNK_SIZE -> usize;
        skip_object_history = SKIP_OBJECT_HISTORY -> bool;
        skip_object_snapshot = SKIP_OBJECT_SNAPSHOT -> bool;
    }

    pub fn epochs_to_keep(&self) -> Option<u64> {
        environment::EPOCHS_TO_KEEP.or(self.epochs_to_keep)
    }
}

impl Default for PgIndexerStoreConfig {
    fn default() -> Self {
        Self {
            pg_commit_parallel_chunk_size: 100,
            pg_commit_objects_parallel_chunk_size: 500,
            epochs_to_keep: None,
            skip_object_history: false,
            skip_object_snapshot: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct FetcherConfig {
    checkpoint_fetch_interval_ms: u64,
}

impl FetcherConfig {
    get_with_env_override! {
        checkpoint_fetch_interval_ms = CHECKPOINT_FETCH_INTERVAL_MS -> u64;
    }
}

impl Default for FetcherConfig {
    fn default() -> Self {
        Self {
            checkpoint_fetch_interval_ms: 500,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct IndexerConfig {
    // The database fields are not deserialized by Serde and must be instead initialized by other
    // means.
    #[serde(default = "default_db_url", skip)]
    pub db_url: Option<Secret<String>>,
    #[serde(default, skip)]
    pub db_user_name: Option<String>,
    #[serde(default, skip)]
    pub db_password: Option<Secret<String>>,
    #[serde(default, skip)]
    pub db_host: Option<String>,
    #[serde(default, skip)]
    pub db_port: Option<u16>,
    #[serde(default, skip)]
    pub db_name: Option<String>,

    #[serde(default = "default_rpc_client_url")]
    pub rpc_client_url: String,
    pub remote_store_url: Option<String>,
    #[serde(default = "default_client_metric_host")]
    pub client_metric_host: String,
    #[serde(default = "default_client_metric_port")]
    pub client_metric_port: u16,
    #[serde(default = "default_rpc_server_url")]
    pub rpc_server_url: String,
    #[serde(default = "default_rpc_server_port")]
    pub rpc_server_port: u16,
    #[serde(default)]
    pub reset_db: bool,
    #[serde(default)]
    pub fullnode_sync_worker: bool,
    #[serde(default)]
    pub rpc_server_worker: bool,
    pub data_ingestion_path: Option<PathBuf>,
    pub name_service_package_address: Option<SuiAddress>,
    pub name_service_registry_id: Option<ObjectID>,
    pub name_service_reverse_registry_id: Option<ObjectID>,

    // Added to the original
    #[serde(default = "default_download_queue_size")]
    download_queue_size: usize,
    #[serde(default = "default_ingestion_reader_timeout_secs")]
    ingestion_reader_timeout_secs: u64,
    #[serde(default)]
    pub starting_checkpoint_number: Option<u64>,
}

impl IndexerConfig {
    /// returns connection url without the db name
    pub fn base_connection_url(&self) -> Result<String, anyhow::Error> {
        let url_secret = self.get_db_url()?;
        let url_str = url_secret.expose_secret();
        let url = Url::parse(url_str).expect("Failed to parse URL");
        Ok(format!(
            "{}://{}:{}@{}:{}/",
            url.scheme(),
            url.username(),
            url.password().unwrap_or_default(),
            url.host_str().unwrap_or_default(),
            url.port().unwrap_or_default()
        ))
    }

    pub fn get_db_url(&self) -> Result<Secret<String>, anyhow::Error> {
        match (&self.db_url, &self.db_user_name, &self.db_password, &self.db_host, &self.db_port, &self.db_name) {
            (Some(db_url), _, _, _, _, _) => Ok(db_url.clone()),
            (None, Some(db_user_name), Some(db_password), Some(db_host), Some(db_port), Some(db_name)) => {
                Ok(secrecy::Secret::new(format!(
                    "postgres://{}:{}@{}:{}/{}",
                    db_user_name, db_password.expose_secret(), db_host, db_port, db_name
                )))
            }
            _ => Err(anyhow!("Invalid db connection config, either db_url or (db_user_name, db_password, db_host, db_port, db_name) must be provided")),
        }
    }

    get_with_env_override! {
        download_queue_size = DOWNLOAD_QUEUE_SIZE -> usize;
        ingestion_reader_timeout_secs = INGESTION_READER_TIMEOUT_SECS -> u64;
    }
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            db_url: default_db_url(),
            db_user_name: None,
            db_password: None,
            db_host: None,
            db_port: None,
            db_name: None,
            rpc_client_url: default_rpc_client_url(),
            remote_store_url: None,
            client_metric_host: default_client_metric_host(),
            client_metric_port: default_client_metric_port(),
            rpc_server_url: default_rpc_server_url(),
            rpc_server_port: default_rpc_server_port(),
            reset_db: false,
            fullnode_sync_worker: false,
            rpc_server_worker: false,
            data_ingestion_path: None,
            name_service_package_address: None,
            name_service_registry_id: None,
            name_service_reverse_registry_id: None,
            download_queue_size: default_download_queue_size(),
            ingestion_reader_timeout_secs: default_ingestion_reader_timeout_secs(),
            starting_checkpoint_number: None,
        }
    }
}

fn default_db_url() -> Option<Secret<String>> {
    Some(secrecy::Secret::new(
        "postgres://postgres:postgres@localhost:5432/sui_indexer".to_string(),
    ))
}

pub fn default_rpc_client_url() -> String {
    "http://0.0.0.0:9000".to_owned()
}

pub fn default_client_metric_host() -> String {
    "0.0.0.0".to_owned()
}

pub fn default_client_metric_port() -> u16 {
    9184
}

pub fn default_rpc_server_url() -> String {
    "0.0.0.0".to_owned()
}

pub fn default_rpc_server_port() -> u16 {
    9000
}

pub fn default_download_queue_size() -> usize {
    200
}

pub fn default_ingestion_reader_timeout_secs() -> u64 {
    20
}

pub async fn build_json_rpc_server<T: R2D2Connection>(
    prometheus_registry: &Registry,
    reader: IndexerReader<T>,
    config: &IndexerConfig,
    custom_runtime: Option<Handle>,
) -> Result<ServerHandle, IndexerError> {
    let mut builder =
        JsonRpcServerBuilder::new(env!("CARGO_PKG_VERSION"), prometheus_registry, None, None);
    let http_client = crate::get_http_client(config.rpc_client_url.as_str())?;

    let name_service_config =
        if let (Some(package_address), Some(registry_id), Some(reverse_registry_id)) = (
            config.name_service_package_address,
            config.name_service_registry_id,
            config.name_service_reverse_registry_id,
        ) {
            sui_json_rpc::name_service::NameServiceConfig::new(
                package_address,
                registry_id,
                reverse_registry_id,
            )
        } else {
            sui_json_rpc::name_service::NameServiceConfig::default()
        };

    builder.register_module(WriteApi::new(http_client.clone()))?;
    builder.register_module(IndexerApi::new(reader.clone(), name_service_config))?;
    builder.register_module(TransactionBuilderApi::new(reader.clone()))?;
    builder.register_module(MoveUtilsApi::new(reader.clone()))?;
    builder.register_module(GovernanceReadApi::new(reader.clone()))?;
    builder.register_module(ReadApi::new(reader.clone()))?;
    builder.register_module(CoinReadApi::new(reader.clone()))?;
    builder.register_module(ExtendedApi::new(reader.clone()))?;

    let default_socket_addr: SocketAddr = SocketAddr::new(
        // unwrap() here is safe b/c the address is a static config.
        config.rpc_server_url.as_str().parse().unwrap(),
        config.rpc_server_port,
    );

    let cancel = CancellationToken::new();
    let system_package_task =
        SystemPackageTask::new(reader.clone(), cancel.clone(), Duration::from_secs(10));

    tracing::info!("Starting system package task");
    spawn_monitored_task!(async move { system_package_task.run().await });

    Ok(builder
        .start(
            default_socket_addr,
            custom_runtime,
            Some(ServerType::Http),
            Some(cancel),
        )
        .await?)
}

fn get_http_client(rpc_client_url: &str) -> Result<HttpClient, IndexerError> {
    let mut headers = HeaderMap::new();
    headers.insert(CLIENT_SDK_TYPE_HEADER, HeaderValue::from_static("indexer"));

    HttpClientBuilder::default()
        .max_request_body_size(2 << 30)
        .max_concurrent_requests(usize::MAX)
        .set_headers(headers.clone())
        .build(rpc_client_url)
        .map_err(|e| {
            warn!("Failed to get new Http client with error: {:?}", e);
            IndexerError::HttpClientInitError(format!(
                "Failed to initialize fullnode RPC client with error: {:?}",
                e
            ))
        })
}
