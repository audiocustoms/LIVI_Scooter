# Indicator and high beam inputs

The original control unit drives the instrument cluster with 12 V: a steady
level while an indicator is on, whether left or right, and another while the high
beam is on. The dashboard lamps already exist and read two telemetry keys, so all
that is missing is someone to tell them. This is that someone.

A GPIO cannot take 12 V, so each signal arrives through a level shifter and is
read as a plain high or low.

## The four lines

| Signal | GPIO | Header pin | Config key |
| --- | --- | --- | --- |
| Indicator left | 5 | 29 | `scooterIndicatorLeftGpio` |
| Indicator right | 6 | 31 | `scooterIndicatorRightGpio` |
| High beam | 13 | 33 | `scooterHighBeamGpio` |
| Spare | 19 | 35 | `scooterIndicatorSpareGpio` |

Four neighbours on one side of the 40 pin header, clear of I²C on GPIO 2/3 and
the UART on 14/15, so a four channel shifter board sits next to the Pi without
crossing other wiring. `-1` in any of the four keys disables the reader.

The inputs are claimed with a pull-down, so an unwired or unpowered line reads as
off rather than floating.

## What the level shifter has to do

The Pi side must never see more than 3.3 V, and a vehicle's 12 V rail is not a
clean 12 V: load dumps and inductive spikes push it far higher for a moment.
Three ways, in order of what I would recommend:

1. **Optocoupler** (e.g. PC817 with 1 kΩ in series to the 12 V side, 10 kΩ
   pull-down on the Pi side). Galvanically separates the vehicle from the Pi,
   which also stops ground offsets from becoming a problem. Careful with the
   output stage, see [Ready-made modules](#ready-made-modules).
2. **Resistor divider with clamping** (10 kΩ / 3.3 kΩ, plus a 3.3 V zener or a
   TVS across the Pi side). Cheaper, but the divider output impedance has to stay
   low enough for the Pi's pull-down not to drag it around.
3. **A logic level converter module** only if it is explicitly rated for 12 V on
   its high side. The common BSS138 boards are made for 5 V ↔ 3.3 V and are not
   rated for 12 V, let alone for spikes.

Assumption: the control unit switches **12 V** onto the wire. If it switches the
wire to ground instead, the levels are inverted and either the hardware or the
`high`/`low` reading has to be flipped.

## Ready-made modules

Four channels, one part. The four families worth looking at, commonest first:

| Family | Typical listing | Output when the 12 V is on | Verdict |
| --- | --- | --- | --- |
| 4-channel optocoupler board, PC817/EL817 | "4 channel optocoupler isolation board 12 V", AZDelivery / ARCELI / HiLetgo / NOYITO, ~5–8 € | **low** (open collector sinks to ground) | Cheapest isolated option, but needs an inverter |
| Same board with selectable NPN/PNP output | "optocoupler module NPN PNP selectable 4 channel", ~10–15 € | high | Drop-in, matches the reader |
| 4-channel 12 V relay board with opto inputs | "4-Kanal Relais Modul 12 V Optokoppler", Elegoo / AZDelivery, ~8 € | high (contact to 3.3 V) | Bulky and mechanical, but nothing to get wrong |
| Four resistor dividers | stripboard, ~1 € total | high | No isolation, but non-inverting and tiny |

### Optocoupler board

The cheapest boards have one gotcha each, and both are worth knowing before the
order is placed.

**The input resistor is usually sized for 5 V.** A board advertised as
3.3–5 V has around 1 kΩ per channel. On 12 V that gives roughly 10 mA, which
works but runs the LED at double its intended current. Rather than desoldering,
put a resistor in series with each input terminal: 1.8 kΩ brings a 1 kΩ board to
about 4 mA on 12 V, which is where a PC817 likes to sit and keeps the control
unit's output stage happy. Boards sold as 12 V already have this, some as
12–24 V.

**The output pulls to ground.** The output transistor's emitter sits on the
board's ground and the collector on the output pin, usually with a pull-up to the
board's output VCC. So 12 V present means the output is pulled **low**, the
opposite of what the reader expects. Two ways out:

- Buy the board with selectable NPN/PNP output, or one that brings both collector
  and emitter out per channel so the emitter can drive the GPIO up through a 10 kΩ
  pull-down.
- Keep the cheap board and put a **74AHCT14** (or 74HCT14) between board and Pi,
  powered from 3.3 V, four of its six gates used. It inverts back and its Schmitt
  inputs swallow the noise a vehicle 12 V line carries, which the bare GPIO does
  not.

Either way the output side of the board must be powered from **3.3 V, not 5 V** —
most boards pull their output up to their own VCC, and 5 V on a GPIO is not
survivable.

### Relay board

The lazy option that just works: coil on the 12 V signal, `COM` to 3.3 V, `NO` to
the GPIO, pull-down on the Pi side already enabled. Contacts do not care about
polarity, spikes or ground offsets, and the coil draws 35–70 mA — the same order
as the bulb the instrument cluster used to light, so the control unit will not
notice. Four coils at once (hazard) is a few hundred milliamps, worth checking
against the control unit's rating.

Downside is size, a click per indicator period on some boards, and contacts that
will eventually pit. For four steady levels it is still more robust than it
sounds.

### Resistor dividers

If the 12 V converter and the Pi share a ground anyway — on a scooter they do —
isolation buys less than it looks. 10 kΩ in series and 3.3 kΩ to ground puts
12 V at 2.99 V and 16 V at 3.99 V, comfortably above the Pi's input threshold;
a BZX84C3V3 from the GPIO to ground clamps whatever comes past that. Non-
inverting, needs no supply, and one 100 nF across the zener is enough to keep
ignition noise out. The cost is that a fault in the divider puts 12 V on the GPIO,
so use two resistors in series for the upper leg rather than one.

### Not suitable

- **BSS138 level shifter boards** ("4-channel logic level converter 5 V ↔ 3.3 V").
  Their high side is rated for 5 V, not 12 V, and they need the signal to be
  pulled up on both sides to work at all.
- **"Voltage sensor" modules** (the 0–25 V Arduino ones). They are a divider with
  no clamp and expect a 5 V ADC, not a 3.3 V GPIO.
- **Bare PC817 optocouplers.** Fine electrically, but you end up adding the input
  resistor, the output resistor and the connector yourself, at which point the
  ready-made board was cheaper.

### Before ordering

1. Is a 12 V input variant offered, or does a series resistor have to be added?
2. How much current per channel at 12 V? Under 10 mA keeps it comparable to an
   LED indicator lamp; a relay coil is the exception and is fine.
3. Does the output go **high** when the input is on, or does it sink to ground?
4. Screw terminals, not Dupont headers. A vehicle shakes those loose.
5. Is the output side's VCC configurable, so it can be run at 3.3 V?
6. Keep the board's milled isolation slot between the 12 V and 3.3 V halves. Do
   not run both sides through the same cable bundle if it can be avoided.

If the only board that turns up inverts its output, the reader currently has no
per-line way to say so — the levels would have to be flipped in hardware, or a
`scooterIndicatorInvert` style flag added to the config.

## Testing it without the control unit

A 3.3 V source and a jumper are enough to prove the whole chain. Pin 17 carries
3.3 V, pin 29 is GPIO 5 (indicator left):

| Jumper | Expected |
| --- | --- |
| pin 17 → pin 29 | left arrow at full brightness |
| pin 17 → pin 31 | right arrow at full brightness |
| pin 17 → pin 33 | high beam at full brightness |
| pin 17 → pin 29 **and** 31 | both arrows blink, hazard lamp lights |

Pulling the jumper leaves the pull-down in charge, so the symbol falls back to
its dim state within one poll interval. The lines are claimed as inputs with a
pull-down, so nothing has to be unloaded between tests.

Low beam is **not** one of these inputs. The scooter harness was wired for four
lines, and the low beam lamp on the dashboard therefore stays dim — that is not a
fault.

## What the dashboard does with it

| Lines | Telemetry | Dashboard |
| --- | --- | --- |
| left only | `turnSignal: "left"` | left arrow blinks |
| right only | `turnSignal: "right"` | right arrow blinks |
| both | `hazards: true`, `turnSignal: "none"` | hazard lamp blinks |
| high beam | `highBeam: true` | blue high beam lamp |

The control unit delivers a **steady** level while an indicator is active; the
blinking is done by the UI, so nothing here has to count flashes.

Every one of these symbols is on screen all the time, on a translucent plate of
the same tone the charge readout uses. An unlit symbol stays at 30 % opacity and
a lit one goes to full, so the place is known before it lights up; a blinking
symbol dims between those two states rather than disappearing, which is far
easier to read at speed than something appearing out of nothing twice a second.

Both sides high is what a hazard flasher looks like from outside, so it is
reported as hazards rather than as two directions at once.

## Limits

The reader runs only when the car type is Scooter. While it runs, it owns those
telemetry keys, so a CarPlay or Android Auto session that also reports turn
signals will be overwritten. On a scooter without projection that is the intended
behaviour; with projection the phone's own indicator state is the better source
and the inputs should be disabled with `-1`.
