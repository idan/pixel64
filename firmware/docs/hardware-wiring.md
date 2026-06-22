# Hardware & wiring

The **pin assignments** (HUB75 → Pico GPIO, with the ESP32 migration table and dupont colors) live
in **[hub75-pico-wiring.md](hub75-pico-wiring.md)**. This doc covers the panel interface itself and
the electrical concerns — all of which are independent of the MCU.

## The interface: HUB75E

The Waveshare P3 64×64 speaks **HUB75E** over a 16-pin (2×8) IDC connector.

- It is a **1/32-scan** panel: the top 32 rows and bottom 32 rows are driven
  *simultaneously*. That's why there are two RGB triples — `R1/G1/B1` (top half)
  and `R2/G2/B2` (bottom half) — plus **5 address lines A–E** (2⁵ = 32 row-pairs).
  The 5th address line `E` is what makes it HUB75**E** (32-row panels only need A–D).
- The panel has **no frame memory**. It only emits light while data is actively
  being clocked out, so the firmware must refresh it continuously. Stop refreshing
  → it goes dark. (On the Pico this is done entirely by PIO+DMA — see
  [firmware.md](firmware.md).)
- There are **two identical headers**: one **INPUT**, one **OUTPUT** (for chaining
  panels). **Drive the INPUT side** — follow the arrow silk pointing away from the
  connector, or the `IN` marking.

## Connector layout (INPUT header, 2×8)

```
        ┌──────────┐
  R1 ─  │  1    2  │ ─ G1
  B1 ─  │  3    4  │ ─ GND
  R2 ─  │  5    6  │ ─ G2
  B2 ─  │  7    8  │ ─ E
   A ─  │  9   10  │ ─ B
   C ─  │ 11   12  │ ─ D     ← silkscreen mislabels this "GND" (see below)
 CLK ─  │ 13   14  │ ─ LAT
  OE ─  │ 15   16  │ ─ GND
        └──────────┘
```

## ⚠️ Silkscreen quirk: pin 12 ("GND") is actually D

On this panel the **pin-12 pad is silkscreened `GND`, but it is electrically the
`D` address line** (address bit 3). This is a common manufacturing carry-over: the
64×64 board reuses the 32×32 (1/16-scan) silkscreen, which didn't need a `D` line
in that position.

Proof it can't be ground: the address lines form a 5-bit counter (A=bit0 … E=bit4)
selecting one of 32 row-pairs. The panel exposes A, B, C, **and** E — and you can't
have bit 4 (E) without bit 3 (D). The only unaccounted pad, pin 12, is therefore D.

**Wire pin 12 → the D address line (GP12 on the Pico). Do not tie it to ground.** Use
the *real* GND pads (pins 4 and 16) for ground.

Symptom if you get this wrong: the image appears **split or duplicated vertically**
(the top and bottom 32-row blocks mirror or interleave) — the signature of a bad
address line. (Verified during bring-up with the `firstlight` bit-bang test.)

## Pin map

See **[hub75-pico-wiring.md](hub75-pico-wiring.md)** for the full HUB75 → Pico GPIO table. In short:
the 6 RGB data lines land on **GP0–GP5** and the 5 address lines on **GP9–GP13** (each group must be
*consecutive* — a constraint of the PIO `out` instruction); CLK/LAT/OE are on **GP6/GP7/GP8** (PIO
side-set, placed freely). All 14 signals sit on the Pico's left header. Avoid GP23/24/25/29 — those
are wired to the onboard CYW43 radio.

## Power

- The panel needs **5 V at up to ~4 A** at full white. Feed it through the panel's
  dedicated power connector (the screw/JST terminals), **not** through the dev board.
- **Never** route the panel's LED current through the MCU board. The MCU only taps a
  small branch off the same rail (see below); the amps go straight from the PSU to the
  panel's power lugs.

### One supply for the whole device (finished packaging)

Goal: **one power cable from the wall to the finished unit.** Both the panel and the
MCU run on 5 V, so you split a single 5 V rail:

```
            ┌──────────────► panel power terminals (fat red/black leads)
5 V PSU ────┤
            └──► Pico VSYS (pin 39)  +  GND
```

- **Pico 2 W:** feed the 5 V rail into **VSYS (pin 39)**, GND to any GND pin. VSYS accepts
  **1.8–5.5 V** and feeds the onboard buck-boost that generates 3.3 V for the RP2350 and the CYW43
  radio — so raw 5 V here is exactly its intended use.
  - The Pico has an onboard Schottky (D1) between VBUS and VSYS. If **only** the external 5 V is
    connected in normal operation (no USB), wire VSYS directly. If you want to plug in **USB for
    flashing/debug while the external supply is live**, either unplug the supply first, or add your
    own Schottky between the 5 V rail and VSYS (the datasheet's "two sources" arrangement) so the
    sources don't fight.

### Sizing & rail hygiene

- The panel's **4 A is its own worst case** (full white, full brightness), not
  panel-plus-everything. The MCU adds only ~50–150 mA, with brief higher peaks when the
  radio transmits — rounding error, but it eats into already-thin margin since full
  white sits near the edge of a 4 A supply.
- For a one-cable finished build, use **5 V / 5 A (or 6 A)** for clean headroom, and/or
  **cap brightness in firmware** so you never approach full-white max draw (you want a
  brightness cap anyway — full brightness is blinding and runs hot).
- **Prefer the headroom — the current rating is a ceiling, not a draw.** A bigger supply doesn't
  push more current or waste more as heat (losses track the *actual* load); it buys **lower
  operating temperature** (≈ longer cap life) and a **stiffer rail under transients** (panel
  row-switching + radio TX peaks), which directly helps brownout/flicker margins.
- Add a **bulk electrolytic (≥1000 µF) across the 5 V rail near the panel** to absorb the
  current spikes as rows switch. HUB75 panels are electrically noisy; radio TX peaks on a
  sagging shared rail are a classic cause of brownout resets.

## Common ground (required)

The logic signals swing 0–3.3 V referenced to the **Pico's** ground; the panel
decides 0/1 relative to **its** ground. Without a shared reference the levels are
meaningless.

- **Connect a HUB75 GND pin (4 or 16) → a GND pin on the Pico.** One wire.
- This wire is a **signal reference**, not a power return — it carries only the tiny
  data-line return currents. The panel's amps return through its own 5 V supply's
  ground wiring.
- End state: **Pico GND ↔ HUB75 GND ↔ 5 V supply GND** all common. On the panel the
  HUB75 GND pins and the power-input GND are the same net, so the single HUB75↔Pico
  link ties everything together. (If powering the Pico from the shared 5 V rail via VSYS, that
  ground is already common.)

## Level shifting (recommended)

The Pico drives **3.3 V** logic; HUB75 panels expect **5 V** logic. Short jumpers often
appear to work but glitch (flicker, wrong colors) as the cable/panel warms up.

For anything beyond a bench test, buffer all 14 signal lines through a **74AHCT245**
(or 74AHCT125). The `AHCT` family reads 3.3 V as a valid logic high and outputs clean
5 V. Its ground joins the common ground above. This is the single most common cause of
"it mostly works but flickers." Signal flow: **Pico GP → '245 input → '245 output (5 V) →
HUB75 pad.**
