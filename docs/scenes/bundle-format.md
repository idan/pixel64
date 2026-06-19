# Bundle format

The **bundle** is the unit of delivery from `web/` to the device: a scene definition plus everything
it needs to render — assets and shader bytecode. It's the contract between cloud and firmware. (Live
input *values* travel separately and lighter — see [inputs-and-binding.md](inputs-and-binding.md).)

## Contents

A bundle is a container holding:

1. **Manifest** — the scene definition: metadata, target fps + default `eval_res`, the ordered
   **layers** (each with its common envelope + kind-specific props + bindings, per
   [layers-and-compositing.md](layers-and-compositing.md)), the **inputs** table
   ([inputs-and-binding.md](inputs-and-binding.md)), and per-shader-layer uniform mappings.
2. **Assets** — images, animation frame sequences, fonts. Content-addressed by hash, referenced from
   the manifest by id. Deduplicated across bundles.
3. **Shader bytecode** — one blob per shader layer, with its `vm_version` and binding metadata
   ([shader-vm.md](shader-vm.md)).

## Encoding: authored as JSON, shipped as binary

- **DECISION: the device never parses JSON.** Authoring/debugging uses a JSON manifest (human-readable,
  diff-able, what the editor emits). The backend **compiles** that to a **compact binary container**
  the firmware reads directly — fixed-layout headers + a TLV section table — so the device needs no
  JSON parser and minimal RAM to load a scene.
- **OPEN:** whether the binary container is a bespoke format or **CBOR**. Bespoke = smallest + zero-copy
  friendly; CBOR = off-the-shelf `no_std` decoders, less code to own. Leaning bespoke for the
  fixed/hot parts (header, layer table, bytecode) and tolerating a small CBOR-ish blob for rarely-read
  metadata — to be settled at implementation.

## Asset encoding

- **DECISION (primary codec): palette-indexed + RLE.** Most content is pixel art with few colors;
  1 byte/pixel into a ≤256-entry RGBA palette, run-length encoded, is dramatically smaller than
  RGB565 and carries alpha for free. 64×64 worst case = 4 KB + palette; typical art is far less.
- **Secondary codec: RGBA8888**, for full-color/photographic frames that don't palettize well.
- **Animations:** a sequence of frames in either codec, plus timing
  ([layers-and-compositing.md](layers-and-compositing.md)). **OPEN:** inter-frame delta encoding
  (only changed runs per frame) — high payoff for typical loops; defer to a v1.1 if it complicates the
  decoder.
- **Fonts:** a small set of **built-in bitmap fonts in firmware** (so most text scenes ship no font
  asset); custom fonts can travel as an asset later. **OPEN:** which built-in fonts/sizes.
- Assets are stored in flash and decoded to premultiplied RGBA on demand during compositing
  ([device-runtime.md](device-runtime.md)).

## Content addressing & dedup

Every asset (and the bytecode) is identified by a **hash of its bytes**. The manifest references
assets by hash, so:

- An unchanged background shared by ten scenes is stored once.
- Re-pushing an edited scene re-sends only the assets whose hashes changed.
- The device can answer "do I already have hash X?" before a transfer.

## Versioning

Three independent version axes, all carried in the bundle header:

| Axis | Meaning | Failure mode handled |
|---|---|---|
| `schema_version` | bundle container + manifest layout | firmware too old for a new bundle shape |
| `vm_version` | shader opcode-set revision (per bytecode blob) | bytecode uses opcodes this firmware lacks |
| asset codec version | per-asset encoding | new codec on old firmware |

The device **advertises** the `schema_version` range and `vm_version`s it supports (and its
[resource limits](shader-vm.md)) so the backend can compile/target correctly and never push something
that won't run. Unknown major versions are rejected with a logged reason, not a crash.

## Delivery

- Bundles are produced by the `web/` backend (Cloudflare) and fetched by the device over Wi-Fi
  (`cyw43` + `embassy-net`).
- The device holds a persistent **WebSocket control channel** to a Cloudflare Durable Object
  ([transport.md](transport.md)) over which it learns "scene/playlist now references bundle hash X."
  It then **fetches missing assets/bytecode by hash over HTTPS** (the bulk channel) and caches them in
  flash.
- **Flash budget:** the Pico 2 W has 4 MB QSPI flash, shared by firmware + scene cache. The cache is
  content-addressed with **LRU eviction**. **OPEN:** firmware/cache partition split and target
  per-bundle ceiling (order of a few hundred KB).

## Open questions

1. **Binary container: bespoke vs CBOR** (see above).
2. **Inter-frame animation delta encoding** in v1 or v1.1.
3. **Built-in font set** (faces/sizes) and whether custom-font assets are v1.
4. **Flash partitioning** and per-bundle / total-cache size limits.
