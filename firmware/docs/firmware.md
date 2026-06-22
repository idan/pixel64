# Firmware

Rust, `no_std`, on **embassy-rp** for the RP2350 (Pico 2 W). Target
`thumbv8m.main-none-eabihf` (ARM Cortex-M33). For the full crate-version set and the
dependency-compatibility constraints, see **[pico-port.md](pico-port.md)** — that's the source of
truth; the table below is an orientation.

## Crate stack

| Crate | Version | Role |
|-------|---------|------|
| `embassy-rp` | `0.10` | HAL for the RP2350 (`rp235xa`, `time-driver`, `binary-info`) + executor |
| `cyw43` / `cyw43-pio` | `0.7` / `0.10` | CYW43439 Wi-Fi **and** BLE, over PIO-emulated SPI |
| `trouble-host` | `0.6` | BLE GATT host for the Improv service (pinned to 0.6 — bt-hci 0.8 to match cyw43) |
| `embassy-net` | `0.9` | DHCP / IP stack over the cyw43 net device |
| `sequential-storage` + `embassy-embedded-hal` | `7` / `0.6` | credentials in flash (CRC + power-fail safe) |
| `pio` / `pio-proc` | `0.3` | assemble the HUB75 PIO programs (`pio_asm!`) |
| `embedded-graphics` | `0.8` | drawing primitives, fonts, text → the HUB75 `DrawTarget` |
| `embassy-usb` / `embassy-usb-logger` | `0.6` | `log` over USB-CDC serial (the no-probe dev loop) |
| `embassy-sync` | `0.7` **+** `0.8` | split: `0.7` for trouble's gatt macros, `0.8` for the rest (see pico-port.md) |
| `heapless` | `0.9` | `String`/`Vec` for RPC payloads + on-panel text |

There is **no heap** — the whole stack is static (no `esp-alloc`/`embedded-alloc`).

## The HUB75 display: PIO + DMA, zero CPU

The panel holds no image, so pixels must be clocked out *continuously*. On the RP2350 this is done
entirely in hardware by the **PIO** (Programmable I/O) blocks + **DMA** — the CPU never touches the
refresh, which is why there's **no flicker** even while the radio is busy (the exact failure mode of
the ESP build, where a single core had to interleave refresh with the radio).

`src/hub75.rs` (ported from [kjagiello/hub75-pio](https://github.com/kjagiello/hub75-pio-rs)) runs
**three PIO state machines on PIO1**, handshaking over PIO IRQs:

- **Data SM** — shifts the 6 RGB bits per pixel out to GP0–GP5, toggling CLK (side-set). One
  bit-plane of one row per pass.
- **Row SM** — drives the A–E address lines + LAT (side-set), advancing row and bit-plane.
- **OE SM** — times the **binary-code-modulation (BCM)** display window per bit-plane via OE.

A **self-chaining 4-channel DMA loop** (DMA_CH2–CH5, programmed via `embassy_rp::pac` since the safe
DMA API doesn't expose channel chaining) feeds the framebuffer to the data SM and the BCM dwell
weights to the OE SM, **forever**, reloading from a pointer so flipping that pointer swaps buffers.

### Color: binary-code modulation

Color depth is `B` bit-planes (currently **8**). Each plane `i` is displayed for `2^i − 1` ticks, so
the planes sum to a weighted intensity. `set_pixel` packs a pixel's per-plane bits into the
framebuffer (`XXBGRBGR` per byte, top half in bits 0–2, bottom half in 3–5). `PLANES` (`B`) and the
clock dividers trade color depth against refresh — see [performance.md](performance.md).

### Double-buffering + drawing

`Display` implements `embedded_graphics::DrawTarget<Color = Rgb888>`, so any embedded-graphics
content draws straight in. Drawing goes into the **inactive** buffer; `commit()` flips it live and
zeroes the next inactive buffer. `src/display.rs` runs one **draw task** that renders the current
`Screen` and commits at ~30 fps — there's no refresh task (the DMA does that in hardware).

> Orientation: the driver carries hub75-pio's un-mirror convention, so the image lands 180° from the
> draw origin. The panel is square/free-orientation — just mount it to suit.

## Wi-Fi onboarding + persistence

See **[wifi-onboarding.md](wifi-onboarding.md)**. In brief: `src/improv.rs` runs the Improv GATT
service over `trouble-host` on cyw43's BLE controller; `src/net.rs` joins Wi-Fi (`control.join` +
DHCP); `src/storage.rs` persists credentials to a reserved region at the top of flash. `main.rs` is
the boot state machine: stored creds → reconnect, else Improv setup.

## Build & flash (no debug probe)

The repo is preconfigured (`.cargo/config.toml` sets the target + a `picotool` runner; `memory.x`
and `build.rs` set up the RP2350 boot image):

```sh
cargo run                      # build + flash the firmware over USB, then run
cargo run --bin firstlight     # diagnostic: bit-bang HUB75 wiring test
cargo run --bin hub75test      # diagnostic: PIO driver full-color test pattern
cargo build / cargo clippy     # check (both clean)
```

Hold **BOOTSEL** while plugging in so the board enters its ROM bootloader, then `cargo run` flashes
via `picotool` (Homebrew install; **not** `elf2uf2-rs` — that emits the RP2040 UF2 family id, which
the RP2350 rejects). Logs come back over **USB-serial** on the same cable (`embassy-usb-logger`):
`screen /dev/tty.usbmodem*` (the lower-numbered data interface). No probe needed; a debug probe is an
optional upgrade (swap the runner to `probe-rs run --chip RP235x`).

## First-light test pattern

`cargo run --bin firstlight` scans a static pattern via plain GPIO (no PIO) to check wiring before
trusting the driver: solid R/G/B (channels + level shifter), a top/bottom split + row bands (address
lines, incl. the pin-12/D trap), and corner markers (orientation). A **vertical split / duplicated
image** points to an address-line error — most often the silk-"GND"/D pad (see
[hardware-wiring.md](hardware-wiring.md)).
