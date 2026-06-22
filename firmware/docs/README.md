# pixel64 documentation

Driving a **Waveshare RGB-Matrix-P3 64×64** LED panel from a **Raspberry Pi Pico 2 W (RP2350)** in
Rust (**embassy-rp**, no_std): a custom **HUB75 PIO+DMA** display driver, **Improv-over-BLE** Wi-Fi
onboarding (**cyw43** + **trouble-host**), and credential persistence to flash. Ported from an
ESP32-C6 (the original tree is preserved at git tag `esp32-final`).

## Contents

- **[pico-port.md](pico-port.md)** — the ESP32 → Pico 2 W port: the verified dependency set, the
  dependency-compatibility constraints (bt-hci / embassy-sync), the hard-won findings (cyw43 BLE
  byte-1 corruption, the macOS fix), and milestones. **The most important doc — read before touching
  deps or BLE.**
- [hardware-wiring.md](hardware-wiring.md) — the HUB75E interface, power, level shifting, and the
  silkscreen quirk on this panel.
- [hub75-pico-wiring.md](hub75-pico-wiring.md) — the Pico pin map (+ the ESP32-C6 migration table and
  recorded dupont colors).
- [firmware.md](firmware.md) — crate stack, the PIO+DMA display architecture, and how to build & flash.
- [performance.md](performance.md) — refresh-rate math and the tuning knobs.
- [wifi-onboarding.md](wifi-onboarding.md) — first-run Wi-Fi provisioning via Improv over BLE: browser
  support (incl. the macOS resolution), boot/persistence behaviour, and architecture.
- **[gotchas.md](gotchas.md)** — subtleties, dependency landmines, and the BLE debugging playbook.
  **Read before debugging display or BLE.**

## Status

- ✅ Panel: custom **PIO+DMA HUB75 driver**, double-buffered, BCM color — **no flicker** (refresh is
  hardware-timed, fully decoupled from the radio).
- ✅ Wi-Fi onboarding over BLE (Improv): provision from Chrome (**Android + macOS** verified), creds
  persist, IP shown on the panel.
- ✅ Boot state machine: stored creds → reconnect; otherwise Improv setup. Auto-recovers if a stored
  network fails; bad creds fail fast (length-validated + join timeout), never hang.
- ✅ Factory reset: hold **BOOTSEL ~3 s** while running → wipe creds, reboot to setup.
- ⏳ Open (polish): BCM color-depth tuning (smooth gradients); building out the `web/` app
  (scaffolded; UI/backend TBD).

## Hardware

| | |
|---|---|
| MCU | Raspberry Pi Pico 2 W (RP2350, ARM Cortex-M33, 3.3 V logic) |
| Panel | Waveshare RGB-Matrix-P3-64x64 (HUB75E, 1/32 scan) |
| Panel power | 5 V, up to ~4 A, via the panel's own power connector |
| Level shifting | 74AHCT245 (3.3 V → 5 V) on all 14 signals |
| Framework | embassy-rp `0.10` (Embassy) |
