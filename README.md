# meshd

A small local helper that lets the [Meshtastic web client](https://github.com/meshtastic/web)
find nearby radios automatically.

**meshd only does discovery.** It never connects to a radio, sends packets, or changes
settings. It tells the web client which radios are nearby and how to reach them; the web client
then connects to the radio itself, exactly as if you had typed its address or picked it in the
browser's Bluetooth chooser.

## Why it exists

The Android app finds radios on its own: it scans Bluetooth for the Meshtastic service and
browses the local network for `_meshtastic._tcp` (mDNS). Browsers can do neither:

- **mDNS:** no browser API can browse for network services.
- **Bluetooth:** Web Bluetooth only shows a chooser after a click. It can't scan in the
  background, and it only works in Chromium-based browsers.

meshd runs next to the browser, does both kinds of discovery, and serves the result over a
small HTTP API that the web client reads.

## How it works

```
  Bluetooth radios ── BLE scan (btleplug) ──┐
                                            ├─▶ merge ─▶ HTTP API (127.0.0.1:4405) ─▶ web client
  WiFi/Ethernet radios ── mDNS browse ──────┘                                          "Discovered nearby"
```

1. **Bluetooth:** scans for radios advertising the Meshtastic service UUID
   `6ba1b218-15a8-461f-9fa8-5dcae273eafd`, the same filter the Android app uses.
2. **mDNS:** browses `_meshtastic._tcp` and reads the `shortname` and `id` TXT records that
   firmware announces on WiFi/Ethernet.
3. **Merge:** network entries with the same node id become one device. A Bluetooth radio joins
   a network device when its name suffix (`Meshtastic_1a2b`) matches exactly one node id ending
   in `1a2b`. That match is only 4 hex digits, so it's flagged as a guess (`mergedOn: "suffix"`).
4. **Serve:** `GET /api/devices` returns the list; `GET /api/devices/stream` pushes it as
   Server-Sent Events whenever a radio appears, disappears or is renamed. `GET /healthz`
   identifies the server as meshd so the web client never trusts some other server at that
   address.

In the web client, open **Add connection**. If meshd is reachable, a **Discovered nearby**
section lists the radios:

- **HTTP** fills in the radio's address and tests it.
- **Bluetooth** opens the browser's chooser narrowed to that one radio. The browser still makes
  the connection, so Web Bluetooth's rules still apply.

If meshd isn't running, the section doesn't appear and the dialog works as before. The address
the web client uses is under **Add connection → Advanced settings → Discovery service URL**
(default `http://127.0.0.1:4405`).

The API, merge rules and platform notes are in [docs/SPEC.md](docs/SPEC.md).

## Run it

### Natively (recommended on macOS and Windows)

Requires Rust 1.100 (pinned in `rust-toolchain.toml`).

```sh
cargo run --release
```

On macOS the first run asks for Bluetooth permission for your terminal app. Until you answer,
Bluetooth discovery waits silently.

If `cargo --version` isn't 1.100, you're probably running Homebrew's Rust, which ignores
`rust-toolchain.toml`. Put rustup's toolchain first (fish):

```fish
fish_add_path --prepend (dirname (rustup which --toolchain stable cargo))
```

### Docker (Linux hosts)

```sh
docker build -t meshd .
docker run --rm --net=host \
  -v /run/dbus/system_bus_socket:/run/dbus/system_bus_socket \
  meshd
```

- `--net=host` is required: mDNS multicast doesn't cross Docker's bridge network.
- The D-Bus socket mount gives meshd the host's BlueZ (`bluetoothd` must be running). Without
  it, meshd logs that Bluetooth discovery stopped and keeps serving mDNS results.
- meshd still listens on `127.0.0.1:4405`, which with host networking is the host's localhost.

**Docker Desktop on macOS or Windows can't discover anything.** Containers run inside a VM that
has no access to the host's Bluetooth and doesn't receive LAN multicast. Run meshd natively
there instead.

### Options

| Flag | Env | Default | |
|---|---|---|---|
| `--listen` | `MESHD_LISTEN` | `127.0.0.1:4405` | Address for the HTTP API |
| `--allow-origin` | `MESHD_ALLOW_ORIGINS` | `http://localhost:3000`, `https://localhost:3000`, `https://client.meshtastic.org` | Web origins allowed to call the API (comma-separated) |
| `--ble-ttl-secs` | `MESHD_BLE_TTL_SECS` | `30` | Forget Bluetooth radios not heard for this long |
| `--no-ble` | | | Disable Bluetooth scanning |
| `--no-mdns` | | | Disable mDNS browsing |

Logs: `RUST_LOG=meshd=debug`.

```sh
curl localhost:4405/healthz
curl localhost:4405/api/devices
curl -N localhost:4405/api/devices/stream
```

## Security

- meshd listens on **localhost only** by default, so only browsers on the same machine can use
  it.
- It answers browser requests only from the origins in `--allow-origin`.
- To serve other machines (for example meshd on a Raspberry Pi), set
  `--listen 0.0.0.0:4405` and add the web client's origin. Anyone on that network can then see
  your radios' names, node ids and addresses. There's no authentication yet, so only do this on
  a network you trust.

## Status

Early and experimental (0.1.0).

| | macOS | Linux | Windows |
|---|---|---|---|
| mDNS | ✓ tested | untested | untested |
| Bluetooth | ✓ tested | untested (BlueZ via D-Bus) | untested |
| Docker | API only | untested on real hardware | API only |

Planned: an optional Avahi backend on Linux, a bearer token for non-localhost use, and
announcing Bluetooth-only radios over mDNS. See [docs/SPEC.md](docs/SPEC.md#future-work).

## Develop

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## License

GPL-3.0-only, matching the Meshtastic projects.
