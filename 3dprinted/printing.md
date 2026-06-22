# Printing the baffle

Notes for printing `baffle.scad` / `baffle-mini.scad` — a 64×64 grid of thin
single-line walls (0.42 mm wide × 5 mm tall) printed flat, walls growing up in
+Z. The whole part is single-line walls at a ~12:1 aspect ratio, which makes it
fragile and adhesion-sensitive. **Always dial in on `baffle-mini` first** — it's
a 10×10 swatch that prints in minutes.

Slicer: **Bambu Studio**, 0.4 mm nozzle, black matte PLA, Arachne wall generator
(the default).

## Why the wall is 0.42, not 0.45

Under Arachne, each wall prints as a single perimeter whose width is set to fill
the model thickness — so **the model `wall` value *is* the printed line width**.
0.40 mm (= nozzle) is the floor; 0.42 keeps one clean line with a hair of margin
and is more robust at 5 mm tall. Going to 0.40 is ~11% less material in the light
path but noticeably more fragile and harder to keep stuck — only worth it on a
swatch once everything else is dialed.

The old "0.45 to dodge thin-wall detection" trick is obsolete with Arachne.

## Keep the 0.8 mm border

The outer frame is two perimeters on purpose:

- It's the rigid handle that holds the otherwise-floppy grid together and gives
  the lip a future enclosure can capture.
- It extends **outward** past the panel (`ext = border - wall/2`), so it never
  overlaps an active pixel — thinning it buys no optical gain.
- A wide continuous contour is the strongest-gripping feature on the plate and
  anchors the connected grid. Thinning it would *hurt* adhesion.

## The settings that matter

**Calibrate first (per filament), or nothing below holds:**

1. **Flow ratio** — single-wall method: print `baffle-mini`, measure a wall,
   set `flow = current × (0.42 / measured)`. Without this every wall prints fat
   regardless of the model value.
2. **Flow Dynamics (Pressure Advance)** — separate from flow ratio. A grid layer
   has thousands of line starts/stops; bad PA leaves a blob at each one. This is
   the big lever for a clean first layer (see below).

**Line width**
- Wall / outer-wall line width → **0.42 mm** (match the model).
- First-layer line width → **0.44–0.45 mm**. (Do *not* widen to 0.5 — see the
  first-layer section; the extra material curls into hairs on single-line walls.)

**Adhesion**
- **Brim: outer, ~5 mm**, gap ~0.1 mm. The grid is one connected body, so
  anchoring the frame holds everything down. Single biggest adhesion win.
- First-layer speed **20–30 mm/s**; outer-wall speed ~50–80 mm/s (low speed on a
  5 mm single wall = less wobble = thinner-looking, cleaner walls).
- Clean the plate with **dish soap + water** (IPA just smears the oils). Textured
  PEI grips thin walls better than smooth.
- Bed **60–65 °C**, first-layer fan off.
- First-layer height 0.2 mm; tune Z-offset so the line is *slightly* squished —
  under-squish is why thin walls don't stick.
- **Elephant-foot compensation 0.1–0.15 mm** to trim the squished-wide base back
  toward 0.42 (don't exceed ~0.2 or single-line walls under-anchor).

**Wall quality at height**
- Optional: enable "slow down for layer cooling" / min layer time ~8–10 s so each
  thin layer sets before the next.

## Clean first layer (the rats'-nest fix)

If layer 1 has hairy clumps but the rest is clean, it's first-layer-specific.
In order of impact:

1. **Don't over-extrude layer 1.** Keep first-layer line width ~0.44–0.45 and
   first-layer flow ~0.95–1.0. Excess material on a single-line wall has nowhere
   to go and curls into hairs. (This was the main culprit.)
2. **Calibrate Flow Dynamics / Pressure Advance** — kills the per-segment blobs
   that travels then smear into nests.
3. **Z-hop on travel (0.2 mm) + "avoid crossing walls/perimeters."** Stops the
   low layer-1 nozzle from ploughing through lines it already laid as it crosses
   open cells.
4. **Retraction + short wipe** — reduces ooze/stringing across the many travels.
5. **Skirt, 2–3 loops** — primes a clean nozzle so a start blob lands in the
   skirt, not the grid.
6. **Filter out tiny gaps / minimize gap fill** — stops over-extruded dabs at the
   grid intersections.
7. Still hairy? Drop nozzle temp 5–10 °C.

Items 1 + 2 alone produced a perfect `baffle-mini`.

## Reading first-layer squish (Z-offset)

Z-offset / squish is a **global, machine-level** setting — every object shares one
first layer, so it can't be per-part. The reliable way to set it is to **watch
layer 1 and live-tune on the printer:** start the print, and during the first
layer tap **Tune → Z offset** (some firmware: "Flow & Z offset"). Adjust in
**0.01–0.02 mm steps; negative = nozzle closer = more squish.** The value
persists for future prints. (Bambu Studio also exposes a global Z offset on the
**Device** tab, but tuning live lets you *see* the result.) The slicer-side
alternative is first-layer flow — but that reintroduces hairs here, so prefer Z.

**What to look at:** judge on the **brim and border** — the solid side-by-side
line patches. The cells are isolated single-line walls and are hard to read.

- **Too high / under-squished** (this part's torn-cell failure mode): lines stay
  rounded like laid string; **gaps between adjacent brim lines** show bare plate;
  poor grip, some lines drag. → nudge Z **more negative**.
- **Just right:** adjacent lines **merge with no gap**, tops slightly flattened,
  surface a **uniform matte sheet** — no bare plate, no ridges; lines stick on
  contact.
- **Too low / over-squished:** lines very wide/flat, **ridges or ripples** of
  pushed-up plastic, scraped/translucent look, plastic piling at the nozzle. →
  nudge Z **more positive**.

Nudge a couple hundredths, watch several lines, reassess — the change isn't
instant. For this part, bias **slightly firmer** than a cosmetic-perfect squish;
the extra anchor helps the single-line walls and elephant-foot comp trims the
fatter base back anyway.

## Full 64×64 / large-part notes

The full part is ~193 mm square — it spans nearly the whole plate, which exposes
problems the 10×10 `baffle-mini` never touches. If the mini is perfect but the
full print has a **small number of torn/missing first-layer cells**, it's an
area-uniformity problem, not a settings problem. In order of impact:

1. **Run a fresh full mesh bed level for this print.** In the send/print dialog,
   tick **Bed Leveling** (alongside Flow Dynamics / Timelapse) so it re-probes the
   whole plate before layer 1. Or run Settings → Calibration → Auto-leveling on
   the printer. Over 193 mm, slight dishing/tilt means some regions print too high
   → those cells barely stick → the next travel drags them loose.
2. **Watch the first layer and live-tune Z-offset** (see above) — bias firmer.
3. **Clean the *entire* plate** (corners and edges, not just the center where the
   mini lived) with dish soap + water. Check the plate isn't warped; swap to a
   flatter one if you have it.
4. **Z-hop on travel (0.2 mm) + "avoid crossing walls."** Easy to skip when the
   mini prints fine without them, but at 4096 cells the travel count explodes and
   one marginally-stuck cell gets ripped out. Probably the biggest fix after
   leveling.
   - **Z-hop is a *Printer* setting, not a Process setting** — that's why it's
     hard to find. **Printer settings** (printer/gear icon) → **Extruder** →
     **"Z hop when retract"** (height, e.g. 0.2) and **"Z hop type"** (Auto/Normal
     is fine). Retraction length/speed and "wipe while retracting" are in the same
     Extruder section. Editing a printer setting prompts you to save a custom
     printer preset — do it so the value sticks.
   - **"Avoid crossing wall"** *is* a Process setting (Quality tab).
   - Two prerequisites: set the settings panel to **Advanced/Expert mode** (Simple
     mode hides these), and use the **search box** (magnifying glass at the top of
     the panel) — type `z hop` / `avoid crossing` to jump straight to the field
     instead of hunting tabs, which move between releases.
5. **Bigger brim (~8 mm)** — large flat PLA parts lift at the corners first.
6. **Slow the first layer further** (15–20 mm/s) for the big part.
7. **Last resort if the plate won't level flat:** a raft (wastes filament, peel it
   off). Exhaust 1–6 first.

**Wall loops = 2 is correct — leave it.** It does nothing to the interior grid (a
0.42 mm wall is one line wide, so Arachne lays one loop regardless), but it's what
makes the 0.8 mm border print as a solid double-line frame. Set it to 1 and the
border prints as a single line with a ~0.4 mm gap down its middle, weakening the
one rigid part that holds the grid together.

Cancelling early on torn cells is right — a torn cell sheds debris the nozzle
drags into its neighbors, so it cascades rather than self-heals.
