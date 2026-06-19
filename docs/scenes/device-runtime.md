# Device runtime

The firmware side of scenes: how bundles are stored, scheduled, fed live inputs, rendered, and clocked
out to the panel — and how that maps onto the RP2350's two cores and SRAM. Builds on the locked
platform decisions in [README.md](README.md).

## Components

```
        ┌──────────────┐   live values   ┌───────────────┐
  Wi-Fi │ control chan │ ───────────────►│ input manager │──┐ uniforms / strings
 (cyw43)│  + fetch     │                 └───────────────┘  │
        └──────┬───────┘                                    ▼
               │ bundles (by hash)              ┌──────────────────────┐
               ▼                                │  compositor          │
        ┌──────────────┐   active scene   ┌─────┤   ├─ image/anim decode│
        │ scene store  │ ───────────────► │ sched│   ├─ text raster     │
        │ (flash, LRU) │                  │ uler │   └─ shader VM        │
        └──────────────┘                  └─────┘          │ f32 RGBA
                                                            ▼
                                              ┌──────────────────────────┐
                                              │ panel driver (PIO+DMA)    │
                                              │ BCM bitplanes, dbl-buffer │
                                              └──────────────────────────┘
```

- **Scene store** — persists bundles/assets in flash, content-addressed with LRU eviction
  ([bundle-format.md](bundle-format.md)); remembers the current scene/playlist across reboot.
- **Scheduler / playlist** — decides what's showing: a playlist is an ordered/looping list of scene
  refs with per-item durations, plus optional time-of-day schedules. **OPEN:** scheduling feature
  scope for v1 (single scene + simple rotation is the floor).
- **Input manager** — holds current values for all `live`/`device` inputs, applies easing, latches a
  per-frame snapshot, and writes uniform tables / formats strings
  ([inputs-and-binding.md](inputs-and-binding.md)).
- **Compositor** — per frame, walks layers and blends to the f32 RGBA accumulator
  ([layers-and-compositing.md](layers-and-compositing.md)); quantizes (with gamma LUT) into the panel
  back buffer.
- **Panel driver** — PIO+DMA HUB75, BCM bitplanes, double-buffered. The **swappable backend below the
  framebuffer seam**; replacing the ESP32-C6 `esp-hub75`/PARL_IO driver is contained here.

## Two-core split

- **DECISION: core 1 is dedicated to compositing + the shader VM** (the heavy per-pixel work). **Core
  0 runs everything else** — panel driver orchestration, Wi-Fi/`cyw43`, control channel, scheduler,
  input manager.
- **Why this is safe (the flicker fix):** the panel refresh is **PIO+DMA** — hardware clocks the
  bitplanes out without per-bit CPU. This is the whole reason for the RP2350 migration: on the C6,
  radio activity stalled the bit-banged refresh and caused flicker. Here, even sharing core 0 between
  radio and the (DMA-driven) panel won't flicker, and core 1's compute is fully decoupled.
- **Frame handoff:** double-buffered f32→panel framebuffers with single-producer/single-consumer
  swap. Core 1 fills the back buffer; core 0's driver consumes the front. Coordinated with
  `embassy-sync` primitives. A slow compute frame on core 1 simply lowers animation fps — core 0 keeps
  refreshing the last good buffer, so **the panel never freezes**.
- **Note (not the C6 constraint):** the `esp_hal::Async` `!Send` limitation was ESP-specific. On
  `embassy-rp`, cross-core sharing has its own `Send`/sync requirements; the buffer-ownership protocol
  is designed around those, not carried over from the C6.

## Render loop & timing

- Panel refresh runs continuously at hundreds of Hz (BCM) regardless of scene fps.
- The compositor produces a new frame at the **scene's target fps** into the back buffer; the scene
  clock `t`/`frame` advance per produced frame. Animation fps and panel refresh are **decoupled**.
- **Transitions** (scene→scene crossfade) = compositing the outgoing and incoming scenes for a short
  window. **OPEN / deferred:** likely v1.1; v1 may hard-cut.

## Memory budget (520 KB SRAM)

Rough envelope; exact panel-buffer sizing waits on the driver:

| Consumer | Estimate |
|---|---|
| Compositor accumulator (f32 RGBA, 64×64) | 64 KB |
| Per-layer shader eval buffer (f32 RGBA, ≤64×64) | ≤64 KB |
| Panel framebuffers (BCM bitplanes, double-buffered) | **OPEN** — depends on bit depth/scan; order of single-digit KB ×2 |
| Asset decode scratch | small, transient |
| Wi-Fi / net buffers, task stacks, LUTs | moderate |

Comfortably within 520 KB. **DECISION:** assets live in **flash**, decoded on demand into small
scratch rather than held in RAM, so flash size (not SRAM) bounds the content library.

## Fault handling

Never brick the display:

- Bad/oversized bundle, unsupported `vm_version`, missing asset → reject with a logged reason; keep
  showing the previous scene.
- VM gas overrun on a pixel → emit current output and move on
  ([shader-vm.md](shader-vm.md)); a pathological shader degrades fps, doesn't hang.
- Decode error on a layer → skip that layer (or show an error glyph), composite the rest.

## Open questions

1. ~~Control-channel transport~~ — **RESOLVED:** WebSocket → Durable Object (push) + HTTPS-by-hash
   (bulk), on core 0. See [transport.md](transport.md).
2. **Scheduler scope for v1** — single scene + timed rotation, or schedules/conditions too?
3. **Transitions** in v1 or v1.1.
4. **Exact panel-buffer sizing** — resolved when the PIO+DMA driver is built (color depth × scan ×
   double-buffer).
