# pixel64 documentation

Driving a **Waveshare RGB-Matrix-P3 64×64** LED panel from an **ESP32-C6-DevKitM-1**
in Rust (esp-hal 1.1 + Embassy), using the [`esp-hub75`](https://github.com/liebman/esp-hub75)
driver with DMA over the C6's **PARL_IO** peripheral and drawing via
[`embedded-graphics`](https://docs.rs/embedded-graphics).

## Contents

- [hardware-wiring.md](hardware-wiring.md) — the HUB75E interface, the full pin map,
  power, level shifting, and the silkscreen quirk on this panel.
- [firmware.md](firmware.md) — crate choices, the double-buffered render architecture,
  and how to build & flash.
- [performance.md](performance.md) — measured refresh/draw rates, the math behind them,
  and the tuning knobs.

## Status

- ✅ Panel wired and confirmed working (first-light test pattern).
- ✅ Double-buffered animation running: ~520 Hz panel refresh, ~52 Hz animation.

## Hardware

| | |
|---|---|
| MCU | ESP32-C6-DevKitM-1 v1.0 (RISC-V, 3.3 V logic) |
| Panel | Waveshare RGB-Matrix-P3-64x64 (HUB75E, 1/32 scan) |
| Panel power | 5 V, up to ~4 A, via the panel's own power connector |
| Framework | esp-hal `~1.1`, esp-rtos `0.3` (Embassy) |
