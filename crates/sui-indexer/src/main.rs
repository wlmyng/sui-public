// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0
#![recursion_limit = "256"]

use clap::Parser;
use errors::IndexerError;
use tracing::info;

#[derive(Parser)]
#[clap(
    name = "Sui indexer",
    about = "An off-fullnode service serving data from Sui protocol",
    rename_all = "kebab-case"
)]
pub struct CliArgs {
    #[clap(subcommand)]
    command: Cmd,
    /// Path to the TOML configuration file.
    #[arg(long)]
    config: Config,

    #[arg(long, global = true)]
    db_url: Option<secrecy::Secret<String>>,
    #[arg(long, global = true)]
    db_user_name: Option<String>,
    #[arg(long, global = true)]
    db_password: Option<secrecy::Secret<String>>,
    #[arg(long, global = true)]
    db_host: Option<String>,
    #[arg(long, global = true)]
    db_port: Option<u16>,
    #[arg(long, global = true)]
    db_name: Option<String>,
}

impl CliArgs {
    fn init() -> (Cmd, Config) {
        let Self {
            command,
            mut config,
            db_url,
            db_user_name,
            db_password,
            db_host,
            db_port,
            db_name,
        } = Self::parse();

        config.database.db_url = db_url;
        config.database.db_user_name = db_user_name;
        config.database.db_password = db_password;
        config.database.db_host = db_host;
        config.database.db_port = db_port;
        config.database.db_name = db_name;
        (command, config)
    }
}

#[derive(clap::Subcommand)]
enum Cmd {
    /// Pulls data from fullnode and writes to the database.
    Sync,
    /// An utility for indexing specific checkpoints.
    AddCheckpoints {
        /// Checkpoint sequence numbers to index.
        checkpoints: Vec<u64>,
    },
    /// Caution: resets (clears) the database.
    ResetDb,
}

#[tokio::main]
async fn main() -> Result<(), IndexerError> {
    // NOTE: this is to print out tracing like info, warn & error.
    let _guard = telemetry_subscribers::TelemetryConfig::new()
        .with_env()
        .init();
    let (command, config) = CliArgs::init();

    info!("Parsed config: {:#?}", config);
    let (_registry_service, registry) = start_prometheus_server(
        // NOTE: this parses the input host addr and port number for socket addr,
        // so unwrap() is safe here.
        format!(
            "{}:{}",
            &config.client_metric_host, &config.client_metric_port
        )
        .parse()
        .unwrap(),
        config.rpc_client_url.as_str(),
    )?;

    match command {
        Cmd::Sync => {
            #[cfg(feature = "postgres-feature")]
            crate::db::setup_postgres::fullnode_sync_worker(registry.clone(), &config).await?;

            #[cfg(not(feature = "postgres-feature"))]
            anyhow::bail!("sync only implemented for postgres-feature");
        }
        Cmd::AddCheckpoints { checkpoints } => {
            #[cfg(feature = "postgres-feature")]
            crate::db::setup_postgres::index_checkpoints(registry, checkpoints, &config).await?;
            #[cfg(not(feature = "postgres-feature"))]
            anyhow::bail!("add-checkpoints only implemented for postgres-feature");
        }
        Cmd::ResetDb => {
            #[cfg(feature = "postgres-feature")]
            crate::db::setup_postgres::reset_db(&config.database).await?;
            #[cfg(not(feature = "postgres-feature"))]
            anyhow::bail!("reset-db only implemented for postgres-feature");
        }
    }

    Ok(())
}

// =============================================================================
//  Configuration
// =============================================================================

use anyhow::anyhow;
use secrecy::{ExposeSecret, Secret};
use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr};
use std::path::PathBuf;
use sui_types::base_types::ObjectID;
use url::Url;

use crate::metrics::start_prometheus_server;

mod db;
mod errors;
mod handlers;
mod indexer;
mod metrics;
mod models;
mod schema;
mod store;
mod types;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    #[serde(default = "default_rpc_client_url")]
    pub rpc_client_url: String,
    #[serde(default = "default_client_metric_host")]
    pub client_metric_host: String,
    #[serde(default = "default_client_metric_port")]
    pub client_metric_port: u16,

    #[serde(default)]
    pub database: DbConfig,

    pub data_ingestion: DataIngestionConfig,

    #[serde(default)]
    pub checkpoint_handler: CheckpointHandlerConfig,

    #[serde(default)]
    pub postgres_store: PgIndexerStoreConfig,
}

impl std::str::FromStr for Config {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let toml_str = std::fs::read_to_string(s)?;
        Ok(toml::from_str(&toml_str)?)
    }
}

/// Settings for the [`sui_data_ingestion_core`] framework.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct DataIngestionConfig {
    pub(crate) path: Option<PathBuf>,

    pub(crate) remote_store_url: Option<String>,

    #[serde(default = "default_download_queue_size")]
    pub(crate) batch_size: usize,

    /// Limit indexing parallelism on big checkpoints to avoid OOM,
    /// by limiting the total size of batch checkpoints to ~20MB.
    /// On testnet, most checkpoints are < 200KB, some can go up to 50MB.
    #[serde(default = "default_ingestion_data_limit")]
    pub(crate) data_limit: usize,

    #[serde(default = "default_ingestion_reader_timeout_secs")]
    pub(crate) timeout_secs: u64,

    #[serde(default)]
    pub(crate) starting_checkpoint: Option<u64>,
}

pub(crate) fn default_ingestion_data_limit() -> usize {
    20000000
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct DbConfig {
    // The database password is not deserialized by Serde and must be instead initialized by other
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

    pub(crate) db_pool_size: u32,
    pub(crate) db_connection_timeout: u64,
    pub(crate) db_statement_timeout: u64,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            db_url: default_db_url(),
            db_user_name: None,
            db_password: None,
            db_host: None,
            db_port: None,
            db_name: None,
            db_pool_size: 100,
            db_connection_timeout: 3600,
            db_statement_timeout: 3600,
        }
    }
}

impl DbConfig {
    /// returns connection url without the db name
    pub fn base_connection_url(&self) -> anyhow::Result<String> {
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

    pub fn get_db_url(&self) -> anyhow::Result<Secret<String>> {
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
}

#[serde_as]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct CheckpointHandlerConfig {
    pub(crate) queue_size: usize,
    pub(crate) commit_batch_size: usize,
    /// If ommitted ([`None`]), disables filtering. If `Some(vec![])`, commit only non-PTB
    /// transactions.
    #[serde_as(as = "Option<Vec<DisplayFromStr>>")]
    pub filter_packages: Option<Vec<ObjectID>>,
}

impl Default for CheckpointHandlerConfig {
    fn default() -> Self {
        Self {
            queue_size: 100,
            commit_batch_size: 100,
            filter_packages: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PgIndexerStoreConfig {
    /// The amount of rows to update in one DB transcation
    pub(crate) commit_parallel_chunk_size: usize,
    /// The amount of rows to update in one DB transcation, for objects particularly
    /// Having this number too high may cause many db deadlocks because of
    /// optimistic locking.
    pub(crate) commit_objects_parallel_chunk_size: usize,
    pub(crate) epochs_to_keep: Option<u64>,
    pub(crate) skip_object_history: bool,
}

impl Default for PgIndexerStoreConfig {
    fn default() -> Self {
        Self {
            commit_parallel_chunk_size: 100,
            commit_objects_parallel_chunk_size: 500,
            epochs_to_keep: None,
            skip_object_history: false,
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
