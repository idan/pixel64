# HUB75 → Pico 2 W wiring (+ ESP32-C6 migration)

Pin map for driving the Waveshare P3 64×64 HUB75E panel from the **Raspberry Pi Pico 2 W**, with
the **old ESP32-C6 GPIO** for each signal so the existing (unlabeled) dupont wires can be re-plugged
1:1 — each wire carries one HUB75 signal, so just move it from the old ESP pin to the new Pico pin.

Fill in the **Dupont color** column as you trace/label each wire.

All 14 signals land on **GP0–GP13** (the Pico's entire left header, phys. pins 1–17), chosen to
satisfy the PIO driver's grouping: **R1–B2 must be 6 consecutive GPIOs** and **A–E must be 5
consecutive GPIOs**; CLK/LAT/OE are standalone (PIO side-set, placed freely). None collide with the
cyw43 radio (GP23/24/25/29).

> Dupont colors: start from the edge with the brown wire. "Brown2" is the brown wire that is further from the start.

| Pico GP | phys. pin | HUB75 signal | HUB75 pad | old ESP32-C6 | Dupont color |
|---------|-----------|--------------|-----------|--------------|--------------|
| GP0  | 1  | R1 (top red)      | 1      | GPIO19 | Brown1 |
| GP1  | 2  | G1 (top green)    | 2      | GPIO20 | Red1 |
| GP2  | 4  | B1 (top blue)     | 3      | GPIO21 | Orange1 |
| GP3  | 5  | R2 (bottom red)   | 5      | GPIO22 | Green1 |
| GP4  | 6  | G2 (bottom green) | 6      | GPIO23 | Blue1 |
| GP5  | 7  | B2 (bottom blue)  | 7      | GPIO15 | Purple |
| GP6  | 9  | CLK (clock)       | 13     | GPIO7  | Orange2 |
| GP7  | 10 | LAT (latch)       | 14     | GPIO6  | Yellow2 |
| GP8  | 11 | OE (blank)        | 15     | GPIO5  | Green2 |
| GP9  | 12 | A (addr0)         | 9      | GPIO2  | White |
| GP10 | 14 | B (addr1)         | 10     | GPIO14 | Black |
| GP11 | 15 | C (addr2)         | 11     | GPIO1  | Brown2 |
| GP12 | 16 | **D (addr3)**     | **12** ⚠️ | GPIO0  | Red2 |
| GP13 | 17 | E (addr4)         | 8      | GPIO3  | Grey |
| GND  | 3 / 8 / 13 / 18 / 23 / 28 / 38 | GND | 4 & 16 | any GND | Yellow1 |

## Notes (carried over from the ESP build — same panel)

- ⚠️ **HUB75 pad 12 is silkscreened "GND" but is electrically the D address line.** Wire it to
  **GP12**, not ground. Symptom if wrong: the image is split/duplicated vertically. (Real grounds are
  pads 4 & 16.) See [hardware-wiring.md](hardware-wiring.md).
- **Level-shift all 14 lines through a 74AHCT245** (3.3 V Pico → 5 V panel): Pico GP → '245 input →
  '245 output (5 V) → HUB75 pad. If your wires already run through the '245, you're only moving the
  MCU-side (3.3 V input) ends — the panel side stays put.
- **Power & common ground unchanged:** feed the panel's own 5 V connector (up to ~4 A), 5 V into the
  Pico's **VSYS** (pin 39), common ground, bulk cap near the panel. See
  [hardware-wiring.md](hardware-wiring.md).
- The ESP's GPIO8-onboard-LED dodge (B on GPIO14) is gone — on the Pico the onboard LED is on the
  cyw43 chip, so the address lines are a clean contiguous A–E on GP9–GP13.

## Re-plug workflow

Going pin-by-pin off the ESP, the same table sorted by **old ESP GPIO**: GPIO0→GP12, GPIO1→GP11,
GPIO2→GP9, GPIO3→GP13, GPIO5→GP8, GPIO6→GP7, GPIO7→GP6, GPIO14→GP10, GPIO15→GP5, GPIO19→GP0,
GPIO20→GP1, GPIO21→GP2, GPIO22→GP3, GPIO23→GP4.
