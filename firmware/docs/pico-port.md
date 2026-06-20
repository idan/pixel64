# Porting to the Raspberry Pi Pico 2 W (RP2350)

Migration plan: move the firmware off the **ESP32-C6 / esp-hal** stack onto the **Raspberry Pi
Pico 2 W (RP2350, ARM Cortex-M33) / embassy-rp** stack. Going forward the Pico 2 W is the **only**
target — the ESP32 code is being replaced in place, not maintained in parallel.

This doc is the working plan; it captures the ecosystem state (verified mid-2026), what ports
cleanly, the two genuinely risky parts, and the staged sequence. Update it as the port progresses.

## Why this is mostly a HAL swap, not a rewrite

The firmware already splits along the seam that matters: `display` / `net` / `improv` / `storage`
are four self-contained modules over a thin HAL surface. The **application logic in each ports
unchanged**; only the bottom layer (peripheral driver / radio controller) swaps.

| Module | Application logic (ports ~as-is) | What actually changes |
|--------|----------------------------------|------------------------|
| `improv.rs` | The entire Improv GATT server, RPC parsing, state machine, the `Server`/`ImprovService` `#[gatt_server]` — **all trouble-host, unchanged** | The BLE **controller**: `esp-radio` `BleConnector` → `cyw43` `BtDriver`, both wrapped in `bt-hci`'s `ExternalController`. trouble-host 0.6 → 0.7. |
| `net.rs` | embassy-net stack, DHCP, `connect()` flow | Driver: `esp-radio` `WifiController` → `cyw43` `Control` + `NetDriver`. `set_power_saving` knob goes away (different mechanism, may not be needed — see flicker note). |
| `storage.rs` | `sequential-storage` map, serialize/parse, save/load/clear | Flash handle: `esp-storage` `FlashStorage` → `embassy_rp::flash::Flash`. **Drop the esp-idf partition-table lookup** — pick a fixed flash offset instead. |
| `display.rs` | Double-buffered refresh/draw task pair, `Screen` enum, embedded-graphics rendering | The **driver**: `esp-hub75` (PARL_IO+DMA) → **a custom RP2350 PIO+DMA HUB75 driver we must write** (no maintained crate exists — see Risk 1). The `DmaFrameBuffer` type and `Hub75Driver` change; the task structure stays. |
| `main.rs` | Boot state machine (stored creds → connect, else Improv setup), factory-reset loop | HAL init (`esp_hal::init` → `embassy_rp::init`), `#[esp_rtos::main]` → `#[embassy_executor::main]`, pin map, onboard-LED + BOOT-button specifics (both differ on the Pico — see below). Drop `esp-alloc` (stack is fully static now). |

## Target dependency set (verified mid-2026)

Cross-checked against crates.io and the embassy `main` examples. The big simplification: **with
`esp-radio` gone, the `embassy-sync` 0.7-vs-0.8 split disappears** — trouble-host 0.7 and the whole
embassy stack use **embassy-sync 0.8** uniformly. No more pinning.

| Crate | Version | Notes |
|-------|---------|-------|
| `embassy-rp` | `0.10` | features `rp235xa` (Pico 2 W = RP2350A), `time-driver`, `critical-section-impl`, `binary-info`, `unstable-pac` |
| `embassy-executor` | `0.10` | `arch-cortex-m`, `executor-thread` (+ `executor-interrupt` if we use core1) |
| `embassy-net` | `0.9` | **same crate we already use** — ports cleanly |
| `embassy-time` | `0.5` | |
| `embassy-sync` | `0.7` **+** `0.8` | split is back via BLE — `0.7` (direct dep) for trouble's gatt macros, `0.8` for the rest; see §"Dependency compatibility" |
| `cyw43` | `0.7` | Wi-Fi **and** BLE for the CYW43439; `new_with_bluetooth()` yields net + BT + control + runner |
| `cyw43-pio` | `0.10` | drives the CYW43439 over PIO-emulated SPI; use `RM2_CLOCK_DIVIDER` |
| `trouble-host` | `0.6` | pinned to match cyw43's bt-hci 0.8 (not 0.7); Improv GATT ports ~verbatim from the ESP build |
| `bt-hci` | `0.8` | `ExternalController` wrapper (transitive via trouble-host); must match cyw43 |
| `sequential-storage` | `7.2` | **our v7 code ports directly** |
| `embedded-graphics` | `0.8` | unchanged |
| `cortex-m-rt` | `0.7` | replaces the esp boot/runtime |
| `embassy-usb` / `embassy-usb-logger` | `0.6` | **chosen logging path** — `log::info!` over USB-CDC serial, no probe (keeps the ESP firmware's `log` API; no defmt) |
| ~~`esp-alloc`~~ | — | **dropped** — embassy-rp + cyw43 + embassy-net + trouble-host are fully static / no-heap |

### Dependency compatibility: the bt-hci / embassy-sync split (and how to retire it)

**Correction to the simplification above.** Dropping esp-radio removes the embassy-sync split *only
if you don't use BLE*. Bringing BLE in over crates.io drags it back, because of a hard constraint:

> **cyw43 and trouble-host must agree on the `bt-hci` major version.** cyw43's `BtDriver` implements
> `bt-hci`'s `Transport`; trouble-host's `ExternalController` wraps that *same* `bt-hci`'s
> `Transport`/`Controller`. If the two crates resolve to different `bt-hci` majors, the types don't
> line up and `ExternalController::new(bt_driver)` won't compile.

State of the crates.io releases (verified mid-2026):

| crate | bt-hci | embassy-sync |
|-------|--------|--------------|
| `cyw43` 0.7.0 | **0.8** | 0.8 |
| `trouble-host` 0.6.0 | **0.8** ✓ | **0.7** |
| `trouble-host` 0.7.0 | 0.9 ✗ | 0.8 |

No single crates.io pair is aligned on **both** axes. We pair **cyw43 0.7.0 + trouble-host 0.6.0**
(aligned on bt-hci 0.8 — this compiles), and pay for it with an **embassy-sync split**: trouble 0.6
pulls embassy-sync **0.7**, the rest of the stack uses **0.8**, and they coexist as two compiled
versions. We add a direct `embassy-sync = "0.7"` dep so the trouble `#[gatt_server]` /
`#[gatt_service]` macros expand against the same 0.7 that trouble was built with. (This is exactly
the arrangement the ESP build used; it's a known, working quirk — not a bug.)

**How to check whether a future release lets us delete the split** — the goal is one `bt-hci` and one
`embassy-sync` (0.8) across the whole graph:

1. Pick the cyw43 version you want (usually latest). Read its `bt-hci` and `embassy-sync`
   requirements: `cargo info cyw43@<ver>` or look at `[dependencies.bt-hci]` /
   `[dependencies.embassy-sync]` in its `Cargo.toml`.
2. Find a `trouble-host` version whose `bt-hci` major **equals cyw43's** *and* whose `embassy-sync`
   major **equals the rest of the embassy stack's** (0.8). List all trouble versions + their reqs
   straight from the index cache:
   ```sh
   python3 - <<'PY'
   import re
   raw=open(__import__('glob').glob('/Users/*/.cargo/registry/index/*/.cache/tr/ou/trouble-host')[0],
            encoding='utf-8',errors='ignore').read()
   starts=[m.start() for m in re.finditer(r'\{"name":"trouble-host","vers":', raw)]+[len(raw)]
   for a,b in zip(starts,starts[1:]):
       seg=raw[a:b]; v=re.search(r'"vers":"([^"]+)"',seg).group(1)
       req=lambda n:(re.search(r'\{"name":"%s"[^}]*?"req":"([^"]+)"'%n,seg) or [None,'-'])[1]
       print(f"{v:8} bt-hci={req('bt-hci'):8} embassy-sync={req('embassy-sync')}")
   PY
   ```
   (run `cargo update -p trouble-host --dry-run` or any fetch first so the cache is current.)
3. If such a version exists, the most likely trigger is **a cyw43 release that moves to bt-hci 0.9**
   (matching trouble 0.7) — then cyw43 + trouble 0.7 align on bt-hci 0.9 *and* both sit on
   embassy-sync 0.8.
4. **Verify before committing:** bump the versions, delete the `embassy-sync = "0.7"` line,
   `cargo build`, and confirm the graph collapsed:
   ```sh
   grep -c 'name = "bt-hci"' Cargo.lock        # want: 1
   grep -A1 'name = "embassy-sync"' Cargo.lock # want: a single 0.8.x
   ```
   If both hold and the build is clean, the split is gone — update this table and drop the pin.

The git-pinned `[patch]` route (embassy/trouble `main` at one rev, where everything already aligns on
bt-hci 0.9 + embassy-sync 0.8) is the other way to a no-split stack; we chose the crates.io pairing
to keep the working baseline on pinned releases (see *Decisions*).

**Reference to track:** the trouble repo ships an official Pico 2 W example —
`embassy-rs/trouble/examples/rp-pico-2-w/` (`ble_bas_peripheral.rs`) — with a working
`Cargo.toml` + `memory.x` + `.cargo/config.toml`. It `[patch]`es all embassy crates to a pinned
`embassy.git` rev; we should match that rev so cyw43/embassy-rp/trouble line up. Clone it first and
build it on the actual board as the baseline before porting our code on top.

### Toolchain / build scaffolding

- **Target triple `thumbv8m.main-none-eabihf`** (Cortex-M33, hard-float). Stick with ARM — the
  RP2350's Hazard3 RISC-V cores are *not* supported by embassy-rp.
- `rust-toolchain.toml`: swap the target to `thumbv8m.main-none-eabihf` (keep `rust-src`,
  `build-std = ["core"]`).
- `memory.x` (new): `FLASH ORIGIN 0x10000000 LENGTH 4096K` (Pico 2 W = 4 MB), `RAM 0x20000000
  LENGTH 512K`, plus the RP2350 `.start_block` / `.bi_entries` / `.end_block` sections from the
  example. The `imagedef-secure-exe` feature injects the RP2350 image-def block automatically — no
  hand-written `ImageDef`.
- `build.rs`: replace the esp linker-arg logic with the cortex-m `--nmagic -Tlink.x` (+ `-Tdefmt.x`
  if defmt). `esp_app_desc!()` / `esp-bootloader-esp-idf` are gone.
- `.cargo/config.toml`: runner is **`picotool load -u -v -x -t elf`** (no-probe path — flashes the
  ELF over USB to a board in BOOTSEL mode). **Not `elf2uf2-rs`** — that tool (latest 2.2.0) only
  emits the RP2040 UF2 family id `0xe48bff56`, which the RP2350 bootrom rejects; picotool tags the
  image `rp2350-arm-s` (`0xe48bff59`). `picotool` is a Homebrew install (`brew install picotool`).
- No `build-std` and no `force-frame-pointers` (both were ESP-isms) — the prebuilt `rust-std` for
  `thumbv8m.main-none-eabihf` is used.

## Risk 1 — the HUB75 driver (biggest effort) ⚠️

**There is no maintained, embassy-rp-native, RP2350-ready HUB75 crate.** The only Rust option,
`hub75-pio` (kjagiello), is stuck at v0.1.0 (2022), built on `rp2040-hal 0.6` (blocking, RP2040-only)
and `embedded-graphics 0.7`. Every actively-maintained RP2350 HUB75 driver is C/C++ or MicroPython.

**Plan: write a custom embassy-rp PIO+DMA HUB75 driver.** The hard part — the PIO assembly program
and the BCM (binary-coded-modulation) + self-restarting DMA-chain design — transfers almost verbatim
from existing drivers; the Rust glue (embassy-rp PIO/DMA HAL calls, the `DrawTarget` impl,
double-buffer hand-off) is what we write.

References to mine (PIO program + RP2350 timing, **not** as dependencies):
- `kjagiello/hub75-pio-rs` — the PIO program + BCM/DMA architecture, in Rust (just on the old HAL).
- `dgrantpete/Pi-Pico-Hub75-Driver` — C+MicroPython, **explicitly RP2040 *and* RP2350**, PIO+DMA,
  double-buffered, BCM, actively maintained (2026). Best RP2350-timed reference.
- `JuPfu/hub75` — C (pico-sdk), tested on RP2350A/B.

embassy-rp's PIO HAL targets rp235x and exposes `StateMachine` `tx_treq()` / FIFO pointers — enough
to wire DMA straight at the PIO TX FIFO (the `mem → PIO-TX` chained-DMA pattern these drivers use).

**RP2350 PIO gotchas to bake in from the start:**
- **Explicitly init every HUB75 GPIO before the SM runs.** RP2350 does *not* reset pins to the same
  default state as RP2040 — the #1 RP2040→RP2350 PIO porting bite (garbage/no output otherwise).
- Pico 2 W = RP2350A (30 GPIO); no GPIO-base offset juggling needed (that's RP2350B/48-pin).
- embassy-rp's RP2350 PIO coverage isn't 100% (e.g. issue #4067) — expect occasional missing-feature
  edges, though none expected to block HUB75.

**Upside:** PIO+DMA refresh is hardware-timed — the CPU only touches the framebuffer when *content*
changes. This is what **kills the flicker** (Risk: gone): refresh is fully decoupled from the radio,
unlike the ESP's CPU-managed gap between DMA transfers. Optionally we can park the refresh
bookkeeping on **core1** (`spawn_core1`) with the radio on core0, but PIO+DMA needs ~zero CPU so
that's a nicety, not a necessity. (If we do use core1, mind the flash-write XIP pause — see storage.)

## Risk 2 — concurrent Wi-Fi STA + live BLE GATT on cyw43 ⚠️

Our provisioning flow connects Wi-Fi **while the BLE GATT link is still up** (to report
Provisioning → Provisioned status to the Improv client). On the single CYW43439, Wi-Fi+BLE
coexistence is **supported by design** (`new_with_bluetooth` multiplexes HCI into the Wi-Fi event
loop over the shared SPI bus; the cyw43 README claims "Concurrent operation with WiFi") **but no
shipped example demonstrates Wi-Fi-STA-up + BLE-GATT-up simultaneously**, and it was historically
the hard part of the driver (early BT support regressed Wi-Fi; a `CYW43_THREAD_ENTER` bus-arbitration
TODO still lingers in cyw43's BT path).

**Mitigation — spike this before committing the architecture.** Bring up the `rp-pico-2-w` example,
then `control.join()` a Wi-Fi network while a central stays connected to the GATT link and confirm
the BLE notification path survives association. Our existing **lazy-Wi-Fi design helps**: BLE setup
runs radio-clean and Wi-Fi only starts on the first credential attempt, so the only contended window
is the brief "connecting → report status" moment. If concurrency proves flaky, fallbacks: persist
creds + report status *after* the join settles, or accept a short BLE stall during association, or
tear BLE down around the join.

While spiking, also **test macOS Chrome Web Bluetooth** — our ESP32 macOS failure was diagnosed as an
`esp-radio ↔ CoreBluetooth` link-layer issue, i.e. in exactly the controller layer we're replacing.
Swapping to the CYW43439's BT controller *could* fix it (plausible mechanism, no evidence either
way). Cheap to test; the only way to know.

## Smaller platform changes

- **Onboard LED is on the CYW43 chip, not a GPIO.** Drive it via `control.gpio_set(0, on).await` —
  and it's **unavailable until cyw43 is initialized** (a real difference from the ESP's GPIO8 LED;
  the whole WS2812 / GPIO8 saga is moot). No "hold a pin low" needed.
- **Factory reset (BOOT-hold):** the Pico has no BOOT GPIO. embassy-rp exposes
  `embassy_rp::bootsel::is_bootsel_pressed()` — readable at runtime (it briefly drives QSPI/CS to
  sense it). Idiomatic drop-in for our 3-s-hold reset, no extra hardware. Don't poll it concurrently
  with flash writes. (External GPIO button is the alternative — Decision 3.)
- **Flash region for creds:** no esp-idf partition table. Reserve a few 4 KB sectors at the **top of
  flash** (e.g. last 64–128 KB), kept out of `memory.x`'s `FLASH` length so the linker never places
  code there; pass that offset range to `sequential-storage`. `embassy_rp::flash::Flash` implements
  the `embedded-storage` NorFlash traits (ERASE_SIZE 4096) that v7 needs.
- **XIP / flash-write caveat:** writing flash pauses XIP; embassy-rp pauses both cores for the
  operation. Single-core (creds on core0) is straightforward; if/when core1 runs the display, be
  aware the write briefly stalls it. (`run-from-ram` feature avoids the pause but we run from flash.)
- **No heap.** Drop `esp-alloc` + the 128 KB `heap_allocator!`. Add `embedded-alloc` *only* if our
  own code later wants `alloc`.
- **Pin map:** new HUB75 pin assignment for the Pico. PIO wants the 6 RGB data pins (and ideally the
  control/address lines) arranged to suit `out`/`side-set`/consecutive-pin constraints — design the
  map around the PIO driver, not ported 1:1 from the C6. Reserve GPIO23–25 + 29 for the CYW43
  (PWR/CS/DIO/CLK, fixed by board wiring) and PIO0 for cyw43-pio — put HUB75 on PIO1 + free GPIOs.
  Level-shifting (74AHCT245) and the panel power story (VSYS, see hardware-wiring.md) are unchanged.

## Staged sequence

Spike the two risks **before** porting application code — they're the only things that can sink the
plan.

0. **Scaffold** ✅ *(done)* — new `Cargo.toml` / `memory.x` / `build.rs` / `.cargo/config.toml` /
   `rust-toolchain.toml` for RP2350; minimal `#[embassy_executor::main]` that logs a heartbeat over
   USB-serial. Builds clean for `thumbv8m.main-none-eabihf`; `picotool` produces a valid
   `rp2350-arm-s` UF2. **Next physical step: flash it and confirm logs appear over the one USB
   cable** — proves the no-probe dev loop before anything else is built on it.
1. **Baseline radio** ✅ *(done — commit ce103c6)* — cyw43 up over PIO0-SPI (PINs 23/24/25/29,
   DMA_CH0), Wi-Fi firmware blob uploaded, onboard LED blinking via `control.gpio_set(0, …)`.
   Verified on hardware. cyw43-firmware/ blobs vendored (incl. the BT fw for M2). Confirms the
   radio chip + firmware blobs + PIO-SPI link all work.
2. **Wi-Fi + BLE.**
   - **2a — Wi-Fi STA join** ✅ *(done — commit e4b0bac)* — embassy-net DHCP over the cyw43
     NetDriver, `control.join()`, online + IP on serial, solid LED. Creds via `WIFI_SSID`/
     `WIFI_PASS` compile-time env vars (Improv replaces this later).
   - **2b — BLE controller swap** ✅ *(done)* — `cyw43::new_with_bluetooth` (+ `bluetooth` feature +
     `43439A0_btfw.bin` blob) → `BtDriver` in trouble-host 0.6's `ExternalController`; advertises
     `pixel64` with a battery service. Verified on hardware (nRF Connect connects + reads). Pairing
     is cyw43 0.7 + trouble 0.6 on bt-hci 0.8 — see §"Dependency compatibility". The ESP `improv.rs`
     GATT patterns ported verbatim (still trouble 0.6).
   - **2c — Spike Risk 2 (concurrency)** — port the Improv GATT, then Wi-Fi-join while a BLE
     central holds the link; verify notifications survive. Test macOS Chrome here too. Decide on the
     provisioning-flow fallback if needed.
3. **Spike Risk 1** — minimal PIO+DMA HUB75 driver lighting the 64×64 panel with a test pattern
   (port the PIO program; prove DMA chain + BCM). The riskiest *build*.
4. **Port `storage.rs`** — swap flash handle, fixed offset. Smallest, self-contained; do it early.
6. **Port `net.rs`** — cyw43 `Control`/`NetDriver` behind the same `start`/`connect` API.
7. **Port `improv.rs`** — swap the BLE controller; trouble 0.6→0.7 API touch-ups.
8. **Port `display.rs`** — wrap the Risk-1 driver in the existing refresh/draw task pair + `Screen`.
9. **Wire `main.rs`** — boot state machine, BOOTSEL factory reset, cyw43 LED.
10. **Docs sweep** — rewrite hardware-wiring (Pico pin map), firmware, performance (PIO refresh
    math), wifi-onboarding, gotchas (retire ESP-only items, keep the Improv-protocol ones), README,
    and root `CLAUDE.md`.

## Decisions (resolved)

1. **Flashing — no probe.** Flash over the Pico's own USB via the BOOTSEL ROM bootloader using
   `picotool` (runner `picotool load -u -v -x -t elf`). A debug probe stays an optional later
   upgrade (swap the runner to `probe-rs run --chip RP235x`); it buys seamless `cargo run`, logs
   from the first instruction, and early-boot/panic visibility.
2. **Logging — USB-serial.** `embassy-usb-logger`, keeps the `log::info!` API over USB-CDC on the
   same cable. No defmt. Caveat: a panic before USB enumerates is invisible (no-probe limitation).
3. **Factory-reset button — BOOTSEL.** `embassy_rp::bootsel::is_bootsel_pressed()`, ~3-s hold; no
   extra hardware.
4. **Cutover — in place.** The ESP32 firmware is replaced in `firmware/`, not kept building in
   parallel. The full ESP32 tree is preserved at git tag **`esp32-final`** (port references pull
   from there, e.g. `git show esp32-final:firmware/src/improv.rs`).
</content>
</invoke>
