// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

//! Register level driver for the TI INA228, the pack monitor a scooter reports
//! its state of charge from.
//!
//! The chip measures bus voltage, shunt voltage, current, power and die
//! temperature, and accumulates charge and energy over time. Nothing here talks
//! to hardware by itself: it works on a [`bus::Bus`], so the register handling
//! and the arithmetic are testable without a board.

pub mod bus;

#[cfg(target_os = "linux")]
pub use bus::LinuxI2c;

use bus::Bus;

/// Registers, as listed in the datasheet and in TI's own driver.
pub mod regs {
    pub const CONFIG: u8 = 0x00;
    pub const ADC_CONFIG: u8 = 0x01;
    pub const SHUNT_CAL: u8 = 0x02;
    pub const SHUNT_TEMPCO: u8 = 0x03;
    pub const VSHUNT: u8 = 0x04;
    pub const VBUS: u8 = 0x05;
    pub const DIETEMP: u8 = 0x06;
    pub const CURRENT: u8 = 0x07;
    pub const POWER: u8 = 0x08;
    pub const ENERGY: u8 = 0x09;
    pub const CHARGE: u8 = 0x0A;
    /// Writing here clears the energy and charge accumulators.
    pub const RESET_ACCUMULATORS: u8 = 0x0B;
    /// The two identification registers sit at 0x3E and 0x3F. Sources name them
    /// the other way round, so [`Monitor::identifies`] accepts either.
    pub const ID_A: u8 = 0x3E;
    pub const ID_B: u8 = 0x3F;
}

/// Texas Instruments' manufacturer id in the identification registers.
const MANUFACTURER_TI: u16 = 0x5449;
/// The INA228 device id, in the high bits of its identification register.
const DEVICE_INA228: u16 = 0x2280;

/// Bus voltage resolution, 195.3125 µV per bit.
const VBUS_LSB_V: f64 = 195.3125e-6;
/// Shunt voltage resolution with the wide input range, 312.5 nV per bit.
const VSHUNT_LSB_V: f64 = 312.5e-9;
/// Shunt voltage resolution with the narrow input range, 78.125 nV per bit.
const VSHUNT_LSB_NARROW_V: f64 = 78.125e-9;
/// Power is 3.2 current steps per bit, energy 16 power steps per bit.
const POWER_LSB_PER_CURRENT_LSB: f64 = 3.2;
const ENERGY_LSB_PER_POWER_LSB: f64 = 16.0;
/// Datasheet constant for the calibration register.
const SHUNT_CAL_SCALE: f64 = 13107.2e6;
/// Half of a signed 20 bit reading.
const CURRENT_FULL_SCALE: f64 = 524_288.0;
/// Die temperature resolution, 7.8125 m°C per bit.
const DIETEMP_LSB_C: f64 = 7.8125e-3;

/// How the shunt is wired and what current it is expected to carry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shunt {
    pub ohms: f64,
    /// Upper end of the current the shunt is expected to see. Sets the scale,
    /// so it should be a little above the real peak.
    pub max_current_a: f64,
    /// True for the narrow ±40.96 mV range, which doubles the resolution.
    pub narrow: bool,
}

impl Default for Shunt {
    fn default() -> Self {
        Self {
            ohms: 0.0005,
            max_current_a: 50.0,
            narrow: false,
        }
    }
}

impl Shunt {
    /// Amperes per bit of the current register, `I_max / 2^19`.
    pub fn current_lsb(&self) -> f64 {
        self.max_current_a / CURRENT_FULL_SCALE
    }

    pub fn vshunt_lsb(&self) -> f64 {
        if self.narrow {
            VSHUNT_LSB_NARROW_V
        } else {
            VSHUNT_LSB_V
        }
    }

    /// The value to program into `SHUNT_CAL`, so the chip reports the current
    /// this scale expects.
    pub fn shunt_cal(&self) -> u16 {
        let cal = SHUNT_CAL_SCALE * self.current_lsb() * self.ohms;
        cal.round().clamp(0.0, f64::from(u16::MAX)) as u16
    }

    pub fn power_lsb(&self) -> f64 {
        POWER_LSB_PER_CURRENT_LSB * self.current_lsb()
    }

    pub fn energy_lsb(&self) -> f64 {
        ENERGY_LSB_PER_POWER_LSB * self.power_lsb()
    }

    pub fn charge_lsb(&self) -> f64 {
        self.current_lsb()
    }
}

/// One set of register readings, in the units the rest of LIVI works in.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Reading {
    pub bus_v: f64,
    pub shunt_v: f64,
    pub current_a: f64,
    pub power_w: f64,
    pub energy_j: f64,
    /// Accumulated charge in Coulombs, the counter a state of charge is built on.
    pub charge_c: f64,
    pub die_c: f64,
}

impl Reading {
    /// State of charge as a percentage, against the pack's rated capacity.
    ///
    /// The counter is expected to have been reset on a full pack, and a
    /// discharging current to read positive — which is what the low-side wiring
    /// in the manual gives. `charge_c` is therefore the charge taken out since
    /// that reset, so what is left is the capacity minus it.
    pub fn soc_percent(&self, capacity_ah: f64) -> Option<f64> {
        if capacity_ah <= 0.0 {
            return None;
        }
        let full_c = capacity_ah * 3600.0;
        Some(((full_c - self.charge_c) / full_c * 100.0).clamp(0.0, 100.0))
    }

    /// Remaining energy in Wh, against the pack's rated capacity.
    pub fn remaining_wh(&self, capacity_ah: f64, nominal_v: f64) -> Option<f64> {
        self.soc_percent(capacity_ah)
            .map(|p| p / 100.0 * nominal_v * capacity_ah)
    }
}

/// When a pack counts as full. Only the two measurements are needed, so this
/// works the same way for a boot-time check and for a running one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FullDetect {
    /// How far below the pack's full voltage the reading may sit. Covers the
    /// drop across wiring and the shunt, and measurement noise.
    pub margin_v: f64,
    /// How far above it may sit before the reading is treated as out of range
    /// rather than as a full pack. A pack cannot exceed its own full voltage by
    /// much, so a larger reading means the sense wiring is wrong.
    pub overshoot_v: f64,
    /// Current below which the pack is considered settled or done charging.
    pub idle_a: f64,
}

impl Default for FullDetect {
    fn default() -> Self {
        Self {
            margin_v: 0.2,
            overshoot_v: 2.0,
            idle_a: 0.5,
        }
    }
}

impl FullDetect {
    /// True when the pack sits at its full voltage with no real current.
    ///
    /// The current matters: during the constant voltage phase a pack reaches
    /// full voltage while still taking amps, and resetting the counter then
    /// would leave the reading permanently high. The upper bound matters just as
    /// much: a reading far above the pack's full voltage is a wiring fault, and
    /// treating it as full would reset the counter on every standstill.
    pub fn reached(&self, bus_v: f64, current_a: f64, full_v: f64) -> bool {
        let near_full = bus_v >= full_v - self.margin_v && bus_v <= full_v + self.overshoot_v;
        near_full && current_a.abs() <= self.idle_a
    }
}

/// A monitor on some bus.
pub struct Monitor<B: Bus> {
    bus: B,
    shunt: Shunt,
}

impl<B: Bus> Monitor<B> {
    pub fn new(bus: B, shunt: Shunt) -> Self {
        Self { bus, shunt }
    }

    pub fn shunt(&self) -> Shunt {
        self.shunt
    }

    /// True when the chip at this address answers with TI's manufacturer id and
    /// the INA228 device id.
    pub fn identifies(&mut self) -> Result<bool, bus::Error> {
        let a = self.bus.read_u16(regs::ID_A)?;
        let b = self.bus.read_u16(regs::ID_B)?;
        let ti = |v: u16| v == MANUFACTURER_TI;
        let ina = |v: u16| (v & 0xFFF0) == DEVICE_INA228;
        Ok((ti(a) && ina(b)) || (ti(b) && ina(a)))
    }

    pub fn configure(&mut self) -> Result<(), bus::Error> {
        self.bus.write_u16(regs::SHUNT_CAL, self.shunt.shunt_cal())
    }

    pub fn read(&mut self) -> Result<Reading, bus::Error> {
        let raw_vbus = self.bus.read_u24_shift4(regs::VBUS)?;
        let raw_vshunt = self.bus.read_i24_shift4(regs::VSHUNT)?;
        let raw_current = self.bus.read_i24_shift4(regs::CURRENT)?;
        let raw_power = self.bus.read_u24_shift4(regs::POWER)?;
        let raw_energy = self.bus.read_u40(regs::ENERGY)?;
        let raw_charge = self.bus.read_i40(regs::CHARGE)?;
        let raw_temp = self.bus.read_i16(regs::DIETEMP)?;
        Ok(Reading {
            bus_v: f64::from(raw_vbus) * VBUS_LSB_V,
            shunt_v: f64::from(raw_vshunt) * self.shunt.vshunt_lsb(),
            current_a: f64::from(raw_current) * self.shunt.current_lsb(),
            power_w: f64::from(raw_power) * self.shunt.power_lsb(),
            energy_j: raw_energy as f64 * self.shunt.energy_lsb(),
            charge_c: raw_charge as f64 * self.shunt.charge_lsb(),
            die_c: f64::from(raw_temp) * DIETEMP_LSB_C,
        })
    }

    /// Clears the energy and charge accumulators. Call it when the pack is full,
    /// so the next state of charge counts up from a known point.
    pub fn reset_accumulators(&mut self) -> Result<(), bus::Error> {
        self.bus.write(regs::RESET_ACCUMULATORS, &[])
    }
}

#[cfg(test)]
mod tests {
    use super::bus::Fake;
    use super::*;

    fn shunt() -> Shunt {
        Shunt {
            ohms: 0.0005,
            max_current_a: 50.0,
            narrow: false,
        }
    }

    #[test]
    fn the_scale_follows_the_shunt_and_the_expected_current() {
        let s = shunt();
        // 50 A over 2^19 steps is 95.37 µA per bit.
        assert!((s.current_lsb() - 50.0 / 524_288.0).abs() < 1e-12);
        // SHUNT_CAL = 13107.2e6 * CURRENT_LSB * R
        let expected = (13107.2e6_f64 * (50.0 / 524_288.0) * 0.0005).round();
        assert_eq!(f64::from(s.shunt_cal()), expected);
        assert!((s.charge_lsb() - s.current_lsb()).abs() < 1e-15);
        assert!((s.power_lsb() - 3.2 * s.current_lsb()).abs() < 1e-15);
        assert!((s.energy_lsb() - 16.0 * s.power_lsb()).abs() < 1e-12);
    }

    #[test]
    fn the_narrow_range_doubles_the_shunt_resolution() {
        let wide = shunt();
        let narrow = Shunt {
            narrow: true,
            ..shunt()
        };
        assert!((wide.vshunt_lsb() - 312.5e-9).abs() < 1e-15);
        assert!((narrow.vshunt_lsb() - 78.125e-9).abs() < 1e-15);
    }

    #[test]
    fn the_chip_is_recognised_from_either_register_order() {
        let mut bus = Fake::default();
        bus.set_u16(regs::ID_A, 0x5449);
        bus.set_u16(regs::ID_B, 0x2281);
        assert!(Monitor::new(bus, shunt()).identifies().unwrap());

        let mut bus = Fake::default();
        bus.set_u16(regs::ID_A, 0x2281);
        bus.set_u16(regs::ID_B, 0x5449);
        assert!(Monitor::new(bus, shunt()).identifies().unwrap());

        let mut bus = Fake::default();
        bus.set_u16(regs::ID_A, 0x1234);
        bus.set_u16(regs::ID_B, 0x5678);
        assert!(!Monitor::new(bus, shunt()).identifies().unwrap());
    }

    #[test]
    fn a_full_pack_on_a_60v_scale_reads_67_2_v() {
        let mut bus = Fake::default();
        // 67.2 V over a 195.3125 µV step is 344064, held in the top 20 bits.
        bus.set_u24(regs::VBUS, 344_064 << 4);
        let r = Monitor::new(bus, shunt()).read().unwrap();
        assert!((r.bus_v - 67.2).abs() < 1e-3);
    }

    #[test]
    fn the_charge_register_becomes_a_percentage() {
        let s = shunt();
        let mut bus = Fake::default();
        // 30 Ah at half charge, counted from a full pack: 54000 C taken out.
        let counts = (54_000.0 / s.charge_lsb()).round() as i64;
        bus.set_i40(regs::CHARGE, counts);
        let r = Monitor::new(bus, s).read().unwrap();
        let soc = r.soc_percent(30.0).unwrap();
        assert!((soc - 50.0).abs() < 0.01, "soc was {soc}");
        let wh = r.remaining_wh(30.0, 60.0).unwrap();
        assert!((wh - 900.0).abs() < 1.0, "remaining was {wh} Wh");
    }

    #[test]
    fn a_freshly_reset_counter_reads_full_and_empty_is_reached() {
        let s = shunt();
        let empty = Monitor::new(Fake::default(), s).read().unwrap();
        assert_eq!(
            empty.soc_percent(30.0),
            Some(100.0),
            "a reset counter starts full"
        );

        let mut bus = Fake::default();
        let counts = (108_000.0 / s.charge_lsb()).round() as i64;
        bus.set_i40(regs::CHARGE, counts);
        let drained = Monitor::new(bus, s).read().unwrap();
        assert_eq!(drained.soc_percent(30.0), Some(0.0), "30 Ah taken out");

        let mut bus = Fake::default();
        bus.set_i40(regs::CHARGE, 4_000_000_000);
        let over = Monitor::new(bus, s).read().unwrap();
        assert_eq!(over.soc_percent(30.0), Some(0.0), "clamped at empty");

        let mut bus = Fake::default();
        bus.set_i40(regs::CHARGE, -500_000_000);
        let charged = Monitor::new(bus, s).read().unwrap();
        assert_eq!(
            charged.soc_percent(30.0),
            Some(100.0),
            "charging past full clamps, it does not overflow"
        );
    }

    #[test]
    fn a_soc_without_capacity_is_unknown() {
        let r = Reading::default();
        assert_eq!(r.soc_percent(0.0), None);
        assert_eq!(r.soc_percent(-1.0), None);
        assert_eq!(r.remaining_wh(0.0, 60.0), None);
    }

    #[test]
    fn full_needs_the_voltage_and_a_settled_current() {
        let d = FullDetect::default();
        assert!(d.reached(67.2, 0.0, 67.2));
        assert!(d.reached(67.0, 0.3, 67.2), "margin and idle current");
        assert!(d.reached(67.1, -0.2, 67.2), "charging backwards counts too");
        assert!(!d.reached(66.9, 0.0, 67.2), "below the margin");
        assert!(
            !d.reached(67.2, 2.0, 67.2),
            "still charging at full voltage is not full"
        );
    }

    #[test]
    fn a_reading_far_above_the_pack_is_not_a_full_pack() {
        let d = FullDetect::default();
        // A broken sense wire can read well above the pack. Calling that full
        // would reset the counter whenever the load is switched off.
        assert!(!d.reached(18.5, 0.0, 12.6));
        assert!(d.reached(13.0, 0.0, 12.6), "a little over is still full");
        assert!(!d.reached(15.0, 0.0, 12.6), "well over is out of range");
    }

    #[test]
    fn configuring_writes_the_calibration() {
        let s = shunt();
        let mut bus = Fake::default();
        Monitor::new(&mut bus, s).configure().unwrap();
        assert_eq!(bus.written(regs::SHUNT_CAL), Some(s.shunt_cal()));
    }
}
