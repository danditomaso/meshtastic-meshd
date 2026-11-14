//! Browses mDNS for radios on WiFi or Ethernet.

use std::net::IpAddr;
use std::sync::Arc;

use anyhow::Context;
use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent};
use tracing::{debug, info, warn};

use crate::parse;
use crate::registry::{MdnsObservation, Registry, now_ms};

/// Service type Meshtastic firmware advertises.
pub const SERVICE_TYPE: &str = "_meshtastic._tcp.local.";

pub async fn run(registry: Arc<Registry>) -> anyhow::Result<()> {
    let daemon = ServiceDaemon::new().context("starting mDNS daemon")?;
    let events = daemon.browse(SERVICE_TYPE).context("browsing mDNS")?;
    info!(service = SERVICE_TYPE, "mDNS browse started");

    while let Ok(event) = events.recv_async().await {
        match event {
            ServiceEvent::ServiceResolved(service) => {
                let obs = observation(&service);
                info!(
                    service = %obs.fullname,
                    node_id = ?obs.node_id,
                    addresses = ?obs.addresses,
                    "mDNS resolved"
                );
                registry.upsert_mdns(obs);
            }
            ServiceEvent::ServiceRemoved(_, fullname) => {
                info!(service = %fullname, "mDNS removed");
                registry.remove_mdns(&fullname);
            }
            other => debug!(?other, "mDNS event"),
        }
    }

    warn!("mDNS browse channel closed");
    Ok(())
}

fn observation(service: &ResolvedService) -> MdnsObservation {
    // Skip loopback, and prefer IPv4: browsers can't use link-local IPv6.
    let all: Vec<IpAddr> = service
        .get_addresses()
        .iter()
        .map(|ip| ip.to_ip_addr())
        .filter(|ip| !ip.is_loopback())
        .collect();
    let mut addresses: Vec<IpAddr> = all.iter().copied().filter(IpAddr::is_ipv4).collect();
    if addresses.is_empty() {
        addresses = all;
    }
    addresses.sort();

    MdnsObservation {
        fullname: service.get_fullname().to_owned(),
        hostname: service.get_hostname().to_owned(),
        addresses,
        port: service.get_port(),
        short_name: service
            .get_property_val_str("shortname")
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        node_id: service
            .get_property_val_str("id")
            .and_then(parse::normalize_node_id),
        last_seen_ms: now_ms(),
    }
}
