// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

//! Reads a scooter's pack from an INA228 and publishes its state of charge.
//!
//! A scooter reports no vehicle telemetry of its own, so the pack has to be
//! measured. The chip accumulates charge; against the capacity set in the car
//! settings that becomes a percentage, and against the pack's limits a decision
//! about when to reset the counter.

use std::sync::Arc;
use std::time::Duration;

use livi_core_proto::config::{CarType, Config};
use livi_ina228::{FullDetect, Monitor, Reading, Shunt};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::server::Core;

/// The bus the monitor sits on, and its address, which the chip fixes.
#[cfg(target_os = "linux")]
const BUS: u32 = 1;
const ADDRESS: u16 = 0x40;
/// How often the pack is sampled.
const SAMPLE: Duration = Duration::from_secs(2);
/// How long to wait before looking for the chip again.
const RETRY: Duration = Duration::from_secs(10);

/// The shunt the scooter is wired with, taken from the car settings.
fn shunt(cfg: &Config) -> Shunt {
    Shunt {
        ohms: cfg.scooter_shunt_milliohm / 1000.0,
        max_current_a: cfg.scooter_shunt_max_current_a,
        narrow: false,
    }
}

/// The reading with the configured bus-voltage correction applied.
fn corrected(cfg: &Config, reading: &Reading) -> Reading {
    let scale = cfg.scooter_vbus_scale;
    if scale > 0.0 && scale.is_finite() && (scale - 1.0).abs() > f64::EPSILON {
        Reading {
            bus_v: reading.bus_v * scale,
            ..*reading
        }
    } else {
        *reading
    }
}

/// The telemetry an INA228 reading turns into. Kept free of I/O so the mapping
/// is testable without a chip.
///
/// `counted` says whether the counter has a reference from a full pack. Without
/// one the accumulated charge means nothing, so only the voltage and current are
/// published and the dashboard keeps showing its label instead of a made-up
/// percentage.
pub fn patch(cfg: &Config, reading: &Reading, counted: bool) -> Option<Value> {
    if cfg.car_type != Some(CarType::Scooter) {
        return None;
    }
    let reading = corrected(cfg, reading);
    let mut out = json!({
        // Watt-hour resolution: a small pack is only tens of Wh, so rounding to
        // hundredths of a kWh would move the readout in 10 Wh steps.
        "batteryCapacityKwh": cfg.scooter_pack_wh().round() / 1000.0,
        "packVoltage": (reading.bus_v * 100.0).round() / 100.0,
        "packCurrent": (reading.current_a * 100.0).round() / 100.0,
    });
    if !counted {
        return Some(out);
    }
    let capacity = cfg.scooter_battery_capacity_ah;
    let soc = reading.soc_percent(capacity)?;
    let remaining_wh = reading.remaining_wh(capacity, f64::from(cfg.scooter_battery_voltage))?;
    let map = out.as_object_mut()?;
    map.insert("fuelPct".into(), json!((soc * 10.0).round() / 10.0));
    map.insert(
        "batteryLevelKwh".into(),
        json!(remaining_wh.round() / 1000.0),
    );
    Some(out)
}

/// True when the reading says the pack is full, which is the point to restart
/// the charge counter from.
pub fn at_full(cfg: &Config, reading: &Reading) -> bool {
    let pack = cfg.scooter_pack();
    let reading = corrected(cfg, reading);
    FullDetect::default().reached(reading.bus_v, reading.current_a, pack.full_v())
}

/// Accumulated charge that counts as evidence the counter has been running.
/// A chip whose pack has never been seen full reports zero, which is exactly the
/// indistinguishable-from-full case; anything above a coulomb is real movement.
const REFERENCE_C: f64 = 1.0;

/// What one sample means for the task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Zero the accumulators, then treat this reading as a full pack.
    Reset,
    /// Publish the reading as it is.
    Publish,
}

/// Follows a pack across samples and decides where its percentage comes from.
///
/// Two facts are tracked apart, and conflating them is what froze the gauge: a
/// pack is "full right now" only while it sits at its full voltage, but the
/// reference the counter was zeroed against stays valid for the whole discharge.
#[derive(Debug, Default)]
pub struct Tracker {
    /// A full pack has been seen at least once since this task started.
    seen_full: bool,
    /// The counter was already zeroed on the pack's current full charge.
    reset_done: bool,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fed every reading in order. Returns `Reset` once per full charge, so a
    /// pack that is charged, discharged and charged again gets a fresh
    /// reference each time.
    pub fn observe(&mut self, cfg: &Config, reading: &Reading) -> Step {
        if at_full(cfg, reading) {
            if !self.reset_done {
                self.reset_done = true;
                self.seen_full = true;
                return Step::Reset;
            }
        } else {
            self.reset_done = false;
        }
        Step::Publish
    }

    /// True when the accumulated charge may be read as what has been taken out
    /// since full.
    ///
    /// The counter is only ever zeroed on a full pack, so a non-zero accumulator
    /// is itself evidence that the reference exists. That keeps the percentage
    /// alive across a restart of this task, while the pack that was never full
    /// stays unclaimed.
    pub fn counted(&self, reading: &Reading) -> bool {
        self.seen_full || reading.charge_c.abs() >= REFERENCE_C
    }
}

pub async fn run(core: Arc<Core>, inside: mpsc::UnboundedSender<Value>) {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (core, inside);
    }
    #[cfg(target_os = "linux")]
    linux::run(core, inside).await;
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use livi_ina228::LinuxI2c;

    pub async fn run(core: Arc<Core>, inside: mpsc::UnboundedSender<Value>) {
        let cfg = core.hub.config();
        if cfg.car_type != Some(CarType::Scooter) || !cfg.scooter_battery_sensor {
            return;
        }
        let mut monitor = loop {
            match LinuxI2c::open(BUS, ADDRESS) {
                Ok(bus) => break Monitor::new(bus, shunt(&cfg)),
                Err(e) => {
                    eprintln!("[battery] i2c-{BUS} @0x{ADDRESS:02X}: {e}");
                    tokio::time::sleep(RETRY).await;
                }
            }
        };
        if !monitor.identifies().unwrap_or(false) {
            eprintln!("[battery] no INA228 at 0x{ADDRESS:02X} on i2c-{BUS}");
            return;
        }
        if let Err(e) = monitor.configure() {
            eprintln!("[battery] calibration failed: {e}");
            return;
        }
        println!("[battery] INA228 on i2c-{BUS} @0x{ADDRESS:02X}");

        let mut tracker = Tracker::new();
        loop {
            tokio::time::sleep(SAMPLE).await;
            let cfg = core.hub.config();
            let reading = match monitor.read() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[battery] read failed: {e}");
                    tokio::time::sleep(RETRY).await;
                    continue;
                }
            };
            let reading = match tracker.observe(&cfg, &reading) {
                // At full the counter is zeroed, so what follows is the charge
                // taken out of a known-full pack.
                Step::Reset => {
                    let _ = monitor.reset_accumulators();
                    println!(
                        "[battery] pack full at {:.1} V, counter reset",
                        reading.bus_v
                    );
                    Reading {
                        charge_c: 0.0,
                        ..reading
                    }
                }
                Step::Publish => reading,
            };
            let counted = tracker.counted(&reading);
            if let Some(patch) = patch(&cfg, &reading, counted) {
                let _ = inside.send(patch);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scooter() -> Config {
        let mut cfg = Config::default();
        cfg.car_type = Some(CarType::Scooter);
        cfg.scooter_battery_voltage = 60;
        cfg.scooter_battery_capacity_ah = 30.0;
        cfg
    }

    fn reading(charge_c: f64, bus_v: f64, current_a: f64) -> Reading {
        Reading {
            bus_v,
            current_a,
            charge_c,
            ..Reading::default()
        }
    }

    #[test]
    fn half_a_pack_becomes_fuel_and_energy() {
        let cfg = scooter();
        // 54 000 C taken out of a full 30 Ah pack is half of it.
        let p = patch(&cfg, &reading(54_000.0, 60.0, 5.0), true).unwrap();
        assert_eq!(p["fuelPct"], json!(50.0));
        assert_eq!(p["batteryLevelKwh"], json!(0.9));
        assert_eq!(p["batteryCapacityKwh"], json!(1.8));
        assert_eq!(p["packVoltage"], json!(60.0));
        assert_eq!(p["packCurrent"], json!(5.0));
    }

    #[test]
    fn a_freshly_reset_pack_reads_full() {
        let cfg = scooter();
        let p = patch(&cfg, &reading(0.0, 67.2, 0.0), true).unwrap();
        assert_eq!(p["fuelPct"], json!(100.0));
        assert_eq!(p["batteryLevelKwh"], json!(1.8));
    }

    #[test]
    fn a_car_gets_no_pack_reading_from_the_sensor() {
        let mut cfg = scooter();
        cfg.car_type = Some(CarType::Electric);
        assert!(patch(&cfg, &reading(54_000.0, 60.0, 5.0), true).is_none());
    }

    #[test]
    fn without_a_reference_nothing_claims_a_percentage() {
        let cfg = scooter();
        // A pack that was never seen full: the counter is still at its reset
        // value, so a percentage would be meaningless.
        let p = patch(&cfg, &reading(0.0, 12.4, 0.0), false).unwrap();
        assert!(p.get("fuelPct").is_none(), "no charge without a reference");
        assert!(p.get("batteryLevelKwh").is_none());
        assert_eq!(p["packVoltage"], json!(12.4));
        assert_eq!(p["packCurrent"], json!(0.0));
    }

    #[test]
    fn the_percentage_stays_inside_the_pack() {
        let cfg = scooter();
        let over = patch(&cfg, &reading(-200_000.0, 67.2, 0.0), true).unwrap();
        assert_eq!(over["fuelPct"], json!(100.0), "charged past full clamps");
        let under = patch(&cfg, &reading(500_000.0, 48.0, 0.0), true).unwrap();
        assert_eq!(under["fuelPct"], json!(0.0), "drained past empty clamps");
    }

    #[test]
    fn full_is_decided_on_voltage_and_current() {
        let cfg = scooter();
        assert!(at_full(&cfg, &reading(0.0, 67.2, 0.0)));
        assert!(!at_full(&cfg, &reading(0.0, 67.2, 3.0)), "still charging");
        assert!(!at_full(&cfg, &reading(0.0, 66.0, 0.0)), "not there yet");
    }

    #[test]
    fn the_fl2_shunt_is_scaled_from_the_settings() {
        let cfg = scooter();
        let s = shunt(&cfg);
        // FL-2 marked 100 A / 75 mV is 0.75 mΩ.
        assert!((s.ohms - 0.00075).abs() < 1e-12);
        assert!((s.max_current_a - 100.0).abs() < 1e-12);
        // 100 A spread over 2^19 steps is 190.7 µA per bit.
        assert!((s.current_lsb() - 100.0 / 524_288.0).abs() < 1e-12);
        // SHUNT_CAL has to stay inside 16 bits.
        let cal = s.shunt_cal();
        assert!(cal > 0 && cal < 32_000, "SHUNT_CAL was {cal}");
    }

    #[test]
    fn the_shunt_voltage_at_full_scale_fits_the_wide_range() {
        for (milliohm, amps) in [(0.75, 100.0), (10.0, 10.0), (0.5, 50.0)] {
            let mut cfg = scooter();
            cfg.scooter_shunt_milliohm = milliohm;
            cfg.scooter_shunt_max_current_a = amps;
            let mv = shunt(&cfg).ohms * amps * 1000.0;
            // The INA228 reads ±163.84 mV in its wide range.
            assert!(mv < 163.84, "{mv} mV would clip the input");
        }
    }

    #[test]
    fn a_bus_scale_corrects_a_board_that_reports_high() {
        let mut cfg = scooter();
        // This module reads 18.49 V where the meter at its pin shows 12.3 V.
        let raw = reading(0.0, 18.489, 0.0);
        cfg.scooter_vbus_scale = 12.3 / 18.489;
        let p = patch(&cfg, &raw, false).unwrap();
        assert_eq!(p["packVoltage"], json!(12.3));
        // The default factor of 1.0 leaves the reading alone.
        cfg.scooter_vbus_scale = 1.0;
        let p = patch(&cfg, &raw, false).unwrap();
        assert_eq!(p["packVoltage"], json!(18.49));
    }

    #[test]
    fn the_full_check_uses_the_corrected_voltage() {
        let mut cfg = scooter();
        cfg.scooter_battery_voltage = 12;
        // Uncorrected this looks far above the pack and must not count as full.
        let raw = reading(0.0, 18.0, 0.0);
        assert!(!at_full(&cfg, &raw));
        // With the scale it lands on the pack's full voltage.
        cfg.scooter_vbus_scale = 12.6 / 18.0;
        assert!(at_full(&cfg, &raw));
    }

    #[test]
    fn a_pack_that_was_never_full_claims_no_percentage() {
        let cfg = scooter();
        let mut t = Tracker::new();
        let r = reading(0.0, 62.0, 2.0);
        assert_eq!(t.observe(&cfg, &r), Step::Publish);
        assert!(!t.counted(&r), "a zero counter is not a reference");
        let p = patch(&cfg, &r, t.counted(&r)).unwrap();
        assert!(p.get("fuelPct").is_none());
    }

    #[test]
    fn the_percentage_keeps_coming_after_the_pack_leaves_full() {
        // The gauge used to freeze here: leaving the full window dropped the
        // reference, so no more percentages were published and the dashboard
        // kept showing whatever it had last received.
        let cfg = scooter();
        let mut t = Tracker::new();
        assert_eq!(t.observe(&cfg, &reading(0.0, 67.2, 0.0)), Step::Reset);
        let discharged = reading(5_400.0, 66.0, 2.0);
        assert_eq!(t.observe(&cfg, &discharged), Step::Publish);
        assert!(t.counted(&discharged), "still counted after leaving full");
        let p = patch(&cfg, &discharged, t.counted(&discharged)).unwrap();
        assert_eq!(p["fuelPct"], json!(95.0), "5 % of 30 Ah taken out");
    }

    #[test]
    fn a_counter_left_running_before_a_restart_still_counts() {
        // The chip keeps accumulating across a restart, so a non-zero counter is
        // its own reference even though this run never saw the pack full.
        let cfg = scooter();
        let mut t = Tracker::new();
        let r = reading(20_000.0, 64.0, 3.0);
        assert_eq!(t.observe(&cfg, &r), Step::Publish);
        assert!(t.counted(&r));
    }

    #[test]
    fn a_full_pack_resets_once_per_charge() {
        let cfg = scooter();
        let mut t = Tracker::new();
        let full = reading(0.0, 67.2, 0.0);
        assert_eq!(t.observe(&cfg, &full), Step::Reset);
        assert_eq!(t.observe(&cfg, &full), Step::Publish, "already reset");
        assert_eq!(t.observe(&cfg, &reading(0.0, 60.0, 4.0)), Step::Publish);
        assert_eq!(t.observe(&cfg, &full), Step::Reset, "charged again");
    }

    #[test]
    fn a_coulomb_of_noise_is_not_a_reference() {
        let t = Tracker::new();
        assert!(!t.counted(&reading(0.4, 64.0, 0.0)));
        assert!(!t.counted(&reading(-0.4, 64.0, 0.0)));
    }

    #[test]
    fn a_small_pack_keeps_its_watt_hours() {
        // The 12 V test pack holds 55 Wh. Hundredths of a kWh would put the
        // readout on 10 Wh steps and make the settings percentage wrong.
        let mut cfg = scooter();
        cfg.scooter_battery_voltage = 12;
        cfg.scooter_battery_capacity_ah = 4.6;
        let p = patch(&cfg, &reading(0.0, 12.0, 0.0), true).unwrap();
        assert_eq!(p["batteryCapacityKwh"], json!(0.055));
        assert_eq!(p["batteryLevelKwh"], json!(0.055));
        let half = patch(&cfg, &reading(8_280.0, 12.0, 1.0), true).unwrap();
        assert_eq!(half["fuelPct"], json!(50.0));
        assert_eq!(half["batteryLevelKwh"], json!(0.028), "27.6 Wh rounded");
    }

    #[test]
    fn a_broken_scale_is_ignored() {
        let mut cfg = scooter();
        for broken in [0.0, -1.0, f64::NAN] {
            cfg.scooter_vbus_scale = broken;
            let p = patch(&cfg, &reading(0.0, 60.0, 0.0), false).unwrap();
            assert_eq!(p["packVoltage"], json!(60.0), "scale {broken} ignored");
        }
    }
}
