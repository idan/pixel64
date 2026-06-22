#!/usr/bin/env bash
# render.sh — regenerate the STLs from the .scad sources via OpenSCAD.
#
# STLs are build artifacts (gitignored). Run this after editing geometry:
#   ./render.sh            # render every *.scad in this dir
#   ./render.sh baffle     # render just baffle.scad -> baffle.stl
set -euo pipefail

cd "$(dirname "$0")"

# Find the OpenSCAD CLI: prefer PATH, fall back to the macOS app bundle.
if command -v openscad >/dev/null 2>&1; then
    OPENSCAD=openscad
elif [ -x /Applications/OpenSCAD.app/Contents/MacOS/OpenSCAD ]; then
    OPENSCAD=/Applications/OpenSCAD.app/Contents/MacOS/OpenSCAD
else
    echo "error: OpenSCAD not found on PATH or at /Applications/OpenSCAD.app" >&2
    echo "install from https://openscad.org or 'brew install --cask openscad'" >&2
    exit 1
fi

# Build the list of sources: an argument (with or without .scad) or all of them.
if [ "$#" -gt 0 ]; then
    sources=()
    for arg in "$@"; do
        sources+=("${arg%.scad}.scad")
    done
else
    sources=(*.scad)
fi

for scad in "${sources[@]}"; do
    stl="${scad%.scad}.stl"
    echo "rendering $scad -> $stl"
    "$OPENSCAD" -o "$stl" "$scad"
done

echo "done."
