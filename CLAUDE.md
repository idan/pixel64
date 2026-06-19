# pixel64 — project context

Monorepo for an internet-connected **64×64 HUB75 LED pixel display**:

- **`firmware/`** — the device: an **ESP32-C6-DevKitM-1** driving a **Waveshare P3 64×64 HUB75**
  panel, in **Rust (esp-hal 1.1 + Embassy, no_std)**. All Cargo/build/docs for the device live here.
- **`web/`** — (planned) the cloud backend + web UI: **Svelte 5 / SvelteKit on Cloudflare**, tooled
  with **Bun**. Not scaffolded yet.

Repo root holds only shared bits (this file, `.claude/`, `.gitignore`). **Run firmware commands from
`firmware/`** (e.g. `cd firmware && cargo run`).

## Current state (firmware)

- ✅ **Display** — esp-hub75 over PARL_IO + DMA, double-buffered, ~520 Hz, embedded-graphics.
- ✅ **Wi-Fi onboarding** — first-run provisioning via **Improv over BLE**. Provision from Chrome
  (Android / Windows / Linux — **not macOS**, see below), credentials persisted to flash, IP shown
  on the panel. Hold **BOOT ~3 s** = factory reset (wipe creds, re-enter setup).
- ⏳ **Open** — panel flicker on radio-active screens; macOS-Chrome provisioning; likely migration
  to a **Raspberry Pi Pico 2 W** for a better wireless/PIO story.

## Read before working

- **[firmware/docs/gotchas.md](firmware/docs/gotchas.md)** — hard-won subtleties, dependency
  landmines, the BLE debugging playbook, and the macOS/flicker conclusions. **Read before debugging.**
- [firmware/docs/README.md](firmware/docs/README.md) — docs index. Also: hardware-wiring, firmware,
  performance, wifi-onboarding.

## Constraints you should NOT relearn the hard way

- **no_std / esp-hal world — never esp-idf.** Switching to esp-idf-hal would break the esp-hub75
  display driver (PARL_IO is esp-hal-only).
- **`embassy-sync` is pinned to 0.7** (trouble-host compat); esp-radio is the renamed `esp-wifi`.
- **`esp_hal::Async` is `!Send`** → thread-mode executor only (no interrupt-executor for the
  drivers).
- **macOS Chrome BLE provisioning does not work** (esp-radio ↔ CoreBluetooth link-layer issue,
  not our code) — provision from Android. iOS never works (no Web Bluetooth).
- Pin map quirks: HUB75 `D` is the silk-"GND" pin 12; `B` is on GPIO14 (GPIO8 is the onboard LED);
  GPIO10/11 aren't broken out. See firmware/docs/hardware-wiring.md.

## Build / run

From **`firmware/`**: `cargo run` flashes + monitors (espflash, `riscv32imac-unknown-none-elf`);
`cargo build` / `cargo clippy` to check (both currently clean).
