# pixel64

An internet-connected **64×64 HUB75 LED pixel display**.

## Layout (monorepo)

- **[`firmware/`](firmware/)** — the device: ESP32-C6, Rust (esp-hal 1.1 + Embassy, `no_std`).
  Drives the Waveshare P3 64×64 HUB75 panel and handles first-run Wi-Fi onboarding over BLE.
  Docs live in [`firmware/docs/`](firmware/docs/).
- **`web/`** — *(planned)* cloud backend + web UI: Svelte 5 / SvelteKit on Cloudflare, tooled with
  Bun.

## Quick start

**Firmware** (run from `firmware/`):

```sh
cd firmware
cargo run        # build, flash over USB, open the serial monitor
```

On first boot the panel shows **SETUP**; provision Wi-Fi from **Chrome on Android / Windows / Linux**
at <https://www.improv-wifi.com/ble/> → pick `pixel64`. (macOS Chrome has a known BLE caveat — see
[firmware/docs/gotchas.md](firmware/docs/gotchas.md).) Hold **BOOT ~3 s** to factory-reset.

**Web** — not scaffolded yet.

## Docs

- [CLAUDE.md](CLAUDE.md) — project orientation (state, constraints, where things are).
- [firmware/docs/](firmware/docs/) — wiring, firmware architecture, performance, Wi-Fi onboarding,
  and a **gotchas / debugging** doc worth reading before diving in.
