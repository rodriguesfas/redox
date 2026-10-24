//! Tiny helper to drive the Raspberry Pi 4 ACT LED (GPIO 42, active low)
//! without a full GPIO driver. Used as a blind-debug channel.

use core::ptr::{read_volatile, write_volatile};
use std::thread;
use std::time::Duration;

pub struct Led {
    base: usize,
}

impl Led {
    pub fn new(base: usize) -> Self {
        Self { base }
    }

    /// Configure GPIO 42 as an output (FSEL4, bits 6..8).
    pub fn init(&self) {
        unsafe {
            let fsel4 = read_volatile((self.base + 0x10) as *const u32);
            let cleared = fsel4 & !(0b111 << 6);
            write_volatile((self.base + 0x10) as *mut u32, cleared | (0b001 << 6));
        }
    }

    /// LED on (active low -> pin low).
    pub fn on(&self) {
        unsafe {
            write_volatile((self.base + 0x2C) as *mut u32, 1 << 10);
        }
    }

    /// LED off (pin high).
    pub fn off(&self) {
        unsafe {
            write_volatile((self.base + 0x20) as *mut u32, 1 << 10);
        }
    }

    pub fn blink(&self, n: u32, on_ms: u64, off_ms: u64) {
        for _ in 0..n {
            self.on();
            thread::sleep(Duration::from_millis(on_ms));
            self.off();
            thread::sleep(Duration::from_millis(off_ms));
        }
    }

    /// Report a milestone: blink `n` times, then pause.
    pub fn stage(&self, n: u32) {
        self.blink(n, 180, 180);
        thread::sleep(Duration::from_millis(900));
    }

    /// Fatal error: 10 rapid blinks, repeated.
    pub fn error(&self) {
        for _ in 0..4 {
            self.blink(10, 80, 80);
            thread::sleep(Duration::from_millis(1500));
        }
    }

    /// Alive/success: a single slow blink.
    pub fn success(&self) {
        self.on();
        thread::sleep(Duration::from_millis(400));
        self.off();
        thread::sleep(Duration::from_millis(400));
    }
}
/// Busy-wait for approximately `ms` milliseconds (no sleep syscall).
pub fn busy_wait(ms: u64) {
    // Rough calibration: ~1M iterations per ms on Pi 4
    let iters = ms * 1_000_000;
    for _ in 0..iters {
        core::hint::spin_loop();
    }
}
