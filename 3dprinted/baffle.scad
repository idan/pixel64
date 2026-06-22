// baffle.scad — grid baffle for the pixel64 64x64 HUB75 LED panel
//
// A grid of thin walls that sits between the LED panel and a diffuser sheet,
// giving each of the 4096 pixels its own optical cell so light can't bleed
// sideways into its neighbours.
//
// Every cell is identical (pitch - wall). The outer wall extends OUTWARD past
// the panel edge rather than inward, so the border pixels stay full size; the
// footprint is therefore slightly larger than the 192mm panel (a future frame
// enclosure can capture this lip).
//
// Print FLAT (this footprint flat on the bed, walls growing up in +Z).
// No supports needed — it's an open grid, nothing overhangs.
// Intended material: black matte PLA.

/* ---------------- parameters ---------------- */
pitch  = 3;     // LED pixel pitch (mm)  -> cell size
n      = 64;    // pixels per side
wall   = 0.42;  // internal wall thickness (mm) == the single printed line width.
                // Under Arachne (Bambu Studio default) each wall is one perimeter
                // whose width is set to fill this value, so this number *is* the
                // printed line. 0.40 (= nozzle) is the floor; 0.42 keeps one clean
                // line with a hair of margin. Set the slicer's wall line width to
                // match, and CALIBRATE FLOW or it prints fat regardless.
height = 3;     // wall / baffle height in Z (mm). Tuned against the real panel +
                // diffuser: 5mm over-isolated and blocked too much off-axis light;
                // ~2.5-3mm gives clean per-pixel cells. Lower height also prints
                // easier (7:1 aspect at 0.42 wall vs 12:1 at 5mm).
border = 0.8;   // outer wall thickness (mm). Extends OUTWARD past the panel so
                // it never eats into the edge pixels. Set == wall for a fully
                // uniform-thickness grid.

span = n * pitch;            // 192 mm — the actual panel/pixel area
ext  = border - wall / 2;    // how far the outer wall sticks out past the panel
echo(span = span, footprint = span + 2 * ext, cells = n * n);

/* ---------------- geometry ------------------ */

// interior walls, centered on the inter-pixel boundaries (x,y = 3, 6, ... 189)
module internal_walls() {
    for (k = [1 : n - 1]) {
        // walls running along Y (constant x)
        translate([k * pitch - wall / 2, 0, 0])
            cube([wall, span, height]);
        // walls running along X (constant y)
        translate([0, k * pitch - wall / 2, 0])
            cube([span, wall, height]);
    }
}

// outer frame: inner faces sit flush with the edge pixels' openings, body
// extends outward past the panel edge so border pixels are full size.
module frame() {
    difference() {
        translate([-ext, -ext, 0])
            cube([span + 2 * ext, span + 2 * ext, height]);
        translate([wall / 2, wall / 2, -1])
            cube([span - wall, span - wall, height + 2]);
    }
}

union() {
    frame();
    internal_walls();
}
