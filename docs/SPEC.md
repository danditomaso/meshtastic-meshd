# meshd spec

Status: draft, v0 (discovery only).

## Goal

Give the Meshtastic web client the automatic discovery the Android app has. Browsers can't
browse mDNS or passively scan BLE, so a small local service does it and serves the result over
HTTP.

## Discovery sources

| Source | What it finds | Implementation | Matches Android |
|---|---|---|---|
| mDNS | `_meshtastic._tcp` services (radios on WiFi/Ethernet) | `mdns-sd` (all platforms) | `NetworkConstants.SERVICE_TYPE`, `JvmServiceDiscovery` |
| BLE | Peripherals advertising service `6ba1b218-15a8-461f-9fa8-5dcae273eafd` | `btleplug` (CoreBluetooth / BlueZ / WinRT) | `MeshtasticBleConstants.SERVICE_UUID`, `KableBleScanner` |

TXT records read: `shortname`, `id` (`!a1b2c3d4`). Display name follows Android's
`processTcpServices`: `<shortname>_<id>` unless the short name already contains the id. IPv4
addresses are preferred; IPv6 is used only when no IPv4 address resolves.

BLE peripherals not seen for `--ble-ttl-secs` (default 30) are dropped. mDNS services leave on
removal events.

## Merging

| Case | Result | `mergedOn` |
|---|---|---|
| Several mDNS services with the same `id` | One device, all TCP endpoints | `nodeId` |
| BLE name ends `_xxxx` and exactly one device's node id ends `xxxx` | BLE joins that device | `suffix` |
| BLE suffix matches zero or several devices | BLE stays its own device (`ble:<id>`) | `null` |

`suffix` merges match 16 bits of the node id and can be wrong; clients should present them as
"probably the same radio".

## HTTP API

Default bind `127.0.0.1:4405`.

### `GET /api/devices`

```json
[
  {
    "key": "!a1b2c3d4",
    "name": "ABCD_a1b2c3d4",
    "nodeId": "!a1b2c3d4",
    "transports": [
      { "type": "tcp", "host": "192.168.1.5", "port": 4403, "hostname": "abcd.local.", "service": "Meshtastic_c3d4._meshtastic._tcp.local." },
      { "type": "ble", "id": "6F1C…", "name": "Meshtastic_c3d4", "rssi": -61 }
    ],
    "mergedOn": "suffix",
    "lastSeenMs": 1794675000000
  }
]
```

- `key` is stable for the life of the device: node id when known, else `mdns:<service>` or
  `ble:<peripheral id>`.
- BLE `id` is platform-specific: a MAC address on Linux/Windows, a per-host UUID on macOS.
- `port` is the advertised TCP API port (4403). The web client's HTTP transport talks to the
  firmware's HTTP server on `host` (port 80/443), not this port.

### `GET /api/devices/stream`

Server-Sent Events. Sends `event: devices` with the full list (same shape as above) on connect
and after every change: a device appearing, disappearing, or being renamed. RSSI changes alone
don't emit (a radio's RSSI swung -55 to -84 dBm several times a second in testing); the latest
RSSI rides along in the next snapshot or `GET /api/devices`. Keep-alive comment every 15s. Full snapshots are fine for the expected
list size (tens of devices); deltas can come later without breaking clients that only read
`devices` events.

### `GET /healthz`

```json
{ "service": "meshd", "api": 1, "version": "0.1.0" }
```

Identifies the server. The web client streams devices only when `service` is `"meshd"` and
`api` matches the version it understands; any other answer (e.g. a dev server returning HTML
with 200) is reported as "not a meshd service" and no devices are shown. `api` is bumped on
breaking changes to routes or the device JSON. The client also validates each device record
and drops malformed ones.

## Browser access

- CORS: only origins in `--allow-origin` / `MESHD_ALLOW_ORIGINS` (defaults:
  `http://localhost:3000`, `https://localhost:3000`, `https://client.meshtastic.org`).
- Private Network Access: preflights carrying `Access-Control-Request-Private-Network` get
  `Access-Control-Allow-Private-Network: true`, so an HTTPS public origin can call loopback.
  Chrome's Local Network Access permission prompt may still appear.
- Mixed content: `http://127.0.0.1` / `http://localhost` count as secure contexts in Chrome and
  Firefox. Binding a LAN address for other machines needs TLS (future).

## Platforms

| Platform | mDNS | BLE | Notes |
|---|---|---|---|
| macOS | ✓ | ✓ | First BLE use triggers the Bluetooth permission prompt for the launching app (terminal). Until it's answered, adapter setup blocks silently (observed ~80s with no log). |
| Linux | ✓ | ✓ via BlueZ D-Bus | Needs `bluetoothd`. `mdns-sd` shares port 5353 with `avahi-daemon`. |
| Linux container | ✓ with `--net=host` | ✓ with host D-Bus socket mounted | Multicast does not cross the Docker bridge. Image: `Dockerfile` (Debian trixie, needs `libdbus-1-3`). Untested against real BlueZ yet. |
| Docker Desktop (macOS/Windows) | ✗ | ✗ | The VM has no BlueZ and never sees LAN multicast. The API runs; discovery doesn't. |
| Windows | ✓ | ✓ | Untested. |

## Alternatives considered

**meshtastic-sdk (Kotlin/JVM) in a container**, spiked 2026-11-14 with the SDK's versions
(Kotlin 2.4.10, Kable 0.45.0, JmDNS 3.6.4). Kept Rust for now.

- Kable's JVM backend is btleplug via `kable-btleplug-ffi`, the same BLE engine meshd uses. It
  ships natives for linux x86-64/aarch64, macOS and Windows x64, needing glibc ≥ 2.38 (Ubuntu
  24.04 works; Debian 12, Ubuntu 22.04 and Alpine don't).
- Kable on JVM scans unfiltered and filters in Kotlin. A missing BlueZ socket surfaces as a Rust
  `unwrap()` panic wrapped in `InternalException`; the process survives.
- Docker Desktop on macOS can do neither discovery source: its Linux VM has no BlueZ, and with
  `--network host` JmDNS binds to the VM network (192.168.65.x) and never sees LAN multicast.
  Containers only work for discovery on a Linux host (`--net=host`, host D-Bus socket mounted).
- Discovery lives in the meshtastic-android app, not the SDK, so this path first means moving it
  into an SDK module. Revisit if the SDK grows a host/RPC server that this could live in.

## Future work

1. **Avahi backend** (Linux, optional cargo feature): browse via `org.freedesktop.Avahi` over
   D-Bus (`zbus`) instead of opening our own mDNS socket, for hosts where avahi-daemon is the
   system resolver.
2. **Re-announce BLE radios** as `_meshtastic._tcp` so Android and the CLI see them too.
3. **Bridging**: `/d/{key}/api/v1/{toradio,fromradio}` mirroring the firmware HTTP API, and a
   TCP 4403 listener per BLE radio. meshd holds the single BLE connection and fans out.
4. **Auth**: bearer token required when bound to a non-loopback address.
5. **Web client**: "Discovered" section in `AddConnectionDialog` driven by
   `/api/devices/stream`, hidden when `/healthz` fails.
6. Packaging: static binary + systemd unit; Docker image for Linux hosts.
