# Repository dev scripts

Tooling for working on LIVI on the development machine itself, instead of
building an AppImage and deploying it to a device for every change.

## `livi-window` — the UI in a window

Syncs the checkout into WSL, builds `livi-ui` incrementally and opens it in a
WSLg window. Only the UI is built: no core, no GStreamer host, no compositor, no
AppImage. Menus, settings pages, layout and the dashboard render from a fixture
state, so UI work takes a build and nothing else.

```powershell
scripts\dev\livi-window.cmd
```

Or from inside WSL:

```sh
scripts/dev/livi-window.sh
```

The first run syncs the checkout to `~/livi` and builds from scratch, which
takes a few minutes. Later runs sync, notice nothing changed and open the window
straight away; a changed `.rs` or `.slint` file costs an incremental build.

Options:

| Option | Effect |
| --- | --- |
| `--page NAME` | open `settings`, `telemetry`, `projection` or `media` |
| `--settings PATH` | with `--page settings`, open that path, e.g. `car/carType` |
| `--size WxH` | window size, e.g. `800x480` for a Pi-sized layout |
| `--state FILE` | render another core state, e.g. `crates/livi-ui/fixtures/scooter.json` |
| `--dir PATH` | build directory in the Linux home (default `~/livi`) |
| `--rebuild` | build even when the binary looks up to date |
| `--no-sync` | keep the build directory untouched |
| `--stop` | close a running window |

Fixture states live in `native/crates/livi-ui/fixtures`. The window opens
`telemetry` with `scooter.json` by default, because that is what the Scooter
dashboard is developed against. `scooter.json` is `telemetry.json` with the car
type set to Scooter. Other states, for example the settings menus:

```powershell
scripts\dev\livi-window.cmd --page settings --settings car/carType
scripts\dev\livi-window.cmd --page telemetry --state crates/livi-ui/fixtures/telemetry.json
```

## Requirements

WSL2 with WSLg, which shows Linux windows on the Windows desktop. WSLg ships
with Windows 11 and with Windows 10 21H2 and newer.

Debian packages, once:

```sh
sudo apt-get install -y build-essential pkg-config cmake git curl rsync file \
  libssl-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  libwayland-dev libxkbcommon-dev libfontconfig1-dev libudev-dev \
  libinput-dev libseat-dev libgbm-dev libegl1-mesa-dev \
  libx11-6 libxcursor1 libxrandr2 libxi6 libxkbcommon-x11-0
```

Build in the Linux home, never on `/mnt`: the Windows filesystem makes cargo
several times slower and keeps virus scanners busy on every object file.

## `livi-pi-i2c` — the pack monitor's bus on a Pi

Brings up the I²C bus an INA228 pack monitor sits on. Run it **on the Pi**, not
through the dev host:

```sh
sudo scripts/dev/livi-pi-i2c.sh --reboot
scripts/dev/livi-pi-i2c.sh --check
```

It does three things, each skipped when already in place:

| What | Why |
| --- | --- |
| `dtparam=i2c_arm=on` in `config.txt` | binds I²C to GPIO 2/3; needs a reboot |
| the `i2c-dev` module, listed in `/etc/modules-load.d/livi-i2c.conf` | without it the bus exists in the kernel but `/dev/i2c-1` never appears |
| `i2c-tools` | `i2cdetect` to look for the chip |

`--check` reports the state and whether the monitor answers at the expected
address, changing nothing. `i2cdetect` lives in `/usr/sbin`, which a
non-interactive ssh session does not always have in `PATH`.

## Where the logs are

Two places, and a fault usually shows in only one of them:

| Source | Command |
| --- | --- |
| core, helper, UI banner | `journalctl -u livi-kiosk -b 0` |
| compositor: input routing, planes, outputs | `~/.config/LIVI/log/compositor.log` |

The compositor logs at `info`. `Environment=LIVI_WLR_DEBUG=1` in a kiosk drop-in
raises it to `debug`, which prints every touch it receives and whether it found a
surface for it.

For the UI's own view of pointer events, `Environment=LIVI_UI_INPUT_LOG=/path`
makes it append one line per event with the nav state it produced. Without the
variable it writes nothing.

## Two UI windows on one output

`screen 'main'` and `screen 'cluster'` can both land on the same physical output.
Input prefers the main one, the window on screen 0, whatever order the two were
registered in; a secondary dashboard mirror must never take touches away from the
menu. This was found the hard way: after a hardware reboot the mirror was
registered first, swallowed every touch, and the side menu could not be woken,
while swiping still worked because the mirror has a dashboard of its own.

## `DEV: Fake Map Background`

A development switch under **System**, below Power Off. It draws a stand-in map
where the projection stream would be, so the dashboard can be built and looked at
with no phone connected.

| Where | What happens |
| --- | --- |
| Projection page | the stand-in image replaces the stream, still inside the view area and its mask |
| Dashboard with a map layout | the stand-in sits behind the gauges, where the stream would show through |

The image is `native/crates/livi-ui/ui/dev/maps_fake.jpg`, embedded in the binary
at build time. It has to be a real JPEG or PNG: the `image` crate is built with
only those two formats, and the source file in this folder, despite its name, is
a WebP.

The switch writes `devFakeMap` to the config like any other setting, so it can be
set from a script:

```sh
jq '.devFakeMap = true' ~/.config/LIVI/config.json > /tmp/c.json && mv /tmp/c.json ~/.config/LIVI/config.json
```

It is a development aid and must not ship in a release, see [TODO.md](TODO.md).

## The pack monitor's settings

The monitor takes its scale and its limits from the car settings, so nothing in
the code has to know which pack is fitted:

| Setting | Default | Meaning |
| --- | --- | --- |
| `scooterBatteryVoltage` | 60 V | nominal pack voltage; full and empty follow from it |
| `scooterBatteryCapacityAh` | 30 Ah | the scale a coulomb counter is read against |
| `scooterShuntMilliohm` | 0.75 mΩ | an FL-2 marked 100 A / 75 mV |
| `scooterShuntMaxCurrentA` | 100 A | sets `CURRENT_LSB` and the calibration register |
| `scooterVbusScale` | 1.0 | corrects a board that reports the bus voltage off by a fixed factor |
| `scooterBatterySensor` | off | reads the pack over I²C; off until a chip is wired |

A 12 V pack is in the list for bench work: 3S, so 12.6 V full and 9.0 V empty.

The wiring, and the checks to run before switching on, are in
[`native/crates/livi-ina228/README.md`](../../native/crates/livi-ina228/README.md).
What is still open on the bench is in [TODO.md](TODO.md).

## `GPIO-INPUTS.md` — indicators and high beam

The dashboard lamps for the indicators and the high beam read two telemetry keys.
On a scooter those come from the vehicle's 12 V harness through four GPIO inputs.
The pin assignment, the level shifter requirements and what the UI does with the
signals are in [GPIO-INPUTS.md](GPIO-INPUTS.md).
