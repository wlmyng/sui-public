// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use tracing::info;

use sui_indexer::errors::IndexerError;
use sui_indexer::metrics::start_prometheus_server;
use sui_indexer::{Config, CONFIG};

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
    pub db_url: Option<secrecy::Secret<String>>,
    #[clap(long)]
    pub db_user_name: Option<String>,
    #[clap(long)]
    pub db_password: Option<secrecy::Secret<String>>,
    #[clap(long)]
    pub db_host: Option<String>,
    #[clap(long)]
    pub db_port: Option<u16>,
    #[clap(long)]
    pub db_name: Option<String>,
}

impl CliArgs {
    fn init_global_config() {
        let Self {
            mut config,
            db_url,
            db_user_name,
            db_password,
            db_host,
            db_port,
            db_name,
        } = Self::parse();

        config.indexer.db_url = db_url;
        config.indexer.db_user_name = db_user_name;
        config.indexer.db_password = db_password;
        config.indexer.db_host = db_host;
        config.indexer.db_port = db_port;
        config.indexer.db_name = db_name;
        CONFIG.set(config)
    }
}

#[tokio::main]
async fn main() -> Result<(), IndexerError> {
    // NOTE: this is to print out tracing like info, warn & error.
    let _guard = telemetry_subscribers::TelemetryConfig::new()
        .with_env()
        .init();
    CliArgs::init_global_config();

    let indexer_config = &CONFIG.indexer;
    info!("Parsed indexer config: {:#?}", indexer_config);
    let (_registry_service, registry) = start_prometheus_server(
        // NOTE: this parses the input host addr and port number for socket addr,
        // so unwrap() is safe here.
        format!(
            "{}:{}",
            indexer_config.client_metric_host, indexer_config.client_metric_port
        )
        .parse()
        .unwrap(),
        indexer_config.rpc_client_url.as_str(),
    )?;
    #[cfg(feature = "postgres-feature")]
    sui_indexer::db::setup_postgres::setup(registry.clone()).await?;

    #[cfg(feature = "mysql-feature")]
    #[cfg(not(feature = "postgres-feature"))]
    sui_indexer::db::setup_mysql::setup(indexer_config, registry).await?;
    Ok(())
}
