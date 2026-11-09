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

## The "1.5× bus voltage" was a measurement error

An earlier note here claimed the chip reports 1.5× the voltage at its `VBUS` pin
and that the board therefore divides the input. That was wrong, and the cost was
real: `scooterVbusScale` was set to `0.6653` on the Pi 5, which made the
dashboard show a third less than the pack actually had.

The chip was right all along. The probe script used **312.5 µV** per `VBUS` bit,
which is the `VSHUNT` LSB on the wide range, not the `VBUS` one. The correct
value, per the datasheet and TI's own driver, is **195.3125 µV** — which is what
`livi-ina228` uses. 312.5 / 195.3125 = 1.6, which is the factor that looked like
a hardware fault.

Measured against a meter on the same board afterwards: the chip read 10.72 V
where the meter showed 11.3 V. That is a 5 % deviation, small enough to be wiring
and divider tolerance rather than a defect, and corrected per device with
`scooterVbusScale` (1.0541 on that board).

**Rule for the next probe:** read `VBUS` with 195.3125 µV per bit, and never
reach into the chip while `livi-core` is reading it. A register select and the
read are two separate I²C transfers, so a second master between them moves the
pointer. That is what produced the oscillating percentages and the implausible
`POWER` reading recorded further down — not the chip.

## Not verified

**The current is not verified against a reference.** Only the internal
consistency is proven. A clone with a wrong LSB would still look consistent.

Two earlier sample pairs were taken while a probe was also talking to the chip,
so their disagreement (0.5544 mV of shunt voltage against 0.607 A, and a `POWER`
register 16× below the product of voltage and current) says more about the shared
bus than about the sensor. Repeat them without a second reader before drawing a
conclusion.

## Items

| # | Item | State | Waits for |
| --- | --- | --- | --- |
| 1 | Compare the current against a reference: a load of known wattage, or a meter in series | blocked, no reference load available | — |
| 2 | Check `VbusScale` at a second voltage (24 V or 60 V). Constant factor means a linear divider and one factor suffices; a changing factor needs a curve instead | pending | 1 |
| 2b | Correct the Pi 5's `scooterVbusScale`, which is still `0.6653` from the measurement error above and makes it read a third low. Should be re-measured against a meter, roughly `1.0` | pending | — |
| 3 | Harden the counter reset: confirm the automatic one, add a manual "Reset Coulomb Counter" entry, record the timestamp and the voltage at that moment | pending | 1 |
| 4 | Move the sensor settings into the menu: `scooterBatterySensor`, shunt mΩ, shunt amps and `VbusScale` are only editable in `config.json` today | pending | — |
| 5 | ~~The core's log lines never reach the journal~~ — they do. `[battery]`, `[inputs]` and `[core]` all appear under the `AppRun` unit tag. What is *not* in the journal is the compositor: it writes its own log to `~/.config/LIVI/log/compositor.log`, which is where its input and plane decisions are | resolved | — |
| 6 | Decide whether the die temperature (register `0x06`) belongs in the telemetry or only shows the chip heating itself | pending | — |
| 7 | Move to the built-in 60 V pack: set Scooter, 60 V, 30 Ah, determine `VbusScale` at that voltage, enable the sensor, and wait for one full charge to give the counter its reference | pending | 2, 4 |
| 8 | Build the four channel level shifter for the indicator and high beam lines and verify each one against the harness; the pin assignment is in [GPIO-INPUTS.md](GPIO-INPUTS.md) | pending | — |
| 9 | Work out the `POWER` scale: the register read 16× below what its own voltage and current multiplied to — but that was measured while a probe shared the bus, so re-measure before believing it. Nothing on the dashboard uses either register yet | pending | 1 |
| 10 | Remove the `DEV: Fake Map Background` switch, its config key and the embedded image before a release. It is a development aid and must not ship | pending | — |
| 11 | Decode the one-wire tacho link to get a real speed, the selected drive level (1-3) and the odometer. Until then the dashboard shows `03551km` and level 2 as fixed stand-ins — both come from `Readings` in `livi-ui-model` | blocked, no capture yet | — |

Item 5 is closed, and it is worth remembering how: the missing logs were never
missing, only looked for in the wrong place. Before the next search for a fault,
read both sources — `journalctl -u livi-kiosk` for the core and the helper,
`~/.config/LIVI/log/compositor.log` for the compositor. `LIVI_WLR_DEBUG=1` in the
unit's environment raises the compositor to `debug`, which is what turned the
dead touch screen into `touch down ... surface=yes`.
