//! meshd: finds nearby Meshtastic radios for the web client.

mod ble;
mod http;
mod mdns;
mod parse;
mod registry;

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::registry::Registry;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Address for the HTTP API.
    #[arg(long, env = "MESHD_LISTEN", default_value = "127.0.0.1:4405")]
    listen: SocketAddr,

    /// Web origins allowed to call the API (comma-separated).
    #[arg(
        long = "allow-origin",
        env = "MESHD_ALLOW_ORIGINS",
        value_delimiter = ',',
        default_values_t = [
            "http://localhost:3000".to_owned(),
            "https://localhost:3000".to_owned(),
            "https://client.meshtastic.org".to_owned(),
        ]
    )]
    allow_origins: Vec<String>,

    /// Forget BLE radios not heard from for this many seconds.
    #[arg(long, env = "MESHD_BLE_TTL_SECS", default_value_t = 30)]
    ble_ttl_secs: u64,

    /// Disable BLE scanning.
    #[arg(long)]
    no_ble: bool,

    /// Disable mDNS browsing.
    #[arg(long)]
    no_mdns: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        // No colour codes when piped (docker logs, journald).
        .with_ansi(std::io::stdout().is_terminal())
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("meshd=info")),
        )
        .init();

    let args = Args::parse();
    let registry = Arc::new(Registry::new());

    if !args.no_mdns {
        let registry = registry.clone();
        tokio::spawn(async move {
            if let Err(err) = mdns::run(registry).await {
                error!("mDNS discovery stopped: {err:#}");
            }
        });
    }

    if !args.no_ble {
        let registry_for_scan = registry.clone();
        tokio::spawn(async move {
            if let Err(err) = ble::run(registry_for_scan).await {
                error!("BLE discovery stopped: {err:#}");
            }
        });

        let registry = registry.clone();
        let ttl = Duration::from_secs(args.ble_ttl_secs);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                registry.prune_ble(ttl);
            }
        });
    }

    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    info!(listen = %args.listen, origins = ?args.allow_origins, "meshd serving");
    axum::serve(listener, http::router(registry, &args.allow_origins))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
