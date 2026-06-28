//! HUB75 LED-matrix driver for the RP2350 — PIO + DMA, zero CPU overhead.
//!
//! Ported from [`kjagiello/hub75-pio-rs`](https://github.com/kjagiello/hub75-pio-rs) onto embassy-rp.
//! Three PIO state machines (data / row / OE) on **PIO1** are fed by a **self-chaining 4-channel
//! DMA loop**, so the panel refreshes forever with no CPU involvement — which is also the fix for
//! the ESP build's flicker (refresh is fully decoupled from the radio). Color depth is **binary code
//! modulation (BCM)**: each of `B` bit-planes is displayed for `2^i − 1` ticks via the OE state
//! machine. Channel values pass through a **perceptual gamma LUT** ([`gamma_lut`]) on the way into
//! the framebuffer so equal-step ramps read as even brightness bands (see [`GAMMA`]).
//!
//! Differences from upstream: fixed dimensions (no `generic_const_exprs` nightly feature), embassy's
//! PIO `Config` API, and the DMA chain written against `embassy_rp::pac` (embassy's safe DMA API
//! doesn't expose channel chaining / the `al2_write_addr_trig` reload trick).
//!
//! Pin map (PIO1): R1=GP0 G1=GP1 B1=GP2 R2=GP3 G2=GP4 B2=GP5, CLK=GP6 LAT=GP7 OE=GP8,
//! A=GP9 B=GP10 C=GP11 D=GP12 E=GP13. See docs/hub75-pico-wiring.md.

use embassy_rp::Peri;
use embassy_rp::pac;
use embassy_rp::peripherals::{
    DMA_CH2, DMA_CH3, DMA_CH4, DMA_CH5, PIN_0, PIN_1, PIN_2, PIN_3, PIN_4, PIN_5, PIN_6, PIN_7,
    PIN_8, PIN_9, PIN_10, PIN_11, PIN_12, PIN_13, PIO1,
};
use embassy_rp::pio::{
    Common, Config, Direction, FifoJoin, LoadedProgram, Pin, ShiftConfig, ShiftDirection,
    StateMachine,
};
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::*;
use fixed::FixedU32;
use fixed::types::extra::U8;

// --- Panel + color configuration (tunables) ---
pub const W: usize = 64;
pub const H: usize = 64;
/// Bit-planes (color depth per channel). More = richer color but slower refresh + more RAM.
pub const B: usize = 10;
/// Address lines (1/32 scan = 5: A–E).
pub const ADDR_PINS: usize = 5;
/// Perceptual gamma applied to every channel in [`Display::set_pixel`] (see [`gamma_lut`]). The
/// panel emits light ~linearly in the BCM code, but the eye is non-linear, so without this an
/// equal-step ramp crushes into the dark end and the bright end looks flat. `2.2` ≈ sRGB; raise for
/// more contrast, lower toward `1.0` to flatten (`1.0` = off / raw linear light). Calibrate on the
/// panel with `cargo run --bin calibrate`.
pub const GAMMA: f32 = 2.2;
/// Framebuffer bytes: one tuple byte per pixel of the active half, × B planes.
const FB_BYTES: usize = W * H / 2 * B;

// Clock dividers (sys_clk ≈ 150 MHz). Data /4 ≈ 18.75 MHz pixel clock (conservative, matches the
// ESP's proven 20 MHz). Row/OE near full speed; OE divider scales the BCM dwell — tune for
// brightness/color once lit.
pub const DATA_DIV: u16 = 4;
const ROW_DIV: u16 = 1;
pub const OE_DIV: u16 = 8;

// DMA channels (must match the Peris passed to `new`). cyw43 owns PIO0 + DMA_CH0/CH1.
const FB_CH: usize = 2;
const FB_LOOP_CH: usize = 3;
const OE_CH: usize = 4;
const OE_LOOP_CH: usize = 5;

/// BCM dwell weights: plane `i` is displayed for `2^i − 1` OE ticks.
const fn delays() -> [u32; B] {
    let mut arr = [0u32; B];
    let mut i = 0;
    while i < B {
        arr[i] = (1 << i) - 1;
        i += 1;
    }
    arr
}

/// Build the perceptual gamma LUT mapping an 8-bit encoded channel (`0..=255`, as embedded-graphics
/// delivers) to a linear-light BCM code in `0..=2^B-1`. Output is `(in/255)^gamma · (2^B-1)`.
/// Endpoints are identity-ish (`0→0`, `255→max`), so solid-color content (status text/UI) is
/// unaffected — only intermediate levels move. Built once at startup; see [`GAMMA`].
///
/// Note the dark-end cost: with `B = 8` a `2.2` curve maps the lowest ~20 inputs all to `0`, so deep
/// gradients lose resolution. If darks band, raise `B` (e.g. `10`–`11`) for headroom — cheap on the
/// RP2350 (RAM is `2·64·64/2·B` bytes) and refresh stays well above flicker (see performance.md).
fn gamma_lut(gamma: f32) -> [u16; 256] {
    let max = ((1u32 << B) - 1) as f32;
    let mut lut = [0u16; 256];
    for (i, slot) in lut.iter_mut().enumerate() {
        let norm = i as f32 / 255.0;
        *slot = (libm::powf(norm, gamma) * max + 0.5) as u16;
    }
    lut
}

/// Backing storage for the framebuffers + DMA reload pointers. Lives in a `StaticCell`.
///
/// Layout (one byte per pixel tuple `XXBGRBGR`, bits 0..2 = top half, 3..5 = bottom half), walked
/// row → bit-plane → column so the data SM streams it with no reordering.
#[repr(C)]
pub struct DisplayMemory {
    fbptr: [u32; 1],
    fb0: [u8; FB_BYTES],
    fb1: [u8; FB_BYTES],
    delays: [u32; B],
    delaysptr: [u32; 1],
}

impl DisplayMemory {
    pub const fn new() -> Self {
        Self {
            fbptr: [0],
            fb0: [0; FB_BYTES],
            fb1: [0; FB_BYTES],
            delays: delays(),
            delaysptr: [0],
        }
    }
}

impl Default for DisplayMemory {
    fn default() -> Self {
        Self::new()
    }
}

/// The 14 HUB75 GPIO peripherals (see docs/hub75-pico-wiring.md).
pub struct Hub75Pins {
    pub r1: Peri<'static, PIN_0>,
    pub g1: Peri<'static, PIN_1>,
    pub b1: Peri<'static, PIN_2>,
    pub r2: Peri<'static, PIN_3>,
    pub g2: Peri<'static, PIN_4>,
    pub b2: Peri<'static, PIN_5>,
    pub clk: Peri<'static, PIN_6>,
    pub lat: Peri<'static, PIN_7>,
    pub oe: Peri<'static, PIN_8>,
    pub a: Peri<'static, PIN_9>,
    pub b: Peri<'static, PIN_10>,
    pub c: Peri<'static, PIN_11>,
    pub d: Peri<'static, PIN_12>,
    pub e: Peri<'static, PIN_13>,
}

/// DMA channels the driver drives via PAC (CH2–CH5). cyw43 uses CH0/CH1.
pub struct Hub75Dma {
    pub fb: Peri<'static, DMA_CH2>,
    pub fb_loop: Peri<'static, DMA_CH3>,
    pub oe: Peri<'static, DMA_CH4>,
    pub oe_loop: Peri<'static, DMA_CH5>,
}

fn div(n: u16) -> FixedU32<U8> {
    FixedU32::<U8>::from_num(n)
}

/// The HUB75 display driver. Drawing goes through the `DrawTarget` impl into the inactive buffer;
/// `commit()` flips. The DMA/PIO refresh runs forever in hardware.
pub struct Display {
    mem: &'static mut DisplayMemory,
    /// Perceptual gamma LUT (8-bit channel → BCM code), built from [`GAMMA`] at construction.
    lut: [u16; 256],
    // Held to keep the PIO + DMA hardware alive for the life of the program.
    _common: Common<'static, PIO1>,
    _data_sm: StateMachine<'static, PIO1, 0>,
    _row_sm: StateMachine<'static, PIO1, 1>,
    _oe_sm: StateMachine<'static, PIO1, 2>,
    _programs: [LoadedProgram<'static, PIO1>; 3],
    _pins: [Pin<'static, PIO1>; 14],
    _dma: Hub75Dma,
}

impl Display {
    /// Bring up the three PIO state machines + the self-chaining DMA loops and start refreshing.
    pub fn new(
        mem: &'static mut DisplayMemory,
        mut common: Common<'static, PIO1>,
        mut data_sm: StateMachine<'static, PIO1, 0>,
        mut row_sm: StateMachine<'static, PIO1, 1>,
        mut oe_sm: StateMachine<'static, PIO1, 2>,
        pins: Hub75Pins,
        dma: Hub75Dma,
    ) -> Self {
        // Make all 14 pins PIO-owned (RP2350 needs explicit pin setup — see module docs).
        let r1 = common.make_pio_pin(pins.r1);
        let g1 = common.make_pio_pin(pins.g1);
        let b1 = common.make_pio_pin(pins.b1);
        let r2 = common.make_pio_pin(pins.r2);
        let g2 = common.make_pio_pin(pins.g2);
        let b2 = common.make_pio_pin(pins.b2);
        let clk = common.make_pio_pin(pins.clk);
        let lat = common.make_pio_pin(pins.lat);
        let oe = common.make_pio_pin(pins.oe);
        let a = common.make_pio_pin(pins.a);
        let b = common.make_pio_pin(pins.b);
        let c = common.make_pio_pin(pins.c);
        let d = common.make_pio_pin(pins.d);
        let e = common.make_pio_pin(pins.e);

        // --- Data SM: shift RGB tuples + clock (CLK on side-set) ---
        let data_prg = pio::pio_asm!(
            ".side_set 1",
            "out isr, 32    side 0b0", // pull screen width-1 into ISR (one-time)
            ".wrap_target",
            "mov x isr      side 0b0",
            "pixel:",
            "out pins, 8    side 0b0", // 8 bits -> 6 RGB pins (top 2 discarded), CLK low
            "jmp x-- pixel  side 0b1", // CLK high: clock the pixel in
            "irq 4          side 0b0", // row of data in; tell row SM
            "wait 1 irq 5   side 0b0", // wait for "send next row"
            ".wrap",
        );
        let data_loaded = common.load_program(&data_prg.program);
        {
            let mut cfg = Config::default();
            cfg.use_program(&data_loaded, &[&clk]);
            cfg.set_out_pins(&[&r1, &g1, &b1, &r2, &g2, &b2]);
            cfg.clock_divider = div(DATA_DIV);
            cfg.shift_out = ShiftConfig {
                auto_fill: true,
                threshold: 32,
                direction: ShiftDirection::Right,
            };
            cfg.fifo_join = FifoJoin::TxOnly;
            data_sm.set_config(&cfg);
            data_sm.set_pin_dirs(Direction::Out, &[&r1, &g1, &b1, &r2, &g2, &b2, &clk]);
        }

        // --- Row SM: drive A–E + LAT (LAT on side-set), advance row/bit-plane ---
        let row_prg = pio::pio_asm!(
            ".side_set 1",
            "pull           side 0b0", // height/2 - 1
            "out isr, 32    side 0b0",
            "pull           side 0b0", // color depth - 1
            ".wrap_target",
            "mov x, isr     side 0b0",
            "addr:",
            "mov pins, ~x   side 0b0", // address = ~x
            "mov y, osr     side 0b0",
            "row:",
            "wait 1 irq 4   side 0b0", // wait until data clocked in
            "nop            side 0b1", // LAT high
            "irq 6          side 0b1", // tell OE SM to run the display window
            "irq 5          side 0b0", // tell data SM to clock the next row
            "wait 1 irq 7   side 0b0", // wait for OE window to finish
            "jmp y-- row    side 0b0",
            "jmp x-- addr   side 0b0",
            ".wrap",
        );
        let row_loaded = common.load_program(&row_prg.program);
        {
            let mut cfg = Config::default();
            cfg.use_program(&row_loaded, &[&lat]);
            cfg.set_out_pins(&[&a, &b, &c, &d, &e]);
            cfg.clock_divider = div(ROW_DIV);
            row_sm.set_config(&cfg);
            row_sm.set_pin_dirs(Direction::Out, &[&a, &b, &c, &d, &e, &lat]);
        }

        // --- OE SM: BCM dwell timing, OE on side-set ---
        let oe_prg = pio::pio_asm!(
            ".side_set 1",
            ".wrap_target",
            "out x, 32      side 0b1", // dwell length for this bit-plane; OE high (blanked)
            "wait 1 irq 6   side 0b1", // wait for latch
            "delay:",
            "jmp x-- delay  side 0b0", // OE low: display window
            "irq 7          side 0b1", // done; OE high
            ".wrap",
        );
        let oe_loaded = common.load_program(&oe_prg.program);
        {
            let mut cfg = Config::default();
            cfg.use_program(&oe_loaded, &[&oe]);
            cfg.clock_divider = div(OE_DIV);
            cfg.shift_out = ShiftConfig {
                auto_fill: true,
                threshold: 32,
                direction: ShiftDirection::Right,
            };
            cfg.fifo_join = FifoJoin::TxOnly;
            oe_sm.set_config(&cfg);
            oe_sm.set_pin_dirs(Direction::Out, &[&oe]);
        }

        // Seed the SM FIFOs with the one-time config words (consumed by the startup pulls).
        data_sm.tx().push((W - 1) as u32);
        row_sm.tx().push((H / 2 - 1) as u32);
        row_sm.tx().push((B - 1) as u32);

        // DMA reload pointers must be live before the loops start.
        mem.fbptr[0] = mem.fb0.as_ptr() as u32;
        mem.delaysptr[0] = mem.delays.as_ptr() as u32;

        let data_fifo = data_sm.tx_fifo_ptr() as u32;
        let data_treq = data_sm.tx_treq();
        let oe_fifo = oe_sm.tx_fifo_ptr() as u32;
        let oe_treq = oe_sm.tx_treq();

        // --- Framebuffer DMA loop: CH(FB) streams fb -> data FIFO; CH(FB_LOOP) reloads CH(FB)'s
        // read addr from `fbptr` and retriggers it. Flipping `fbptr` swaps buffers. ---
        setup_feed_channel(
            FB_CH,
            mem.fbptr[0],
            (FB_BYTES / 4) as u32,
            data_fifo,
            data_treq,
            FB_LOOP_CH,
        );
        setup_loop_channel(
            FB_LOOP_CH,
            mem.fbptr.as_ptr() as u32,
            pac::DMA.ch(FB_CH).read_addr().as_ptr() as u32,
            FB_CH,
        );

        // --- OE DMA loop: same pattern feeding the BCM `delays` to the OE SM. ---
        setup_feed_channel(
            OE_CH,
            mem.delays.as_ptr() as u32,
            B as u32,
            oe_fifo,
            oe_treq,
            OE_LOOP_CH,
        );
        setup_loop_channel(
            OE_LOOP_CH,
            mem.delaysptr.as_ptr() as u32,
            pac::DMA.ch(OE_CH).read_addr().as_ptr() as u32,
            OE_CH,
        );

        data_sm.set_enable(true);
        row_sm.set_enable(true);
        oe_sm.set_enable(true);

        Self {
            mem,
            lut: gamma_lut(GAMMA),
            _common: common,
            _data_sm: data_sm,
            _row_sm: row_sm,
            _oe_sm: oe_sm,
            _programs: [data_loaded, row_loaded, oe_loaded],
            _pins: [r1, g1, b1, r2, g2, b2, clk, lat, oe, a, b, c, d, e],
            _dma: dma,
        }
    }

    fn fb_loop_busy(&self) -> bool {
        pac::DMA.ch(FB_LOOP_CH).ctrl_trig().read().busy()
    }

    /// Read-only refresh probe (for `bin/refbench`). The framebuffer DMA streams the whole active
    /// buffer once per refreshed frame, so its read address ramps from the buffer base to the end
    /// and resets each frame. Counting wrap-arounds (a drop in this value) over a known interval
    /// yields the measured refresh rate. Has no effect on the running display.
    pub fn fb_read_addr(&self) -> u32 {
        pac::DMA.ch(FB_CH).read_addr().read()
    }

    /// Flip the buffers. Call after drawing a frame into the inactive buffer; this makes it live and
    /// clears the now-inactive buffer for the next frame.
    pub fn commit(&mut self) {
        if self.mem.fbptr[0] == self.mem.fb0.as_ptr() as u32 {
            self.mem.fbptr[0] = self.mem.fb1.as_ptr() as u32;
            while !self.fb_loop_busy() {}
            self.mem.fb0.fill(0);
        } else {
            self.mem.fbptr[0] = self.mem.fb0.as_ptr() as u32;
            while !self.fb_loop_busy() {}
            self.mem.fb1.fill(0);
        }
    }

    /// Write one pixel into the *inactive* buffer (the DMA isn't scanning it).
    pub fn set_pixel(&mut self, x: usize, y: usize, color: Rgb888) {
        // Un-mirror (pairs with `mov pins, ~x` in the row program).
        let x = W - 1 - x;
        let y = H - 1 - y;
        let shift = if y > H / 2 - 1 { 3 } else { 0 }; // bottom half -> bits 3..5
        // Perceptual gamma + map 8-bit channels to the B-bit BCM code range (see `gamma_lut`).
        let cr = self.lut[color.r() as usize];
        let cg = self.lut[color.g() as usize];
        let cb = self.lut[color.b() as usize];
        let base = x + (y % (H / 2)) * W * B;
        let fb = if self.mem.fbptr[0] == self.mem.fb0.as_ptr() as u32 {
            &mut self.mem.fb1
        } else {
            &mut self.mem.fb0
        };
        for plane in 0..B {
            let bit = ((cb >> plane) & 1) << 2 | ((cg >> plane) & 1) << 1 | ((cr >> plane) & 1);
            let idx = base + plane * W;
            fb[idx] = (fb[idx] & !(0b111 << shift)) | ((bit as u8) << shift);
        }
    }
}

/// Configure a "feed" DMA channel: incrementing read from a buffer, fixed write to a PIO TX FIFO,
/// paced by the SM's DREQ, chained to its reload channel. Enabled but not triggered (the reload
/// channel kicks it off / re-triggers it).
fn setup_feed_channel(
    ch: usize,
    read_addr: u32,
    words: u32,
    fifo_addr: u32,
    treq: pac::dma::vals::TreqSel,
    chain_to: usize,
) {
    let mut ctrl = pac::dma::regs::CtrlTrig(0);
    ctrl.set_incr_read(true);
    ctrl.set_incr_write(false);
    ctrl.set_data_size(pac::dma::vals::DataSize::SIZE_WORD);
    ctrl.set_treq_sel(treq);
    ctrl.set_irq_quiet(true);
    ctrl.set_chain_to(chain_to as u8);
    ctrl.set_en(true);
    let c = pac::DMA.ch(ch);
    c.al1_ctrl().write_value(ctrl.0); // non-triggering ctrl write
    c.read_addr().write_value(read_addr);
    c.trans_count().write(|w| w.set_count(words));
    c.write_addr().write_value(fifo_addr);
}

/// Configure a "reload" DMA channel: reads a single pointer word and writes it to the feed
/// channel's read-addr trigger register, chained back to the feed channel. The final
/// `al2_write_addr_trig` write triggers the loop into motion.
fn setup_loop_channel(ch: usize, ptr_addr: u32, feed_read_addr_reg: u32, chain_to: usize) {
    let mut ctrl = pac::dma::regs::CtrlTrig(0);
    ctrl.set_incr_read(false);
    ctrl.set_incr_write(false);
    ctrl.set_data_size(pac::dma::vals::DataSize::SIZE_WORD);
    ctrl.set_treq_sel(pac::dma::vals::TreqSel::PERMANENT);
    ctrl.set_irq_quiet(true);
    ctrl.set_chain_to(chain_to as u8);
    ctrl.set_en(true);
    let c = pac::DMA.ch(ch);
    c.al1_ctrl().write_value(ctrl.0);
    c.read_addr().write_value(ptr_addr);
    c.trans_count().write(|w| w.set_count(1));
    c.al2_write_addr_trig().write_value(feed_read_addr_reg); // sets write addr + triggers
}

impl OriginDimensions for Display {
    fn size(&self) -> Size {
        Size::new(W as u32, H as u32)
    }
}

impl DrawTarget for Display {
    type Color = Rgb888;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels {
            if (0..W as i32).contains(&coord.x) && (0..H as i32).contains(&coord.y) {
                self.set_pixel(coord.x as usize, coord.y as usize, color);
            }
        }
        Ok(())
    }
}
