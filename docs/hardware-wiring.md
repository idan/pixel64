# Hardware & wiring

## The interface: HUB75E

The Waveshare P3 64×64 speaks **HUB75E** over a 16-pin (2×8) IDC connector.

- It is a **1/32-scan** panel: the top 32 rows and bottom 32 rows are driven
  *simultaneously*. That's why there are two RGB triples — `R1/G1/B1` (top half)
  and `R2/G2/B2` (bottom half) — plus **5 address lines A–E** (2⁵ = 32 row-pairs).
  The 5th address line `E` is what makes it HUB75**E** (32-row panels only need A–D).
- The panel has **no frame memory**. It only emits light while data is actively
  being clocked out, so the firmware must refresh it continuously. Stop refreshing
  → it goes dark.
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

**Wire pin 12 → GPIO0 (the `D` address line). Do not tie it to ground.** Use the
*real* GND pads (pins 4 and 16) for ground.

Symptom if you get this wrong: the image appears **split or duplicated vertically**
(the top and bottom 32-row blocks mirror or interleave) — the signature of a bad
address line.

## Pin map: HUB75 → ESP32-C6

These GPIO assignments come from the `esp-hub75` author's tested C6 configuration,
adapted to pins that are actually broken out on the **DevKitM-1** (the original
example used GPIO10/GPIO11, which this board does **not** expose).

| HUB75 | Function | C6 GPIO | DevKit header |
|-------|----------|---------|---------------|
| R1 | top-half red | GPIO19 | J3 |
| G1 | top-half green | GPIO20 | J3 |
| B1 | top-half blue | GPIO21 | J3 |
| R2 | bottom-half red | GPIO22 | J3 |
| G2 | bottom-half green | GPIO23 | J3 |
| B2 | bottom-half blue | GPIO15 | J3 |
| A | addr0 | GPIO2 | J1 |
| B | addr1 | GPIO8 | J1 |
| C | addr2 | GPIO1 | J1 |
| **D** | addr3 (**pin 12, silk "GND"**) | GPIO0 | J1 |
| E | addr4 | GPIO3 | J1 |
| CLK | pixel clock | GPIO7 | J1 |
| LAT (STB) | row latch | GPIO6 | J1 |
| OE | output enable (active-low blank) | GPIO5 | J1 |
| GND | ground (pins 4 & 16) | any GND | — |

This keeps all 8 control/address lines on **J1** and all 6 RGB lines on **J3** — a
clean split for ribbon routing.

### Pin selection notes (DevKitM-1)

- Pins are flexible: the C6's PARL_IO routes through the GPIO matrix, so any free
  GPIO works. Edit the `Hub75Pins16` struct in `src/bin/main.rs` if you rearrange.
- **GPIO10 and GPIO11 are not broken out** on the DevKitM-1 — don't use them.
- **Avoid** GPIO16/GPIO17 (UART to the USB bridge), GPIO12/GPIO13 (USB-JTAG D-/D+),
  and GPIO9 (BOOT strapping) — using them breaks flashing or the serial monitor.
- GPIO8 and GPIO15 are strapping pins but are safe here, since they're only used as
  outputs *after* boot.
- Spare broken-out pins if you need them: **GPIO4, GPIO14, GPIO18**.

## Power

- The panel needs its **own 5 V supply, up to ~4 A** at full white. Feed it through
  the panel's dedicated power connector (the screw/JST terminals), **not** through
  the dev board.
- **Never** route the panel's LED current through the ESP32-C6 board.

## Common ground (required)

The logic signals swing 0–3.3 V referenced to the **MCU's** ground; the panel
decides 0/1 relative to **its** ground. Without a shared reference the levels are
meaningless.

- **Connect a HUB75 GND pin (4 or 16) → a GND pin on the C6 DevKitM-1.** One wire.
- This wire is a **signal reference**, not a power return — it carries only the tiny
  data-line return currents. The panel's amps return through its own 5 V supply's
  ground wiring.
- End state: **MCU GND ↔ HUB75 GND ↔ 5 V supply GND** all common. On the panel the
  HUB75 GND pins and the power-input GND are the same net, so the single HUB75↔MCU
  link ties everything together.

## Level shifting (recommended)

The C6 drives **3.3 V** logic; HUB75 panels expect **5 V** logic. Short jumpers often
appear to work but glitch (flicker, wrong colors) as the cable/panel warms up.

For anything beyond a bench test, buffer all 14 signal lines through a **74AHCT245**
(or 74AHCT125). The `AHCT` family reads 3.3 V as a valid logic high and outputs clean
5 V. Its ground joins the common ground above. This is the single most common cause of
"it mostly works but flickers."
