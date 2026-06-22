# Gotchas, subtleties & debugging notes

Hard-won knowledge from the Pico 2 W bring-up + the ESP32→RP2350 port. **Read this before debugging
display or BLE issues.** The dependency-version story and the deeper port findings live in
[pico-port.md](pico-port.md); this is the field guide.

## Dependency landmines

The known-good set (don't bump blindly — several are tightly coupled). Full table + the
upgrade-evaluation recipe in [pico-port.md](pico-port.md).

- **`cyw43` and `trouble-host` must agree on the `bt-hci` major version.** cyw43's `BtDriver`
  implements a given `bt-hci`'s `Transport`; trouble's `ExternalController` wraps that *same*
  `bt-hci`. We pair **cyw43 0.7 + trouble-host 0.6** (both `bt-hci 0.8`). **Do not bump trouble to
  0.7** — it needs `bt-hci 0.9`, which cyw43 0.7 can't provide; the types won't line up.
- **The embassy-sync 0.7/0.8 split is back** (because of the above). trouble 0.6 pulls
  `embassy-sync 0.7`; the rest of the stack uses `0.8`. We add a **direct `embassy-sync = "0.7"`** so
  trouble's `#[gatt_server]` macro expands against the version trouble was built with. The two
  versions coexist as separate compiled crates — fine. (This was *gone* until BLE came in; see
  pico-port.md for how to retire it on a future upgrade.)
- **No heap.** The whole stack is static — no `esp-alloc`/`embedded-alloc`. Don't add one unless your
  own code needs `alloc`.
- **Target is ARM** (`thumbv8m.main-none-eabihf`). The RP2350's RISC-V cores aren't supported by
  embassy-rp.

## Display / HUB75 subtleties

- **RP2350 PIO pins don't reset to RP2040 defaults — set pin dirs explicitly.** The driver calls
  `sm.set_pin_dirs(Direction::Out, …)` for every HUB75 pin before enabling the SMs. Skip it and you
  get no/garbage output. (The #1 RP2040→RP2350 PIO porting bite.)
- **Pin 12 silk says "GND" but is the D address line** — wire it to **GP12**, not ground (see
  [hardware-wiring.md](hardware-wiring.md)). Symptom if wrong: image split/duplicated vertically.
- **The image is 180° from the draw origin.** The driver carries hub75-pio's un-mirror convention
  (`mov pins, ~x` + `W-1-x`/`H-1-y`). The panel is square / free-orientation — **rotate it to suit**
  rather than fighting the code.
- **BCM color depth has an LSB floor.** A bit-plane can't display for less than the pixel-shift time,
  so low planes are floored and the bright end of a gradient saturates. For smooth gradients, raise
  `OE_DIV` and add a gamma LUT (see [performance.md](performance.md)). Solid-color text is unaffected.
- **HUB75 column ghosting follows the drawn color** (a bright pixel casts a faint tint in the column
  below). Normal HUB75 OE/latch timing, **not a code bug**.
- **The DMA chain is hand-rolled against `embassy_rp::pac`.** embassy's safe DMA API doesn't expose
  channel chaining / `al2_write_addr_trig`, so the 4-channel self-restarting loop pokes registers
  directly. It's the most fragile part of the driver — if the panel is black, suspect the DMA setup
  first (verify with `cargo run --bin hub75test`).
- **cyw43 owns PIO0 + DMA_CH0/CH1.** HUB75 uses **PIO1 + DMA_CH2–CH5**. The onboard LED is on the
  cyw43 chip (`control.gpio_set(0, …)`), available only after cyw43 init — not a GPIO.

## BLE / Improv subtleties (cyw43 + trouble-host 0.6)

- **⚠️ cyw43 BLE byte-1 corruption.** GATT-write values occasionally arrive with **byte index 1
  decremented by one** — reproducible, intermittent, and *masked by logging latency* (a timing race
  in the cyw43 BT RX path or trouble's attribute write, below our code). For the Improv send-wifi RPC,
  byte 1 is the redundant length field; `parse_send_wifi` therefore **reconstructs the length from the
  packet structure and validates the Improv checksum** (which covers the SSID/password), so creds are
  accepted only when provably intact — else rejected and the client retries. Don't "simplify" that
  back to trusting the length byte.
- **`GattEvent::Other` is a catch-all.** trouble surfaces *simple* `Read`/`Write` as those variants;
  service discovery, MTU exchange, and long writes all arrive as `Other`. After every accepted GATT
  event we `server.get(&rpc_command)` and process if non-empty — catching both the simple write and a
  long write (which never surfaces as `Write`).
- **`RequestConnectionParams` must be answered.** trouble surfaces a connection-parameter-update
  request that *must* be `accept()`ed/`reject()`ed (we accept). Swallowing it can stall the link with
  some centrals. (Needs the `connection-params-update` feature.)
- **Notifications, not re-reads.** Runtime state changes (`current_state` Provisioning→Provisioned,
  `rpc_result`) reach the client via `char.notify(&conn, …)`. `server.set()` only affects what a
  *read* returns; the Improv client relies on notifications to advance.
- **ATT MTU** negotiates up (macOS reaches 251); reads/writes that fit don't fragment.

## Wi-Fi join: cyw43 can hang instead of erroring

`cyw43::Control::join` has **no internal timeout** — `wait_for_join` loops waiting for a result
event. A *wrong SSID* yields `NetworkNotFound` and a *wrong (but valid-length) password* usually
yields `AuthenticationFailure`, both of which return — but an **invalid-length passphrase** (e.g. a
7-char WPA2 password; valid is 8–63) makes the firmware never emit an auth event, so it **hangs
forever**. Defenses, all in place (`src/net.rs` / `src/improv.rs`):

- **Validate passphrase length before joining** (`improv.rs` rejects non-empty length outside 8–63 →
  instant "bad password", never reaches the driver). This is the real fix for the common typo.
- **Bound `join` with a 20 s timeout** (`select(join, Timer)`) as a backstop for other hangs.
- **`leave()` (disassociate) before every join.** Abandoning `join` on timeout is cancel-unsafe —
  it can leave cyw43 mid-association so the *next* join hangs. `leave()` resets the state so retries
  are clean.

## macOS Chrome provisioning — SOLVED (was a browser bug)

The ESP build's macOS wall was **not** the device. The Improv JS SDK wrote credentials with
`writeValueWithoutResponse()`, which **Chrome on macOS silently drops** (CoreBluetooth's flaky
`canSendWriteWithoutResponse` flow control, worst after the idle typing window) — so the write never
left the Mac, which is why it reproduced identically on esp-radio *and* cyw43. Fixed upstream in
[improv-wifi/sdk-ble-js#213](https://github.com/improv-wifi/sdk-ble-js/issues/213) (PR #217, Dec
2025) by switching to `writeValue()` (write *with* response). Our `rpc_command` characteristic
already supports write-with-response, so **no firmware change** — just use a client on the fixed SDK.
`web/improv-test/` is a controlled client that uses the correct path (and has a checkbox to reproduce
the bug). A stale `improv-wifi.com` PWA cache can still serve the old SDK — clear site data.

## Flash / storage

- **Writing flash pauses XIP** (embassy-rp pauses the core during erase/write). Credential saves are
  a one-time ~ms stall; the PIO+DMA panel refresh is hardware so it keeps running through it. The
  cyw43 link layer is autonomous and survives the brief host pause.
- **Creds live in a reserved region at the top of flash** (last 16 KiB, kept out of `memory.x`'s
  `FLASH` length), via `sequential-storage`. No esp-idf partition table — a fixed offset.
- **Factory reset (hold BOOTSEL ~3 s while running).** `embassy_rp::bootsel` is gated to **RP2040**
  in 0.10, so we hand-rolled the RP2350 read in `src/bootsel.rs` (ported from embassy `main`: the
  QSPI-SS pin is IO_QSPI **`gpio(3)`** on RP2350 vs `gpio(1)` on RP2040; OEOVER=DISABLE to float CS,
  read `status().infrompad()`, run from RAM with IRQs off). It needs a minimal `in_ram` since
  embassy's is `pub(crate)` — a `critical_section` suffices here because no DMA reads flash. The
  runtime read is independent of the power-on bootrom BOOTSEL sampling (flashing mode). `Oeover` is
  `DISABLE` (all-caps) in rp-pac 7.0.0, not `Disable`.

## Build / flash gotchas

- **Use `picotool`, not `elf2uf2-rs`.** The latter (even 2.2.0) tags UF2s with the **RP2040** family
  id (`0xe48bff56`), which the RP2350 bootrom rejects. `picotool` tags `rp2350-arm-s` correctly.
- **No-probe USB-serial caveats.** Logs come over USB-CDC, which disappears on reset (~1 s gap) and
  **a panic before USB enumerates is invisible**. Re-flashing needs a manual BOOTSEL hold (our
  firmware doesn't expose picotool's reset-to-BOOTSEL interface). A debug probe is an optional upgrade
  (swap the runner to `probe-rs run --chip RP235x`).
- **Two serial interfaces enumerate** — the lower-numbered `…01` is the data interface; the other is
  the CDC control endpoint (silent).

## Debugging playbook (BLE)

- **See the link layer:** add the `log` feature to `trouble-host` → `[host]`/`[link]` lines print
  (agreed ATT MTU, connection events, etc.). Bump the USB logger to `Debug` to capture them. Strip
  both when done. (This is how the macOS write-loss was localized — and note the byte-1 corruption is
  *masked* by this added latency, a useful Heisenbug signal that it's a timing race.)
- **Localize a lost write:** if `[host]` never logs the write and no GATT event fires, it's lost
  below trouble (controller/link or — as with macOS — never sent by the client). Confirm client-side
  with `chrome://bluetooth-internals` (its writes are write-with-response and always work) and
  DevTools console at Verbose.
- **Recover a killed Workflow's research:** structured agent outputs live in
  `…/subagents/workflows/<run-id>/agent-*.jsonl` — grep for the `StructuredOutput` tool_use input.
