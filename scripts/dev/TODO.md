# Open work on the pack monitor

Where the INA228 bring-up stands, and what is left. The wiring is in
[`../../native/crates/livi-ina228/README.md`](../../native/crates/livi-ina228/README.md),
the settings in [README.md](README.md).

## Verified so far

On a 12 V bench pack with an FL-2 shunt and a load attached:

- the chip answers at `0x40`, `0x3E` reads `0x5449` and `0x3F` reads `0x2281`
- `SHUNT_CAL` is written as 1875, which is `13107.2e6 × (100 A / 2^19) × 0.75 mΩ`
- the stack works end to end: the counter resets on a full pack, and `fuelPct`
  and `batteryLevelKwh` reach the telemetry and the dashboard
- the percentage tracks the counter while the pack discharges. It used to freeze
  at the last value published before the pack left its full-voltage window,
  which showed 95 % at 11.5 V; the reference is now kept for the whole discharge

## Not verified

**The bus voltage reads 1.5× high.** The meter at the `VBUS` pin shows 12.3 V
where the chip reports 18.49 V, stably across samples, with the ground verified
at 0 V against the pack's negative. A gain error that large is far outside the
INA228 specification, so either the board divides `VBUS` deliberately or the
chip is a relabelled clone.

`VbusScale` is set to `0.6653` on the device to make the reading match the meter.
It lives in that device's `config.json` only, not in the repository: it is a
measurement of one board, not a project default.

**The current is not verified against a reference.** Only the internal
consistency is proven. A clone with a wrong LSB would still look consistent.

One sample pair, taken under a load that switches between about 0.6 A and 0.9 A,
read 0.5544 mV of shunt voltage and 0.607 A of current — 0.739 A if the shunt is
taken at its word. Two conversions about 1 ms apart can differ that much on a
switching load, so this is a hint rather than proof. What does not fit is the
`POWER` register: 0.42 W where the voltage and current in the same burst say
7 W. `ENERGY` on the other hand integrates to something plausible for the time
the pack has been running, so the power scale deserves a look of its own.

## Items

| # | Item | State | Waits for |
| --- | --- | --- | --- |
| 1 | Compare the current against a reference: a load of known wattage, or a meter in series | blocked, no reference load available | — |
| 2 | Check `VbusScale` at a second voltage (24 V or 60 V). Constant factor means a linear divider and one factor suffices; a changing factor needs a curve instead | pending | 1 |
| 3 | Harden the counter reset: confirm the automatic one, add a manual "Reset Coulomb Counter" entry, record the timestamp and the voltage at that moment | pending | 1 |
| 4 | Move the sensor settings into the menu: `scooterBatterySensor`, shunt mΩ, shunt amps and `VbusScale` are only editable in `config.json` today | pending | — |
| 5 | ~~The core's log lines never reach the journal~~ — they do. `[battery]`, `[inputs]` and `[core]` all appear under the `AppRun` unit tag. What is *not* in the journal is the compositor: it writes its own log to `~/.config/LIVI/log/compositor.log`, which is where its input and plane decisions are | resolved | — |
| 6 | Decide whether the die temperature (register `0x06`) belongs in the telemetry or only shows the chip heating itself | pending | — |
| 7 | Move to the built-in 60 V pack: set Scooter, 60 V, 30 Ah, determine `VbusScale` at that voltage, enable the sensor, and wait for one full charge to give the counter its reference | pending | 2, 4 |
| 8 | Build the four channel level shifter for the indicator and high beam lines and verify each one against the harness; the pin assignment is in [GPIO-INPUTS.md](GPIO-INPUTS.md) | pending | — |
| 9 | Work out the `POWER` scale: the register reads 16× below what its own voltage and current readings multiply to, while `ENERGY` looks right. Nothing on the dashboard uses either yet | pending | 1 |
| 10 | Remove the `DEV: Fake Map Background` switch, its config key and the embedded image before a release. It is a development aid and must not ship | pending | — |

Item 5 is closed, and it is worth remembering how: the missing logs were never
missing, only looked for in the wrong place. Before the next search for a fault,
read both sources — `journalctl -u livi-kiosk` for the core and the helper,
`~/.config/LIVI/log/compositor.log` for the compositor. `LIVI_WLR_DEBUG=1` in the
unit's environment raises the compositor to `debug`, which is what turned the
dead touch screen into `touch down ... surface=yes`.
