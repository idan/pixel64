# Wi-Fi onboarding (Improv over BLE)

First-run provisioning: the device gets onto the user's Wi-Fi using only a phone or laptop —
no app, no USB tool — via the open [Improv Wi-Fi](https://www.improv-wifi.com/ble/) standard
over Bluetooth LE, on the Pico 2 W's CYW43439 radio (`cyw43` + `trouble-host`).

## How to provision

1. Power on a device with no stored credentials → the panel shows **SETUP** and it advertises
   over BLE as **`pixel64`**.
2. In a supported browser, open an Improv web client → **Connect** → pick **`pixel64`** in the
   Bluetooth picker. (Use `web/improv-test/` or [improv-wifi.com](https://www.improv-wifi.com/ble/) —
   see the macOS note below.)
3. Enter your home SSID + password → **Submit**.
4. The panel shows **Connecting → ONLINE + IP**; the page shows success with the device URL
   (`http://<device-ip>`). Credentials are saved to flash, so subsequent boots connect
   automatically.

### Browser support

| Platform | Works? | Notes |
|----------|--------|-------|
| **Android** Chrome | ✅ | Verified end-to-end |
| **macOS** Chrome/Edge | ✅ | With a current Improv SDK (write-with-response) — see below |
| **Windows / Linux** Chrome/Edge | ✅ | Web Bluetooth supported |
| **iOS** (any browser) | ❌ | No Web Bluetooth on iOS at all |

### macOS Chrome — works now (was a browser-SDK bug)

The ESP build couldn't provision from macOS; that turned out to be **not the device**. The Improv JS
SDK wrote credentials with `writeValueWithoutResponse()`, which **Chrome on macOS silently drops**
(CoreBluetooth flow-control quirk) — so the write never reached *any* device (it reproduced on the
ESP *and* the Pico). It's fixed upstream
([improv-wifi/sdk-ble-js#213](https://github.com/improv-wifi/sdk-ble-js/issues/213), PR #217, Dec
2025) by using **write-with-response**, which our characteristic already supports — **no firmware
change**. Provision from any client on the fixed SDK; `web/improv-test/` uses the correct path (and
can A/B the bug). If `improv-wifi.com` still fails on macOS, clear its cached PWA (stale SDK). See
[gotchas.md](gotchas.md) for the full diagnosis.

## Boot behaviour

```
boot ─► panel: Booting ─► load creds from flash
         │ found                    │ none
         ▼                          ▼
   Wi-Fi connect ──fail──────►  SETUP MODE (panel: SETUP; BLE advertise "pixel64")
         │ ok                         on Improv "send Wi-Fi" RPC (auto-authorized):
         ▼                              control.join() + DHCP, persist, report IP over BLE
   ONLINE (panel: IP)                  └─ success ─► ONLINE
```

- Credentials that **fail to connect** (changed/unreachable network) fall through to setup, so the
  device re-provisions itself without intervention.
- **Bad provisioning input fails fast, never hangs:** the passphrase length is validated (WPA2
  8–63 chars) before the driver sees it, and the join is bounded by a 20 s timeout — a wrong
  SSID/password shows `FAILED` on the panel and the Improv client gets an error, ready to retry.
- **Factory reset:** hold **BOOTSEL ~3 s** while the device is running → credentials are wiped and it
  reboots into setup. (Implemented via a custom RP2350 BOOTSEL read — `src/bootsel.rs` — since
  embassy-rp 0.10's `bootsel` is RP2040-only. Holding BOOTSEL *at power-on* still enters flashing
  mode; the two are independent. See [gotchas.md](gotchas.md).)

## Architecture

| Module | Role |
|--------|------|
| [src/improv.rs](../src/improv.rs) | Improv GATT service over `trouble-host` on cyw43's BLE controller; parses the send-Wi-Fi RPC, drives the connection, reports state/result; persists on success |
| [src/net.rs](../src/net.rs) | Wi-Fi join via cyw43 `Control` + embassy-net DHCP; `connect(ssid, pw) -> Ipv4` |
| [src/storage.rs](../src/storage.rs) | Credentials in flash via `sequential-storage` (CRC + power-fail safe), in a reserved top-of-flash region |
| [src/display.rs](../src/display.rs) | Status screens (Booting / Connecting / Setup / Online / Failed) |
| [src/bin/main.rs](../src/bin/main.rs) | Boot state machine |

### Key decisions & gotchas

- **One radio, two handles.** On the CYW43439, `cyw43::new_with_bluetooth` yields a `Control`
  (Wi-Fi/LED), a `BtDriver` (BLE), and a `NetDriver` (embassy-net) from a single runner. Wi-Fi and
  BLE coexist by design — there's no separate "coex" feature or lazy-radio dance (both were ESP
  esp-radio specifics). Joining Wi-Fi *while the BLE link is up* (to report `Provisioned` + the URL)
  is the concurrency this was spiked on; it works.
- **Credential-write integrity.** `parse_send_wifi` reconstructs the Improv length byte from the
  packet structure and validates the checksum, to tolerate an intermittent cyw43 BLE byte-1
  corruption (see [gotchas.md](gotchas.md)) — creds are accepted only when the checksum proves them
  intact, else the client retries.
- **Answer connection-param requests.** macOS-style centrals request a connection-parameter update;
  trouble surfaces it and it must be accepted (`connection-params-update` feature) or the link can
  stall.
- **Long-write handling.** Some clients send the credential write as a BLE *long write*
  (Prepare/Execute), which `trouble` surfaces as `GattEvent::Other` and commits to the
  characteristic's backing store *without* a `Write` event. So after every GATT event we read
  `rpc_command` from the backing store (`server.get`) — catching both simple and long writes.
- **Keep the radio + stack alive.** `control` and the embassy-net `Stack` must live for the whole
  program; `main` holds them (it never returns), and the cyw43 + net runner tasks run forever.

## Crate stack

`cyw43` (Wi-Fi + BLE) · `trouble-host` 0.6 (BLE GATT) · `embassy-net` (DHCP/IP) ·
`sequential-storage` + `embassy-embedded-hal` (credentials) · `hub75` + `embedded-graphics` (display).
See [pico-port.md](pico-port.md) for versions and the dependency-compatibility constraints.
