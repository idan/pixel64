# pixel64 — project context

Monorepo for an internet-connected **64×64 HUB75 LED pixel display**:

- **`firmware/`** — the device: a **Raspberry Pi Pico 2 W (RP2350)** driving a **Waveshare P3 64×64
  HUB75** panel, in **Rust (embassy-rp, no_std)**. All Cargo/build/docs for the device live here.
- **`web/`** — the cloud backend + web UI: **Svelte 5 / SvelteKit on Cloudflare** (Workers + D1 +
  Drizzle), tooled with **Bun**. Currently a **procedural scene editor / simulator** spike (text
  buffer + live 64×64 preview); also hosts `web/improv-test/`, a zero-build Web Bluetooth
  provisioning test client. See [web/README.md](web/README.md) and
  [docs/scenes/authoring.md](docs/scenes/authoring.md).
- **`renderer/`** — the **shared scene renderer** (Rust): a shader-bytecode stack VM that compiles
  to **wasm32** (the web preview) and is meant to also drive the device above the framebuffer seam.
  See [renderer/README.md](renderer/README.md).

Repo root holds shared bits (this file, `.claude/`, `.gitignore`, `docs/scenes/`, `renderer/`).
**Run firmware commands from `firmware/`** (e.g. `cd firmware && cargo run`); web commands from
`web/` (`bun run dev`, `bun run build:wasm`).

The firmware was **ported from an ESP32-C6** to the Pico 2 W. The full ESP32 tree is preserved at git
tag **`esp32-final`**; the migration story and the verified dependency set live in
**[firmware/docs/pico-port.md](firmware/docs/pico-port.md)**.

## Current state (firmware) — feature-complete on the Pico 2 W

- ✅ **Display** — custom **HUB75 PIO+DMA driver** (`src/hub75.rs`): three PIO state machines on PIO1
  fed by a self-chaining 4-channel DMA loop, double-buffered, binary-code-modulation color,
  embedded-graphics. Refresh is hardware-timed → **no flicker** (the ESP build's radio-contention
  flicker is gone).
- ✅ **Wi-Fi onboarding** — first-run provisioning via **Improv over BLE** (cyw43 + trouble-host).
  Provision from Chrome on **Android / Windows / Linux *and* macOS** (the ESP's macOS wall was a
  browser-SDK bug, not the device — see below). Credentials **persist to flash**; the IP shows on the
  panel; on reboot it rejoins automatically (else re-enters setup). Hold **BOOTSEL ~3 s** while
  running = factory reset (wipe creds, reboot to setup). Bad creds fail fast (length-validated +
  join timeout) with a `FAILED` screen — never a hang.
- ✅ **Color calibration** — perceptual gamma LUT + BCM tuning dialed in on the panel (`GAMMA=2.2`,
  `B=10`, `OE_DIV=8`); gradients read evenly. Tools: `cargo run --bin calibrate` / `--bin refbench`.
- ✅ **Renderer on-device (MVP)** — the shared `renderer/` shader VM is wired into the firmware
  (`src/scene.rs`, `cargo run --bin scene`) and animates an embedded scene on the panel via the same
  `render_grid` the web preview uses. **Deferred** (see docs/scenes/device-runtime.md): flash scene
  store, network delivery, multi-layer compositor, live-input uniforms, core-1 offload, sin/cos LUT.
- ⏳ **Open** — building out the `web/` app (scaffolded; UI/backend still TBD) and the scene
  store/transport that feeds real scenes to the device.

## Read before working

- **[firmware/docs/pico-port.md](firmware/docs/pico-port.md)** — the port: the verified dependency
  set, the **dependency-compatibility constraints** (bt-hci / embassy-sync), the hard-won findings
  (cyw43 BLE byte-1 corruption, the macOS fix), and milestones. **Read before touching deps or BLE.**
- **[firmware/docs/gotchas.md](firmware/docs/gotchas.md)** — subtleties + landmines + the BLE
  debugging playbook. **Read before debugging display or BLE.**
- [firmware/docs/README.md](firmware/docs/README.md) — docs index (hardware-wiring, hub75-pico-wiring,
  firmware, performance, wifi-onboarding).

## Constraints you should NOT relearn the hard way

- **no_std / embassy-rp world.** Target **`thumbv8m.main-none-eabihf`** (ARM Cortex-M33); the RP2350's
  RISC-V (Hazard3) cores are **not** supported by embassy-rp.
- **Dependency compatibility is delicate.** `cyw43` and `trouble-host` must agree on the **bt-hci**
  major. We pair **cyw43 0.7 + trouble-host 0.6** (both bt-hci 0.8), which forces an **embassy-sync
  0.7/0.8 split** (a direct `embassy-sync = "0.7"` for trouble's `#[gatt_server]` macros). Do **not**
  bump trouble to 0.7 — it needs bt-hci 0.9, which cyw43 0.7 can't provide. See pico-port.md before
  upgrading anything in this cluster.
- **Radio owns PIO0 + DMA_CH0/CH1; HUB75 uses PIO1 + DMA_CH2–CH5.** The **onboard LED is on the cyw43
  chip** (`control.gpio_set(0, …)`), not a GPIO.
- **macOS Chrome provisioning works** — but only with an Improv client that uses **write-with-response**
  (improv-wifi/sdk-ble-js#213, fixed Dec 2025). `web/improv-test/` uses the correct path; a stale
  `improv-wifi.com` PWA cache can still fail. iOS never works (no Web Bluetooth).
- **Pin map:** HUB75 **`D` is the silk-"GND" pad 12** — wire to **GP12**, not ground. Full map +
  ESP32 migration table in [firmware/docs/hub75-pico-wiring.md](firmware/docs/hub75-pico-wiring.md).

## Build / run

From **`firmware/`**: `cargo run` builds + flashes the firmware over USB via **`picotool`** (hold
**BOOTSEL** while plugging in; `picotool` is a Homebrew install — no debug probe needed). Logs come
back over **USB-serial**: use **`tio /dev/cu.usbmodem*`** (`brew install tio`; Ctrl-T then Q to
quit). Use the **`cu.`** device, not `tty.` — the `tty.` call-in device blocks on carrier-detect a USB
CDC port never asserts, and a hung `tty.` monitor will hold the port ("resource busy" / "could not
find a PTY"); free it with `screen -wipe` / kill the stuck process. macOS's bundled `screen` (v4, 2006)
is buggy here — prefer `tio`. (`cu` hits uucp-lock permission errors.)
Diagnostics: `cargo run --bin firstlight` (bit-bang wiring test), `cargo run --bin hub75test` (PIO
driver test pattern), `cargo run --bin calibrate` (gamma/BCM calibration target), `cargo run --bin
refbench` (refresh-rate benchmark), `cargo run --bin scene` (shared shader VM rendering an embedded
scene on the panel). `cargo build` / `cargo clippy` to check (both currently clean; building the
firmware emits a harmless `dropping unsupported crate type cdylib` note from the renderer dep).

## Authoring PRs

PR descriptions must be **structured**, never one long paragraph. Always follow this shape:

- **Lead with a root analysis** — a short opening that states the *goal* the PR implements and the
  problem it solves (the "why" and "what it achieves"), not just a restatement of the title.
- **Then list the changes as bullets/notes** — concrete, scannable points describing what actually
  changed, not prose.
- **Split into logical sections** (with `##`/`###` headings) when the change set has distinct parts
  (e.g. firmware vs. web vs. renderer, or feature vs. refactor vs. tests/docs). A small single-purpose
  PR can stay as one analysis paragraph + one bullet list; only add sections when there are genuinely
  separate concerns.
- Call out anything reviewers must know: behavior changes, deferred work, follow-ups, manual test
  steps, and risks/caveats.
