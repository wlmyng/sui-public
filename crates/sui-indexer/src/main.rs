// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use tracing::info;

use sui_indexer::errors::IndexerError;
use sui_indexer::metrics::start_prometheus_server;
use sui_indexer::Config;

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
    /// Run indexer as a writer, which pulls data from fullnode and writes data to DB.
    Sync,
    /// run indexer as a reader, which is a JSON RPC server.
    ///
    /// Serves the interface: https://docs.sui.io/sui-api-ref#suix_getallbalances
    Serve,
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
        &config.rpc_client_url.as_str(),
    )?;

    match command {
        Cmd::Sync => {
            #[cfg(feature = "postgres-feature")]
            sui_indexer::db::setup_postgres::fullnode_sync_worker(registry.clone(), &config)
                .await?;

            #[cfg(not(feature = "postgres-feature"))]
            anyhow::bail!("sync only implemented for postgres-feature");
        }
        Cmd::Serve => {
            #[cfg(feature = "postgres-feature")]
            sui_indexer::db::setup_postgres::rpc_server_worker(registry.clone(), &config).await?;
            #[cfg(not(feature = "postgres-feature"))]
            anyhow::bail!("serve only implemented for postgres-feature");
        }
        Cmd::AddCheckpoints { checkpoints } => {
            #[cfg(feature = "postgres-feature")]
            sui_indexer::db::setup_postgres::index_checkpoints(registry, checkpoints, &config)
                .await?;
            #[cfg(not(feature = "postgres-feature"))]
            anyhow::bail!("add-checkpoints only implemented for postgres-feature");
        }
        Cmd::ResetDb => {
            #[cfg(feature = "postgres-feature")]
            sui_indexer::db::setup_postgres::reset_db(&config.database).await?;
            #[cfg(not(feature = "postgres-feature"))]
            anyhow::bail!("reset-db only implemented for postgres-feature");
        }
    }

    Ok(())
}
