// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

//! Reads the indicator and high beam lines of a scooter.
//!
//! The original control unit drives the instrument cluster with 12 V, which a
//! GPIO cannot take, so each signal comes in through a level shifter and is read
//! as a plain high or low. What arrives here is published as telemetry, which is
//! where the dashboard lamps already look.

use livi_core_proto::config::Config;
use serde_json::{Value, json};

/// What the four inputs currently say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Lines {
    pub left: bool,
    pub right: bool,
    pub high_beam: bool,
    /// Wired for a later use, read to keep the pin from floating unused.
    pub spare: bool,
}

/// How many lines the reader claims. The order matches `pins`.
pub const LINES: usize = 4;

/// The telemetry these lines turn into.
///
/// Both indicators at once is what a hazard flasher looks like from outside, so
/// that is reported as hazards rather than as a direction. Charging from the
/// phone is left alone: an unset direction stays `none`.
pub fn patch(lines: Lines) -> Value {
    let (turn, hazards) = match (lines.left, lines.right) {
        (true, true) => ("none", true),
        (true, false) => ("left", false),
        (false, true) => ("right", false),
        (false, false) => ("none", false),
    };
    json!({
        "turnSignal": turn,
        "hazards": hazards,
        "highBeam": lines.high_beam,
    })
}

/// The four pins, or `None` when the lines are not wired.
pub fn pins(cfg: &Config) -> Option<[u32; 4]> {
    let all = [
        cfg.scooter_indicator_left_gpio,
        cfg.scooter_indicator_right_gpio,
        cfg.scooter_high_beam_gpio,
        cfg.scooter_indicator_spare_gpio,
    ];
    if all.iter().any(|p| *p < 0) {
        return None;
    }
    let mut out = [0u32; 4];
    for (slot, pin) in out.iter_mut().zip(all) {
        *slot = u32::try_from(pin).ok()?;
    }
    Some(out)
}

/// One bitmap from the kernel into the four lines, in the order `pins` gives.
pub fn decode(bits: u64) -> Lines {
    let on = |i: usize| bits & (1 << i) != 0;
    Lines {
        left: on(0),
        right: on(1),
        high_beam: on(2),
        spare: on(3),
    }
}

pub async fn run(
    core: std::sync::Arc<crate::server::Core>,
    inside: tokio::sync::mpsc::UnboundedSender<Value>,
) {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (core, inside);
    }
    #[cfg(target_os = "linux")]
    linux::run(core, inside).await;
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs;
    use std::os::fd::AsRawFd;
    use std::time::Duration;

    use livi_core_proto::config::CarType;
    use serde_json::Value;

    use super::{Lines, decode, patch, pins};

    const GPIO_CHIP: &str = "/dev/gpiochip0";
    /// Input, biased low so an unwired or unpowered line reads as off.
    const FLAG_INPUT: u64 = 1 << 2;
    const FLAG_BIAS_PULL_DOWN: u64 = 1 << 9;
    const GET_LINE_IOCTL: u32 =
        (3 << 30) | ((size_of::<LineRequest>() as u32) << 16) | (0xB4 << 8) | 0x07;
    /// `GPIO_V2_LINE_GET_VALUES_IOCTL`. `linux/gpio.h` numbers it `0x0E`; `0x0F`
    /// is `SET_VALUES`, which the kernel answers with `EINVAL` when it is handed
    /// a read buffer.
    const GET_VALUES_IOCTL: u32 =
        (3 << 30) | ((size_of::<LineValues>() as u32) << 16) | (0xB4 << 8) | 0x0E;
    /// The four lines share one request, so their bits travel together. The
    /// kernel rejects the call when the mask names no line at all.
    const LINES_MASK: u64 = (1 << super::LINES) - 1;
    /// A turn signal blinks at a few hertz; polling faster than this gains
    /// nothing and keeps the CPU out of the idle states for no reason.
    const POLL: Duration = Duration::from_millis(50);
    const RETRY: Duration = Duration::from_secs(5);

    #[repr(C)]
    struct LineRequest {
        offsets: [u32; 64],
        consumer: [u8; 32],
        flags: u64,
        num_attrs: u32,
        config_padding: [u32; 5],
        attrs: [[u64; 3]; 10],
        num_lines: u32,
        event_buffer_size: u32,
        padding: [u32; 5],
        fd: i32,
    }

    #[repr(C)]
    struct LineValues {
        bits: u64,
        mask: u64,
    }

    const _: () = assert!(size_of::<LineRequest>() == 592 && size_of::<LineValues>() == 16);

    struct Lines4(fs::File);

    impl Lines4 {
        fn claim(offsets: [u32; 4]) -> Result<Self, String> {
            let chip = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(GPIO_CHIP)
                .map_err(|e| format!("open {GPIO_CHIP}: {e}"))?;
            let mut req: LineRequest = unsafe { std::mem::zeroed() };
            req.offsets[..4].copy_from_slice(&offsets);
            req.num_lines = 4;
            req.flags = FLAG_INPUT | FLAG_BIAS_PULL_DOWN;
            req.consumer[..11].copy_from_slice(b"livi-inputs");
            if unsafe { libc::ioctl(chip.as_raw_fd(), GET_LINE_IOCTL as _, &mut req) } < 0 {
                return Err(format!(
                    "claim gpio {offsets:?}: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self(unsafe {
                <fs::File as std::os::fd::FromRawFd>::from_raw_fd(req.fd)
            }))
        }

        fn read(&self) -> std::io::Result<Lines> {
            let mut values = LineValues {
                bits: 0,
                mask: LINES_MASK,
            };
            if unsafe { libc::ioctl(self.0.as_raw_fd(), GET_VALUES_IOCTL as _, &mut values) } < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(decode(values.bits))
        }
    }

    pub async fn run(
        core: std::sync::Arc<crate::server::Core>,
        inside: tokio::sync::mpsc::UnboundedSender<Value>,
    ) {
        if core.hub.config().car_type != Some(CarType::Scooter) {
            return;
        }
        let lines = loop {
            let cfg = core.hub.config();
            let Some(offsets) = pins(&cfg) else {
                println!("[inputs] no indicator pins configured");
                return;
            };
            match Lines4::claim(offsets) {
                Ok(l) => break l,
                Err(e) => {
                    eprintln!("[inputs] {e}");
                    tokio::time::sleep(RETRY).await;
                }
            }
        };
        println!("[inputs] indicator and high beam lines on gpiochip0");

        let mut last: Option<Lines> = None;
        loop {
            tokio::time::sleep(POLL).await;
            let now = match lines.read() {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[inputs] read failed: {e}");
                    tokio::time::sleep(RETRY).await;
                    continue;
                }
            };
            if last == Some(now) {
                continue;
            }
            last = Some(now);
            let _ = inside.send(patch(now));
        }
    }
}

#[cfg(test)]
mod tests {
    use livi_core_proto::config::Config;

    use super::*;

    fn lines(left: bool, right: bool, high_beam: bool) -> Lines {
        Lines {
            left,
            right,
            high_beam,
            spare: false,
        }
    }

    #[test]
    fn one_side_lights_that_side() {
        assert_eq!(patch(lines(true, false, false))["turnSignal"], "left");
        assert_eq!(patch(lines(false, true, false))["turnSignal"], "right");
        assert_eq!(patch(lines(false, false, false))["turnSignal"], "none");
    }

    #[test]
    fn both_sides_are_a_hazard_flasher() {
        let p = patch(lines(true, true, false));
        assert_eq!(p["hazards"], true);
        assert_eq!(p["turnSignal"], "none", "no direction while both blink");
    }

    #[test]
    fn the_high_beam_is_reported_on_its_own() {
        assert_eq!(patch(lines(false, false, true))["highBeam"], true);
        assert_eq!(patch(lines(false, false, false))["highBeam"], false);
        let p = patch(lines(true, false, true));
        assert_eq!(p["turnSignal"], "left");
        assert_eq!(p["highBeam"], true);
    }

    #[test]
    fn the_bitmap_lands_on_the_right_line() {
        assert_eq!(decode(0b0000), lines(false, false, false));
        assert_eq!(decode(0b0001), lines(true, false, false));
        assert_eq!(decode(0b0010), lines(false, true, false));
        assert_eq!(decode(0b0100), lines(false, false, true));
        assert_eq!(decode(0b1000).spare, true);
        assert_eq!(decode(0b0111), lines(true, true, true));
        // Bits above the four claimed lines belong to nothing.
        assert_eq!(decode(0b1111_0000), lines(false, false, false));
    }

    #[test]
    fn the_pins_come_from_the_settings() {
        let cfg = Config::default();
        assert_eq!(pins(&cfg), Some([5, 6, 13, 19]));

        let mut off = cfg.clone();
        off.scooter_indicator_spare_gpio = -1;
        assert_eq!(pins(&off), None, "one unwired line disables the reader");
    }
}
