//! Stores what each discovery source has seen and merges it into one device list.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::watch;

use crate::parse;

/// A resolved `_meshtastic._tcp` service.
#[derive(Clone, Debug)]
pub struct MdnsObservation {
    pub fullname: String,
    pub hostname: String,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    pub short_name: Option<String>,
    pub node_id: Option<String>,
    pub last_seen_ms: u64,
}

/// A BLE peripheral advertising the Meshtastic service.
#[derive(Clone, Debug)]
pub struct BleObservation {
    pub id: String,
    pub name: Option<String>,
    pub rssi: Option<i16>,
    pub last_seen_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Transport {
    #[serde(rename_all = "camelCase")]
    Tcp {
        host: String,
        port: u16,
        hostname: String,
        service: String,
    },
    #[serde(rename_all = "camelCase")]
    Ble {
        id: String,
        name: Option<String>,
        rssi: Option<i16>,
    },
}

/// How a device's entries were joined. `Suffix` matches only 4 hex digits, so it's a guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MergedOn {
    NodeId,
    Suffix,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// Node id when known, else `mdns:<service>` or `ble:<id>`.
    pub key: String,
    pub name: String,
    pub node_id: Option<String>,
    pub transports: Vec<Transport>,
    pub merged_on: Option<MergedOn>,
    pub last_seen_ms: u64,
}

#[derive(Default)]
struct State {
    mdns: HashMap<String, MdnsObservation>,
    ble: HashMap<String, BleObservation>,
}

pub struct Registry {
    state: RwLock<State>,
    changed: watch::Sender<u64>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            state: RwLock::default(),
            changed: watch::Sender::new(0),
        }
    }

    /// Ticks whenever the device list may have changed.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub fn devices(&self) -> Vec<Device> {
        let state = self.state.read().expect("registry lock poisoned");
        let mdns: Vec<_> = state.mdns.values().cloned().collect();
        let ble: Vec<_> = state.ble.values().cloned().collect();
        merge(&mdns, &ble)
    }

    pub fn upsert_mdns(&self, obs: MdnsObservation) {
        self.update(|s| {
            s.mdns.insert(obs.fullname.clone(), obs);
            true
        });
    }

    pub fn remove_mdns(&self, fullname: &str) {
        self.update(|s| s.mdns.remove(fullname).is_some());
    }

    /// RSSI jitters constantly, so only new radios and renames notify clients.
    pub fn upsert_ble(&self, obs: BleObservation) {
        self.update(|s| match s.ble.get_mut(&obs.id) {
            Some(old) if old.name == obs.name => {
                old.rssi = obs.rssi;
                old.last_seen_ms = obs.last_seen_ms;
                false
            }
            _ => {
                s.ble.insert(obs.id.clone(), obs);
                true
            }
        });
    }

    /// Drops BLE radios not seen within `ttl`.
    pub fn prune_ble(&self, ttl: Duration) {
        let cutoff = now_ms().saturating_sub(ttl.as_millis() as u64);
        self.update(|s| {
            let before = s.ble.len();
            s.ble.retain(|_, obs| obs.last_seen_ms >= cutoff);
            s.ble.len() != before
        });
    }

    fn update(&self, f: impl FnOnce(&mut State) -> bool) {
        let changed = f(&mut self.state.write().expect("registry lock poisoned"));
        if changed {
            self.changed.send_modify(|version| *version += 1);
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Joins mDNS entries by node id, then attaches each BLE radio to the one device matching its name suffix.
pub fn merge(mdns: &[MdnsObservation], ble: &[BleObservation]) -> Vec<Device> {
    let mut devices: Vec<Device> = Vec::new();
    let mut by_node_id: HashMap<String, usize> = HashMap::new();

    let mut mdns: Vec<_> = mdns.iter().collect();
    mdns.sort_by(|a, b| a.fullname.cmp(&b.fullname));
    for obs in mdns {
        let transports = obs.addresses.iter().map(|addr| Transport::Tcp {
            host: addr.to_string(),
            port: obs.port,
            hostname: obs.hostname.clone(),
            service: obs.fullname.clone(),
        });

        if let Some(index) = obs.node_id.as_ref().and_then(|id| by_node_id.get(id)) {
            let device = &mut devices[*index];
            device.transports.extend(transports);
            device.merged_on = Some(MergedOn::NodeId);
            device.last_seen_ms = device.last_seen_ms.max(obs.last_seen_ms);
            continue;
        }

        if let Some(id) = &obs.node_id {
            by_node_id.insert(id.clone(), devices.len());
        }
        devices.push(Device {
            key: obs
                .node_id
                .clone()
                .unwrap_or_else(|| format!("mdns:{}", obs.fullname)),
            name: parse::tcp_display_name(obs.short_name.as_deref(), obs.node_id.as_deref()),
            node_id: obs.node_id.clone(),
            transports: transports.collect(),
            merged_on: None,
            last_seen_ms: obs.last_seen_ms,
        });
    }

    let mut ble: Vec<_> = ble.iter().collect();
    ble.sort_by(|a, b| a.id.cmp(&b.id));
    for obs in ble {
        let transport = Transport::Ble {
            id: obs.id.clone(),
            name: obs.name.clone(),
            rssi: obs.rssi,
        };

        let suffix = obs.name.as_deref().and_then(parse::ble_name_suffix);
        let candidates: Vec<usize> = suffix
            .as_deref()
            .map(|suffix| {
                devices
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| {
                        d.node_id.as_deref().and_then(parse::node_id_suffix) == Some(suffix)
                            && !d
                                .transports
                                .iter()
                                .any(|t| matches!(t, Transport::Ble { .. }))
                    })
                    .map(|(i, _)| i)
                    .collect()
            })
            .unwrap_or_default();

        if let [index] = candidates[..] {
            let device = &mut devices[index];
            device.transports.push(transport);
            device.merged_on = Some(MergedOn::Suffix);
            device.last_seen_ms = device.last_seen_ms.max(obs.last_seen_ms);
            continue;
        }

        devices.push(Device {
            key: format!("ble:{}", obs.id),
            name: obs
                .name
                .clone()
                .unwrap_or_else(|| parse::DEFAULT_SHORT_NAME.to_owned()),
            node_id: None,
            transports: vec![transport],
            merged_on: None,
            last_seen_ms: obs.last_seen_ms,
        });
    }

    devices.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.key.cmp(&b.key)));
    devices
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mdns(fullname: &str, node_id: Option<&str>, addr: &str) -> MdnsObservation {
        MdnsObservation {
            fullname: fullname.into(),
            hostname: "radio.local.".into(),
            addresses: vec![addr.parse().unwrap()],
            port: 4403,
            short_name: Some("ABCD".into()),
            node_id: node_id.map(Into::into),
            last_seen_ms: 1,
        }
    }

    fn ble(id: &str, name: &str) -> BleObservation {
        BleObservation {
            id: id.into(),
            name: Some(name.into()),
            rssi: Some(-60),
            last_seen_ms: 2,
        }
    }

    #[test]
    fn mdns_services_with_same_node_id_merge() {
        let devices = merge(
            &[
                mdns(
                    "a._meshtastic._tcp.local.",
                    Some("!a1b2c3d4"),
                    "192.168.1.5",
                ),
                mdns("b._meshtastic._tcp.local.", Some("!a1b2c3d4"), "10.0.0.5"),
            ],
            &[],
        );
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].key, "!a1b2c3d4");
        assert_eq!(devices[0].transports.len(), 2);
        assert_eq!(devices[0].merged_on, Some(MergedOn::NodeId));
    }

    #[test]
    fn ble_joins_mdns_device_on_unique_suffix() {
        let devices = merge(
            &[mdns(
                "a._meshtastic._tcp.local.",
                Some("!a1b2c3d4"),
                "192.168.1.5",
            )],
            &[ble("AA:BB", "Meshtastic_C3D4")],
        );
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].merged_on, Some(MergedOn::Suffix));
        assert_eq!(devices[0].last_seen_ms, 2);
    }

    #[test]
    fn ble_stays_separate_when_suffix_is_ambiguous() {
        let devices = merge(
            &[
                mdns(
                    "a._meshtastic._tcp.local.",
                    Some("!1111c3d4"),
                    "192.168.1.5",
                ),
                mdns(
                    "b._meshtastic._tcp.local.",
                    Some("!2222c3d4"),
                    "192.168.1.6",
                ),
            ],
            &[ble("AA:BB", "Meshtastic_c3d4")],
        );
        assert_eq!(devices.len(), 3);
        assert!(devices.iter().any(|d| d.key == "ble:AA:BB"));
        assert!(devices.iter().all(|d| d.merged_on.is_none()));
    }

    #[test]
    fn ble_without_suffix_is_its_own_device() {
        let devices = merge(&[], &[ble("AA:BB", "Meshtastic")]);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].key, "ble:AA:BB");
        assert_eq!(devices[0].name, "Meshtastic");
    }

    #[test]
    fn mdns_without_node_id_keys_on_service_name() {
        let devices = merge(
            &[mdns("x._meshtastic._tcp.local.", None, "192.168.1.5")],
            &[],
        );
        assert_eq!(devices[0].key, "mdns:x._meshtastic._tcp.local.");
        assert_eq!(devices[0].name, "ABCD");
    }

    #[test]
    fn device_serialises_to_spec_shape() {
        let devices = merge(
            &[mdns(
                "a._meshtastic._tcp.local.",
                Some("!a1b2c3d4"),
                "192.168.1.5",
            )],
            &[],
        );
        let json = serde_json::to_value(&devices[0]).unwrap();
        assert_eq!(json["nodeId"], "!a1b2c3d4");
        assert_eq!(json["transports"][0]["type"], "tcp");
        assert_eq!(json["transports"][0]["host"], "192.168.1.5");
        assert_eq!(json["lastSeenMs"], 1);
    }

    #[test]
    fn rssi_jitter_refreshes_without_notifying() {
        let registry = Registry::new();
        let rx = registry.subscribe();
        registry.upsert_ble(ble("AA:BB", "Meshtastic_c3d4"));
        registry.upsert_ble(BleObservation {
            rssi: Some(-65),
            last_seen_ms: 9,
            ..ble("AA:BB", "Meshtastic_c3d4")
        });
        assert_eq!(*rx.borrow(), 1);
        assert_eq!(registry.devices()[0].last_seen_ms, 9);

        registry.upsert_ble(BleObservation {
            name: Some("Renamed_c3d4".into()),
            ..ble("AA:BB", "Meshtastic_c3d4")
        });
        assert_eq!(*rx.borrow(), 2);
    }

    #[test]
    fn prune_drops_stale_ble_and_notifies() {
        let registry = Registry::new();
        let rx = registry.subscribe();
        registry.upsert_ble(BleObservation {
            last_seen_ms: 0,
            ..ble("AA:BB", "Meshtastic_c3d4")
        });
        registry.prune_ble(Duration::from_secs(30));
        assert!(registry.devices().is_empty());
        assert_eq!(*rx.borrow(), 2);
    }
}
