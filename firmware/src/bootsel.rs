//! Runtime BOOTSEL-button read for the RP2350.
//!
//! embassy-rp 0.10's `bootsel` module is gated to RP2040, so this ports embassy `main`'s RP2350-ready
//! version (BOOTSEL is multiplexed onto the flash QSPI chip-select; float CS briefly and sense it).
//! The only change is the crate-private `in_ram` → a minimal single-core version: a critical section
//! is enough here because **none of our DMA ever reads flash** (HUB75 + cyw43 DMA are RAM-only) and
//! the sense runs from RAM with IRQs off, so momentarily floating CS can't disrupt anything.
//!
//! This is independent of the power-on bootrom BOOTSEL sampling — it does NOT affect entering
//! flashing mode (that's only sampled at reset, with BOOTSEL physically held).

use core::mem;

use embassy_rp::Peri;
use embassy_rp::pac::IO_QSPI;
use embassy_rp::pac::io::regs::{GpioCtrl, GpioStatus};
use embassy_rp::pac::io::vals::Oeover;
use embassy_rp::peripherals::BOOTSEL;

/// Reads the BOOTSEL button. Returns true while it's pressed. Core-0 only.
pub fn is_bootsel_pressed(_p: Peri<'_, BOOTSEL>) -> bool {
    unsafe {
        // QSPI SS (chip-select) is IO_QSPI gpio index 3 on RP2350 (index 1 on RP2040).
        let cs_gpio = IO_QSPI.gpio(3).as_ptr();

        let mut cs_ctrl = GpioCtrl::default();
        cs_ctrl.set_oeover(Oeover::DISABLE); // disable CS output drive → pad floats
        let cs_ctrl: u32 = mem::transmute(cs_ctrl);

        let mut cs_status = 0u32;
        in_ram(|| cs_status = ram_helpers::read_cs_status(cs_gpio, cs_ctrl));

        // BOOTSEL is active-low (the button pulls CS low through ~1 kΩ).
        !mem::transmute::<u32, GpioStatus>(cs_status).infrompad()
    }
}

/// Run `op` with interrupts disabled. `op` must touch only RAM (no flash/XIP). Single-core only —
/// no core1 to pause and no flash-reading DMA to drain (cf. embassy's fuller `flash::in_ram`).
#[inline]
unsafe fn in_ram(op: impl FnOnce()) {
    critical_section::with(|_| op());
}

mod ram_helpers {
    /// Temporarily floats the CS gpio and returns its `GpioStatus`. Lives in RAM and uses inline asm
    /// (no calls that might land in flash) so it's safe to run while XIP is briefly disturbed.
    #[inline(never)]
    #[unsafe(link_section = ".data.ram_func")]
    #[cfg(target_arch = "arm")]
    pub unsafe fn read_cs_status(cs_gpio: *mut (), cs_ctrl: u32) -> u32 {
        let cs_status: u32;
        unsafe {
            core::arch::asm!(
                ".equiv GPIO_STATUS, 0x0",
                ".equiv GPIO_CTRL,   0x4",

                "ldr {orig_ctrl}, [{cs_gpio}, $GPIO_CTRL]",

                // Disable CS's output drive and let it float...
                "str {val}, [{cs_gpio}, $GPIO_CTRL]",

                // ...wait for the state to settle (~4000-cycle delay loop)...
                "2:",
                "subs {delay}, #8",
                "bne 2b",

                // ...read the current state of bootsel...
                "ldr {val}, [{cs_gpio}, $GPIO_STATUS]",

                // ...and restore CS to normal operation so XIP can continue.
                "str {orig_ctrl}, [{cs_gpio}, $GPIO_CTRL]",

                cs_gpio = in(reg) cs_gpio,
                orig_ctrl = out(reg) _,
                val = inout(reg) cs_ctrl => cs_status,
                delay = in(reg) 8192,
                options(nostack),
            );
        }
        cs_status
    }

    #[cfg(not(target_arch = "arm"))]
    pub unsafe fn read_cs_status(_cs_gpio: *mut (), _cs_ctrl: u32) -> u32 {
        unimplemented!()
    }
}
