# The M1S controller to cluster link

Research notes, 2026-11-07. No bench work yet. Every claim below is tagged
either **[measured]** (we have the number) or **[inferred]** (derived from the
industry standard this controller family follows, or from arithmetic). Nothing
here has been confirmed with a logic analyser.

## Where this stands

The scooter's DC motor controller (M1S, Zhejiang Taizhou Chifeng Electric
Technology) drives the instrument cluster over **one signal wire plus ground**.
That single wire carries the speed, the selected drive level (1-2-3) and the
odometer.

**[measured]** Multimeter, signal wire against ground, DC: the voltage sits
between **1.4 V and 1.8 V and wanders**.

That is not what a tachometer pulse looks like, and the reason is not the
voltage but the payload. The conclusion so far: the link is a **single-wire
serial telegram**, in the "一线通" (yīxiàntōng, single-wire) family that Chinese
e-scooter and e-bike controllers use, standardised as **T/JSEBA 002—2022
Annex A**. It is *not* UART framing in the 8N1 sense, and it is *not* Dallas
1-Wire.

| Question | Answer | Basis |
| --- | --- | --- |
| How many wires? | 1 signal + GND | measured / stated |
| Direction | controller → cluster only | stated |
| Payload | speed, drive level 1-2-3, odometer | stated |
| Framing | 2 ms PWM-coded bits, sync pulse, sum checksum | inferred |
| Effective bit rate | 500 bit/s | inferred |
| Frame period | ≈ 260 ms (≈ 3.8 frames/s) | inferred |
| Logic level | 5 V more likely than 3.3 V | inferred, see below |

## Why it has to be a telegram

A tacho pulse carries exactly one number: frequency, and therefore speed. It
cannot carry a drive level, and an odometer needs accumulated state. Three
independent quantities on one wire means the wire carries *data*, not a duty
cycle.

The obvious loophole — "frequency = speed, duty cycle = gear" — fails at
standstill, where there is no frequency to measure, and it is not something
this controller class does. The discriminator is cheap, see [Next
steps](#next-steps) T2: a telegram produces **periodic bursts at a fixed
raster regardless of wheel speed**; a pulse train produces **evenly spaced
pulses whose rate tracks the wheel**.

## What 1.4-1.8 V means

A multimeter integrates. It reports the mean of a PWM waveform, not the logic
level. Using the T/JSEBA Annex A timings with a 12-byte payload:

- 96 data bits × 2 ms = 192 ms
- sync: LOW 10 ms + HIGH 1 ms
- stop: LOW 5 ms, idle: LOW ≥ 50 ms
- one bit is HIGH for 1.5 ms (logical 1) or 0.5 ms (logical 0)

With *p* = fraction of one-bits, the HIGH time per frame is
`1 + 96 · (0.5 + p)` ms over a `258 ms` period:

| one-bits *p* | duty | mean at 5 V | mean at 3.3 V |
| --- | --- | --- | --- |
| 0.20 | 26 % | 1.32 V | 0.87 V |
| 0.30 | 30 % | 1.51 V | 1.00 V |
| 0.42 | 35 % | 1.73 V | 1.14 V |
| 0.50 | 38 % | 1.88 V | 1.24 V |

So **1.4-1.8 V at 5 V logic matches a plausible telegram** comfortably, with
*p* ≈ 0.22-0.44 — about the mix of zero and one bits real data produces.

Two things this arithmetic *does* establish, and one it does not:

- **[inferred] A 50 % square wave is excluded.** 50 % of 5 V is 2.5 V; we see
  1.4-1.8 V. Even a pulse train at 30 % duty would have to be stationary in
  frequency versus speed *and* would drop to 0 V at standstill — worth
  checking, but see T2.
- **[inferred] The line idles LOW.** If the cluster's pull-up held the wire
  HIGH through the ≥ 50 ms inter-frame gap, the same framing would read
  ≈ 2.3 V, not 1.8 V. So the sender holds the wire down between frames, or the
  gap is much shorter than the standard's 50 ms.
- **Not established: 5 V vs 3.3 V.** To reach 1.4-1.8 V on 3.3 V logic the
  payload would need *p* ≈ 0.63-0.95, i.e. almost no zero bits. Since the
  odometer bytes are small numbers and therefore zero-heavy, that is unlikely
  — 5 V is the better hypothesis, but it is a hypothesis. T1 settles it.

**[measured] The wandering is expected**, not noise: the mean shifts as the
payload content changes (drive level, odometer), and a handheld meter samples
over a window that is not an integer number of frames. Use min/max or a scope
before drawing conclusions from a single reading.

## Candidate protocols, ranked

| # | Candidate | Why it fits / fails | Discriminator |
| --- | --- | --- | --- |
| 1 | 一线通 / SIF, T/JSEBA 002—2022 Annex A | The de-facto standard for Chinese controllers with a one-wire cluster; matches the observed mean; covers speed + level + odometer in one telegram | 2 ms bits, 10-30 ms sync LOW, sum checksum |
| 2 | Half-duplex single-wire UART, 1200-2400 bps, 8N1, multi-byte frames (e.g. `0x55 0xAA` header) | Also seen in this market, especially where the same wire is used bidirectionally | Bit width 417 µs (2400 bps) or 833 µs (1200 bps), real start/stop bits, **no** 1-3 ms sync HIGH |
| 3 | Custom combined pulse scheme (f = speed, duty = level) | Would explain the mean, ignores the odometer | Pulse rate must scale with wheel speed |
| 4 | Dallas/Maxim 1-Wire, LIN, CAN | No reset-pulse slots; LIN idles recessive at 12 V (mean would read near 12 V); CAN needs two wires | excluded on inspection |

Candidates 1 and 2 differ only in bit timing, and both are decoded with the
same instrument, so the plan below covers both.

## Reference timings (SIF family)

Taken from the standard's Annex A, not from a vendor datasheet:
[T/JSEBA 002—2022](https://static1.tianyancha.com/czd_file/standard/20a57dd087553ac36f31a191890a9781.pdf)
([catalogue entry](https://www.spc.org.cn/online/d06613166c916ed10793cdacee580260.html)).

| Element | Value |
| --- | --- |
| Wire | 1 signal + GND, open drain, slave side pulls up to VCC |
| VCC | 5.0 V preferred; 3.3 V or 12 V allowed |
| Sync pulse | HIGH `T2` = 1-3 ms, LOW `T1` = 10 × `T2` (10-30 ms) |
| Data bit | 2 ms total, PWM coded by HIGH share |
| Logical 1 | HIGH 1.5 ms / LOW 0.5 ms, η = 75 % (accept 60-80 %) |
| Logical 0 | HIGH 0.5 ms / LOW 1.5 ms, η = 25 % (accept 20-40 %) |
| Stop | LOW 5-10 ms |
| Frame gap | ≥ 50 ms |
| Frame | `ID`, protocol version (hi/lo nibble), data, sum checksum |
| Checksum | `SUM = ID + version + Byte0 … Byte(Len-4)`, mod 256 |
| Length | public telegrams ID `0x01`, fixed 20 bytes; private IDs vendor defined, often 12 bytes |
| Bit order | LSB first per byte, multi-byte little endian (Intel) |
| Undefined bytes | filled with `0xFF` |

Vendor caveat: private telegram layouts and IDs are proprietary, and the
standard explicitly allows encrypted private frames. The *field positions*
(speed, level, odometer) must come from a capture, not from this table.

## Next steps

Ordered so each step is cheap and falsifies something. All deferred until the
bench is free.

### T1 — peak level and idle state
Peak detector (1N4148 + 100 nF + 1 MΩ) or scope. Also note the static level
while the controller is off, and during a frame gap.
**Settles:** 5 V vs 3.3 V logic; whether the line idles LOW (see above);
whether the cluster provides the pull-up.

### T2 — burst raster vs. pulse rate
Logic analyser ≥ 1 MS/s, ground referenced, on the wire. Capture 2 s with the
wheel still, then 2 s while spinning the wheel by hand.
**Settles:** telegram (fixed ≈ 260 ms bursts, content changes) vs. pulse train
(rate ∝ wheel speed). Also yields the frame period, which validates or
corrects the 12-byte/2 ms assumption.

### T3 — bit timing
From the same capture, histogram the HIGH and LOW run lengths. Expect clusters
at ≈ 0.5 ms / 1.5 ms (SIF), or ≈ 0.42 ms with start/stop bits (2400 bps UART).
**Settles:** candidate 1 vs 2, and the sync pulse shape.

### T4 — frame extraction and checksum
Decode to bytes, then test the sum rule. A valid checksum on every frame is the
strongest available confirmation that the protocol family is right.
**Settles:** whether the decode is correct at all — no checksum, no protocol.

### T5 — field mapping
Record frames at drive level 1/2/3, at several speeds, and with the odometer
changing (or a long ride).
**Expect:** one byte tracking the level 1:1; one or two bytes rising
monotonically with speed (scale likely 1 km/h or 0.1 km/h per LSB); two to
three bytes acting as a counter.
**Caveat:** the cluster may keep its own odometer in EEPROM from the speed
value. If no counter is in the payload, that is the answer, not a decoding
failure.

### T6 — wiring decision
Once the level is known (T1), pick the shifter. If the wire is 5 V and the
scooter ground is shared, a series resistor plus a 3.3 V clamp is enough; if
ground offsets from the PWM output stage cause trouble, an optocoupler is the
robust choice.

## Decoder sketch

Not written yet, kept here so the next session starts at the right place.
Runs on edge timestamps, not on a UART peripheral.

```
edges = list of (timestamp_us, level)      # alternating, starting LOW
for each LOW run followed by a HIGH run:
    if low  >= 8000 us and high in 1000..3000 us:   frame start (sync)
    elif low + high in 1600..2400 us:               bit, one if high/low_high > 0.4
    elif low in 4000..12000 us and not in frame:    stop / idle
on frame end: bytes = pack(bits, LSB first)
               valid = (sum(bytes[:-1]) & 0xFF) == bytes[-1]
```

Practical notes, for whoever writes it:

- Edge rate is ~1 kHz worst case (two edges per 2 ms bit), so userspace is
  fast enough. `libgpiod`'s `gpiomon --timestamp` gives kernel timestamps with
  resolution far below 100 µs; `lgpio` callbacks work too.
- GPIO choice: 14/15 are the UART and 5/6/13/19 are taken by the indicator and
  high-beam inputs, see [GPIO-INPUTS.md](./GPIO-INPUTS.md). Pick a free pin and
  add a config key next to `scooterHighBeamGpio`.
- Do not use the 8N1 path: at 9600 baud a bit is 104 µs, a SIF bit is 2000 µs.
  Reading one as the other produces garbage, not a slow but valid stream.
- Treat the checksum as the frame validator. Half-parsed frames are normal
  while the wheel starts and the transmitter's first frames arrive mid-burst.

## Sources

- T/JSEBA 002—2022, *电动自行车通信协议*, Annex A — the timing and framing table
  above. [PDF](https://static1.tianyancha.com/czd_file/standard/20a57dd087553ac36f31a191890a9781.pdf) ·
  [catalogue](https://www.spc.org.cn/online/d06613166c916ed10793cdacee580260.html)
- Manufacturer, no protocol documentation published:
  [en.tzchifeng.com](https://en.tzchifeng.com/product_list.html) ·
  [contact](https://en.tzchifeng.com/contact.html). Contact them with the exact
  model and serial from the controller label if a definitive answer is needed;
  Chifeng is an OEM and releases protocol documents to customers only.
- [teixeluis/escooter-lcd-esc-decode](https://github.com/teixeluis/escooter-lcd-esc-decode)
  — counter-example for the two-wire case: 1200 bps 8N1 over separate RX/TX
  between ESC and LCD.
- [ASD123-dsh/One-line-test](https://github.com/ASD123-dsh/One-line-test) —
  PC-side frame generator for 19 named Chinese one-wire cluster protocols,
  useful as a source of real frame layouts once the ID is known.
