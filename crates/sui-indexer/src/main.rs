// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use tracing::info;

use sui_indexer::errors::IndexerError;
use sui_indexer::metrics::start_prometheus_server;
use sui_indexer::CONFIG;

#[tokio::main]
async fn main() -> Result<(), IndexerError> {
    // NOTE: this is to print out tracing like info, warn & error.
    let _guard = telemetry_subscribers::TelemetryConfig::new()
        .with_env()
        .init();

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
