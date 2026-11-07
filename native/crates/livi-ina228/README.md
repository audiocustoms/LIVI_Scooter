# Wiring an INA228 pack monitor

A scooter reports no pack telemetry of its own, so LIVI measures it: an INA228
sits across a shunt in the pack's negative lead, and this crate turns its
registers into a state of charge.

This page is the hardware side: which pin goes where, and what to measure before
switching anything on. The settings that go with it are in
[`scripts/dev/README.md`](../../../scripts/dev/README.md).

## The module

The reference board is an Adafruit 5832 style INA228 breakout, the one marked
`B2QF` with a `R015` shunt. Other boards work the same way as long as the shunt
is external.

```
        ┌─────────────────────────┐
        │  Vin−         Vin+      │   screw terminals, thick current path
        │   ⊙────⊙────⊙           │
        │        └── R015 ──┘     │   on-board shunt, removing it separates the nets
        │                         │
        │  [INA228]        [103]  │
        │                         │
        │  VIN  SCL  VBUS  VIN+   │   header, top row
        │  GND  SDA  VIN−  ALRT   │   header, bottom row
        └─────────────────────────┘
```

`VIN+` and `VIN-` each appear twice: once on the screw terminal, once on the
header. **They are the same two nets.** The screw terminal carries the current,
the header is for tapping the same node. Either can be used, not both.

The default I²C address is `0x40`, which is what [`ADDRESS`](../livi-core/src/battery.rs)
in the core expects.

## Pins

| Module | Raspberry Pi 5 | Note |
| --- | --- | --- |
| `VIN` | pin 1, 3V3 | **logic supply only, 3–5 V** |
| `GND` | pin 6, GND | common with the pack's negative |
| `SDA` | pin 3, GPIO2 | 10 kΩ pull-ups are on the board |
| `SCL` | pin 5, GPIO3 | |
| `ALRT` | — | unused, the driver polls |
| `VBUS` | pack positive | up to 85 V against the module's GND |
| `VIN+` | shunt, load side | |
| `VIN-` | shunt, pack side | this node is the system ground |

`VIN` is **not** the pack. Feeding 12 V into `VIN` destroys the module; the
measurement goes through `VIN+`, `VIN-` and `VBUS` only.

## Low-side wiring

The shunt sits in the negative lead. Because `R015` is removed, the module does
not conduct between its two terminals: the external shunt carries the current
and the module only senses the drop across it.

```
Batterie +  ──────────────────────────────────  Last +
Batterie −  ──[ FL-2 ]──[ FL-2 ]──────────────  Last −   (= system ground)
                   │         │
              VIN− │         │ VIN+
                   │         │
VBUS ──────────────┼─────────┼────────────────  Batterie +
```

Concretely:

1. Pack negative to the first current terminal of the shunt.
2. Second current terminal of the shunt to the load's negative terminal. This
   node is the system ground the Pi, the module and the load all share.
3. `VIN-` to the pack side of the shunt.
4. `VIN+` to the load side of the shunt.
5. `VBUS` to the pack positive.
6. `VIN` and `GND` to the Pi, `SDA`/`SCL` to the header pins.

Take the sense terminals of the shunt for steps 3 and 4, not the thick current
terminals. At 0.75 mΩ a few tenths of a milliohm of screw contact are already
20 % error.

Point the shunt the right way round: `VIN-` to the pack side and `VIN+` to the
load side. Only that way does a discharging pack read a positive current, which
is what [`Reading::soc_percent`](src/lib.rs) counts against.

## Before switching on

Check these with a multimeter on a dead circuit. They take a minute and catch
every mistake that would otherwise look like a software bug.

1. **Are both `VIN+` pins one node, and both `VIN-` pins one node?** Expect
   0 Ω between the screw terminal and the matching header pin.
2. **Are `VIN+` and `VIN-` apart?** Expect no continuity. Any continuity means
   the board's shunt is still fitted and the reading would be wrong.
3. **Where does `VBUS` go on the board?** Measure it against `VIN+` and `VIN-`.
   Some boards tie it to one of them. If yours does, `VBUS` cannot read the pack
   from the load side and has to be wired to the side that carries pack
   potential — or the driver has to add the shunt drop to the reading.
4. **Is `VIN` free of the pack?** Continuity between `VIN` and `VIN+` or `VBUS`
   would mean the board feeds logic from the measurement side; do not apply pack
   voltage before sorting that out.

## Settings

The monitor is scaled entirely from the car settings, so no code knows which
pack is fitted:

| Setting | Default | Meaning |
| --- | --- | --- |
| `scooterBatteryVoltage` | 60 V | full and empty limits follow from it |
| `scooterBatteryCapacityAh` | 30 Ah | scale for the coulomb counter |
| `scooterShuntMilliohm` | 0.75 mΩ | an FL-2 marked 100 A / 75 mV |
| `scooterShuntMaxCurrentA` | 100 A | sets `CURRENT_LSB` and `SHUNT_CAL` |
| `scooterVbusScale` | 1.0 | factor applied to the bus voltage reading |
| `scooterBatterySensor` | off | reads the pack over I²C |

`SHUNT_CAL` comes out as `13107.2e6 × (I_max / 2^19) × R`, which is 1875 for the
FL-2. The bus itself is brought up with
[`scripts/dev/livi-pi-i2c.sh`](../../../scripts/dev/livi-pi-i2c.sh).

## When something is wrong

| Symptom | Likely cause |
| --- | --- |
| always 100 %, never falls while riding | `VIN+` and `VIN-` swapped |
| voltage near zero | `VBUS` on the load side, or the shunt is still fitted |
| voltage reads high by a fixed factor | board scales `VBUS`; set `scooterVbusScale` to meter ÷ reading |
| no chip at `0x40` | I²C not enabled, or `SDA`/`SCL` swapped |
| current reads far too high or low | wrong `scooterShuntMilliohm` |

The counter is reset when the pack reaches its full voltage with the current
settled, so a fresh reset reads 100 % by definition. Charge left is then the
capacity minus what the counter has accumulated since.

That reference outlives the moment it was taken: it holds for the whole
discharge, not just while the pack sits at its full voltage. It also outlives a
restart of the core, because a counter that still holds a value is its own
evidence that it was zeroed on a full pack — the chip keeps accumulating while
LIVI is not looking. Only a pack that has never been full reads zero, and that
one gets no percentage at all rather than a made-up one.
