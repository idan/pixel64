# Wi-Fi onboarding (Improv over BLE)

First-run provisioning: the device gets onto the user's Wi-Fi using only a phone or laptop —
no app, no USB tool — via the open [Improv Wi-Fi](https://www.improv-wifi.com/ble/) standard
over Bluetooth LE.

## How to provision

1. Power on a device with no stored credentials → the panel shows **SETUP** and it advertises
   over BLE as **`pixel64`**.
2. On a supported browser, open **https://www.improv-wifi.com/ble/** → **Connect** → pick
   **`pixel64`** in the Bluetooth picker.
3. Enter your home SSID + password → **Submit**.
4. The panel shows **Connecting → ONLINE + IP**; the page shows success with the device URL
   (`http://<device-ip>`). Credentials are saved to flash, so subsequent boots connect
   automatically.

### Browser support

| Platform | Works? | Notes |
|----------|--------|-------|
| **Android** Chrome | ✅ | Verified end-to-end |
| **Windows / Linux** Chrome/Edge | ✅ (expected) | Web Bluetooth supported |
| **macOS** Chrome/Edge | ❌ | Known issue — see below |
| **iOS** (any browser) | ❌ | No Web Bluetooth on iOS at all |

### ⚠️ macOS Chrome limitation

macOS Chrome connects and reads the GATT service fine, but the **credential write never reaches
the device** — it's lost at the esp-radio ↔ CoreBluetooth link layer once the connection goes
idle (the host stack never receives the packet). We ruled out the display DMA and Wi-Fi/BLE
coex; it tracks with [esp-idf #11280](https://github.com/espressif/esp-idf/issues/11280) and
Apple's connection-parameter behavior. **Provision from Android (or Windows/Linux Chrome)
instead.** A future move to the Raspberry Pi Pico 2 W (CYW43439 + `cyw43`/`trouble`) would
likely resolve this.

## Boot behaviour

```
boot ─► load creds from flash (nvs partition)
         │ found            │ none / connect fails
         ▼                  ▼
   Wi-Fi connect ──fail──► SETUP MODE (BLE advertise "pixel64", Wi-Fi off)
         │ ok                 on Improv "send Wi-Fi" RPC (auto-authorized):
         ▼                      start Wi-Fi + connect, persist, report IP over BLE
   ONLINE (panel: IP)          └─ success ─► ONLINE
```

- **Factory reset:** hold the **BOOT** button (~3 s) while running → credentials are wiped and
  the device restarts into setup mode. (BOOT is a strapping pin, so it's polled at runtime;
  holding it across a power-on reset enters download mode instead — release it before the restart.)

## Architecture

| Module | Role |
|--------|------|
| [src/improv.rs](../src/improv.rs) | Improv GATT service over `trouble`; parses the send-Wi-Fi RPC, drives the connection, reports state/result |
| [src/net.rs](../src/net.rs) | esp-radio Wi-Fi STA + embassy-net DHCP; `connect(ssid, pw) -> Ipv4` |
| [src/storage.rs](../src/storage.rs) | Credentials in flash via `sequential-storage` (CRC + power-fail safe) |
| [src/display.rs](../src/display.rs) | Status screens (Booting / Connecting / Setup / Online / Failed) |
| [src/bin/main.rs](../src/bin/main.rs) | Boot state machine + BOOT-hold factory reset |

### Key decisions & gotchas

- **Lazy Wi-Fi.** Wi-Fi is *not* started during BLE setup — only once credentials arrive. This
  keeps the radio uncontended during BLE discovery (an early version started Wi-Fi up front and
  the coex contention starved the GATT stack and strobed the display).
- **Coex is required.** Provisioning connects Wi-Fi *while the BLE link is still up* so it can
  report `Provisioned` + the device URL (the Improv client waits for that before disconnecting).
  That brief overlap needs esp-radio's `coex` feature and a larger heap (128 KB).
- **Long-write handling.** Some clients send the credential write as a BLE *long write*
  (Prepare/Execute), which `trouble` surfaces as `GattEvent::Other` and commits to the
  characteristic's backing store *without* a `Write` event. So after every GATT event we read
  `rpc_command` from the backing store (`server.get`) — catching both simple and long writes.
- **Flash region.** Credentials reuse the `nvs` data partition's flash range (found via the
  partition table), not the esp-idf NVS *format* — this no_std firmware just borrows that
  otherwise-unused region.
- **Keep Wi-Fi alive.** The `WifiController` + `Stack` must live for the whole program or the
  connection drops right after connecting. `main` holds them as `_wifi`/`_stack`, and
  `run_setup` returns them from the provisioner. ("ONLINE" must actually mean online.)

> More traps and the BLE debugging playbook live in [gotchas.md](gotchas.md).

## Crate stack

`esp-radio` (Wi-Fi + BLE + coex) · `trouble-host` (BLE GATT) · `embassy-net` (DHCP/IP) ·
`sequential-storage` + `esp-storage` (credentials) · `esp-hub75` + `embedded-graphics` (display).
