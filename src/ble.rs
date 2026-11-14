//! Scans for radios advertising the Meshtastic BLE service.

use std::sync::Arc;

use anyhow::Context;
use btleplug::api::{Central, CentralEvent, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, PeripheralId};
use futures::StreamExt;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::registry::{BleObservation, Registry, now_ms};

/// Meshtastic BLE service UUID.
pub const SERVICE_UUID: Uuid = Uuid::from_u128(0x6ba1b218_15a8_461f_9fa8_5dcae273eafd);

pub async fn run(registry: Arc<Registry>) -> anyhow::Result<()> {
    // macOS blocks here until Bluetooth permission is granted.
    info!("opening BLE adapter; macOS may prompt for Bluetooth permission");
    let manager = Manager::new().await.context("opening BLE manager")?;
    let Some(adapter) = manager.adapters().await?.into_iter().next() else {
        warn!("no BLE adapter found; BLE discovery disabled");
        return Ok(());
    };

    let info = adapter.adapter_info().await.unwrap_or_default();
    let state = adapter.adapter_state().await.ok();
    info!(adapter = %info, ?state, "BLE adapter");

    let mut events = adapter
        .events()
        .await
        .context("subscribing to BLE events")?;
    adapter
        .start_scan(ScanFilter {
            services: vec![SERVICE_UUID],
        })
        .await
        .context("starting BLE scan")?;
    info!(service = %SERVICE_UUID, "BLE scan started");

    while let Some(event) = events.next().await {
        match event {
            CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) => {
                if let Some(obs) = observation(&adapter, &id).await {
                    debug!(id = %obs.id, name = ?obs.name, rssi = ?obs.rssi, "BLE seen");
                    registry.upsert_ble(obs);
                }
            }
            CentralEvent::StateUpdate(state) => info!(?state, "BLE adapter state"),
            _ => {}
        }
    }

    warn!("BLE event stream ended");
    Ok(())
}

async fn observation(adapter: &Adapter, id: &PeripheralId) -> Option<BleObservation> {
    let peripheral = adapter.peripheral(id).await.ok()?;
    let props = peripheral.properties().await.ok()??;
    // Some backends ignore the scan filter.
    if !props.services.contains(&SERVICE_UUID) {
        return None;
    }
    Some(BleObservation {
        id: id.to_string(),
        name: props.local_name.or(props.advertisement_name),
        rssi: props.rssi,
        last_seen_ms: now_ms(),
    })
}
