# Gotchas, subtleties & debugging notes

Hard-won knowledge from bring-up. **Read this before debugging display or BLE issues** — almost
every item below cost real time to discover.

## Dependency landmines

The known-good version set (don't bump blindly — several of these are tightly coupled):

| Crate | Version | Notes |
|-------|---------|-------|
| `esp-hal` | `~1.1` (1.1.1) | features `esp32c6`, `unstable` |
| `esp-rtos` | `0.3.0` | features include **`esp-radio`** (required for radio) + `embassy` |
| `esp-radio` | `1.0.0-beta.0` | the **renamed `esp-wifi`**; features `wifi`, `ble`, `coex`, `unstable`; needs a heap |
| `trouble-host` | `0.6.0` | features `default-packet-pool-mtu-255`, `derive`, `scan`; uses `bt-hci 0.8` |
| `embassy-net` | `0.8` | DHCP/IP |
| `embassy-sync` | **`0.7`** | see split below |
| `esp-hub75` | `0.11` | esp-hal 1.1-based; drives HUB75 via PARL_IO+DMA on the C6 |
| `sequential-storage` / `esp-storage` / `embassy-embedded-hal` | `7` / `0.9` / `0.5` | creds (BlockingAsync adapts blocking flash → async) |

- **`embassy-sync` version split.** trouble-host 0.6 pins `embassy-sync ^0.7`; esp-radio pins `^0.8`.
  They coexist as **two compiled versions**. *Our crate must use 0.7* so the trouble `gatt_server`
  macro expands against the same version trouble uses. Do **not** "upgrade" to 0.8 — the macro
  breaks. (esp-hub75 internally pulls 0.6.x too; three versions coexisting is fine.)
- **esp-radio is the no_std / esp-hal world, NOT esp-idf.** Do not reach for esp-idf provisioning
  or `esp-matter` — switching to esp-idf-hal would **break esp-hub75** (it's esp-hal/PARL_IO-only;
  esp-idf-hal has no PARL_IO driver). Verified.
- **`coex` is required**, not optional: provisioning connects Wi-Fi *while BLE is still up* (to
  report status), so both radios run concurrently. That needs the `coex` feature + a 128 KB heap.
- `esp_hal::Async` is **`!Send`** → the BLE and display drivers can't move to an interrupt
  executor's `SendSpawner`. Everything runs on the thread-mode executor. (This also kills the
  "run the panel refresh at high priority" idea.)

## Display / HUB75 subtleties

- **Pin 12 silk says "GND" but is actually the D address line** (this 64-row panel reuses the
  32-row silkscreen). Wire it as D → GPIO0. Symptom if wrong: image split/duplicated vertically.
  See [hardware-wiring.md](hardware-wiring.md).
- **GPIO8 is the onboard WS2812 RGB LED**, not a free pin. We moved HUB75 `B` to **GPIO14** and
  hold GPIO8 low.
- **The WS2812 retains its colour across a SOFT reset.** `cargo run`/espflash resets the CPU but
  doesn't cut power, so the LED keeps whatever it last latched (e.g. bright white from earlier
  firmware that toggled GPIO8 as an address line). Holding the line low does **not** clear it —
  only a real **power cycle** (unplug USB) or an explicit `(0,0,0)` frame does. Diagnostic: "the
  onboard LED won't turn off after reflashing" → power-cycle once; it then stays dark across soft
  resets (because our firmware never drives GPIO8 again).
- **Ghosting follows the drawn colour** (cyan ball → greenish ghost, blue ball → blue ghost) in
  the column below bright pixels. This is normal HUB75 column ghosting (OE/latch timing), **not a
  code bug**.
- **Flicker on radio-active screens (ONLINE / setup) is single-core contention.** esp-hub75
  refreshes frame-by-frame with a **CPU-managed gap between DMA transfers**; the radio steals CPU
  and widens that gap → visible flicker. `PowerSaveMode::None` was tried (see net.rs) and made it
  **slightly worse** — it's a **revert candidate**. The real fix is the Pico's PIO (hardware-timed
  refresh, fully radio-decoupled). See *Open issues*.
- **GPIO10 / GPIO11 are not broken out** on the DevKitM-1. Avoid GPIO12/13 (USB-JTAG), GPIO16/17
  (USB-UART), GPIO9 (BOOT strap).

## BLE / Improv subtleties (trouble-host 0.6)

- **GATT handle map** (Improv service, observed from logs): `current_state` value=9 CCCD=10,
  `error_state` value=12 CCCD=13, `rpc_command` value=15, `rpc_result` value=17 CCCD=18,
  `capabilities` value=20. Useful when reading raw `handle` numbers in logs.
- **`GattEvent::Other` is a catch-all.** trouble only surfaces *simple* `Read`/`Write` as those
  variants; **service discovery, MTU exchange, and long writes (Prepare/Execute) all arrive as
  `Other`.** Don't assume `Other` is uninteresting.
- **Read the credential write from the backing store, not from a `Write` event.** After every
  accepted GATT event we do `server.get(&rpc_command)` and process it if non-empty. This catches
  *both* the simple write Android sends *and* long writes (which never surface as `Write`).
  Background: trouble's `handle_prepare_write` commits the value to the attribute store but (a)
  reports it as `Other`, and (b) its Prepare-Write **response omits the value echo** (a spec
  violation that can make some clients abort) — so don't depend on trouble's long-write path.
- **Initial values** via `server.set(&char, &val)` are returned to client *reads*; runtime state
  changes must use `char.notify(&conn, &val)` — the Improv client relies on **notifications**, not
  re-reads, to advance from Provisioning → Provisioned.
- **ATT MTU starts at 23 and negotiates to ~251** (`default-packet-pool-mtu-255` → server max =
  MTU−4). macOS runs the MTU exchange a beat *after* connecting.

### ⚠️ macOS Chrome doesn't work (and what we ruled out)

The browser **issues** the credential write (confirmed in Chrome console: `RPC COMMAND …`), but it
**never reaches the device's host stack** (no `[host]` log, no `gatt write handle 15`) — it's lost
at the **esp-radio ↔ CoreBluetooth link layer** once the connection goes idle while you type.

- **Ruled out:** display DMA interference (fails identically with the panel disabled) and Wi-Fi/BLE
  coex (fails identically with `coex` removed; and Wi-Fi isn't even running yet during setup —
  lazy init).
- Consistent with [esp-idf #11280](https://github.com/espressif/esp-idf/issues/11280) and Apple's
  post-connect connection-parameter update behavior.
- **Works on Android Chrome (verified end-to-end) and, expected, Windows/Linux Chrome.** iOS never
  works (no Web Bluetooth at all). Likely resolved by moving to the **Pico 2 W** (cyw43 + trouble).

## Debugging playbook (BLE)

- **See the link layer:** add the `log` feature to `trouble-host` in Cargo.toml → `[host]` lines
  print "agreed att MTU", packet-pool size, device address, etc. (Removed by default; re-add when
  debugging.)
- **Stuck vs. idle?** Log right before `conn.next().await` ("awaiting…") and right after
  `event.accept()` ("handled"). If "awaiting…" prints after the last event, the device is *healthy
  and idle* — the missing data is client/link-side, not ours.
- **Client side:** Chrome DevTools → Console → **Verbose** level shows the Improv SDK's
  `RPC COMMAND` (the actual write) and `improv current state` logs — they're all `debug` level and
  hidden by default.
- **Isolate platform vs. device:** run the *same* firmware from Android Chrome and macOS Chrome.
  Android working + macOS not ⇒ platform/link interop, not our code. (This is how we cornered the
  macOS issue.)
- **Recover a killed Workflow's research:** structured agent outputs live in
  `…/subagents/workflows/<run-id>/agent-*.jsonl` — grep for the `StructuredOutput` tool_use input.

## Wi-Fi lifetime

- **Keep `WifiController` + `Stack` alive for the whole program**, or the connection drops after
  connecting. `main` binds them as `_wifi`/`_stack` (held, not dropped); `improv::run_setup`
  *returns* them from the provisioner so they survive setup. Don't let them fall out of scope —
  "ONLINE" must mean online.

## Open issues / next steps

1. **Flicker** on radio-active screens — single-core radio↔refresh contention. `PowerSaveMode::None`
   is currently set and was *slightly worse* (revert it first). Real fix: **Pico 2 W PIO**.
2. **macOS Chrome provisioning** — link-layer interop wall on esp-radio; use Android for now.
3. **Pico 2 W migration** (hardware arriving) — the Improv / display / storage *architecture* ports
   over; the HAL + radio layers change (cyw43 for Wi-Fi/BLE, RP2350 **PIO** for HUB75, which also
   kills the flicker).
