use clap::Parser;
use secrecy::Secret;
use sui_indexer::db::setup_postgres::index_checkpoints;
use sui_indexer::errors::IndexerError;
use sui_indexer::metrics::start_prometheus_server;
use sui_indexer::{Config, CONFIG};

#[derive(Parser, Clone, Debug)]
#[clap(
    name = "Sui index",
    about = "An utility for indexing specific checkpoints",
    rename_all = "kebab-case"
)]
pub struct CliArgs {
    /// Checkpoint sequence numbers to index.
    pub checkpoints: Vec<u64>,

    /// Path to the TOML configuration file used by sui-indexer.
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

#[tokio::main]
async fn main() -> Result<(), IndexerError> {
    let _guard = telemetry_subscribers::TelemetryConfig::new()
        .with_env()
        .init();
    let sequence_numbers = parse_args();
    tracing::info!("Using sui-indexer config: {:#?}", *CONFIG);

    let (_registry_service, registry) = start_prometheus_server(
        // NOTE: this parses the input host addr and port number for socket addr,
        // so unwrap() is safe here.
        format!(
            "{}:{}",
            CONFIG.indexer.client_metric_host, CONFIG.indexer.client_metric_port
        )
        .parse()
        .unwrap(),
        CONFIG.indexer.rpc_client_url.as_str(),
    )?;

    #[cfg(feature = "postgres-feature")]
    index_checkpoints(registry, sequence_numbers).await?;

    Ok(())
}

fn parse_args() -> Vec<u64> {
    let CliArgs {
        checkpoints,
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
    CONFIG.set(config);
    checkpoints
}
