// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

//! Finding out what a connected receiver can do.
//!
//! A receiver says nothing about itself in NMEA, so the only way to learn what
//! it supports is to try: switch it to a baud rate and look whether sentences
//! arrive, then step the position rate and count the fixes. That is what this
//! module does, over the binary CASIC protocol the AT6558 family speaks
//! (`$PCAS01` for the baud rate, `$PCAS02` for the rate in milliseconds).
//!
//! Every step is verified before it is believed, and a receiver that stops
//! answering is searched for again on all rates, so a probe that fails halfway
//! does not leave the port unusable. The caller restores the configured baud
//! rate and rate afterwards.

use std::path::Path;
use std::time::{Duration, Instant};

use livi_core_proto::gnss::{Caps, Rate};
use serde::{Deserialize, Serialize};

use super::port::Port;

/// The rates the AT6558 family offers, in the order its index expects.
pub const BAUDS: [u32; 6] = [4800, 9600, 19200, 38400, 57600, 115200];

/// Position rates worth asking for, slowest first.
pub const PERIODS: [u32; 5] = [1000, 500, 200, 100, 50];

/// How long a step may take to show up in the data stream.
const SETTLE: Duration = Duration::from_millis(1500);
/// Time between letting one baud rate go and opening the next, so the module has
/// switched and the old reader cannot eat the new rate's bytes.
const SWITCH: Duration = Duration::from_millis(250);
/// How long a rate is measured. One second more than the slowest period, so a
/// single fix never has to decide on its own.
const MEASURE: Duration = Duration::from_millis(2200);
/// Share of the line a rate may use. Half of it leaves room for the extra
/// sentences a receiver adds at higher rates without pushing the line to its
/// limit.
const LINE_USE: f64 = 0.5;

/// One CASIC frame: `$PCAS01,1*1D`, with the checksum over everything between
/// the `$` and the `*`.
pub fn frame(body: &str) -> Vec<u8> {
    let checksum = body.bytes().fold(0u8, |acc, byte| acc ^ byte);
    format!("${body}*{checksum:02X}\r\n").into_bytes()
}

/// The frame that makes the receiver talk at `baud_rate`, if it is one of the
/// rates the protocol knows.
pub fn set_baud(baud_rate: u32) -> Option<Vec<u8>> {
    let index = BAUDS.iter().position(|b| *b == baud_rate)?;
    Some(frame(&format!("PCAS01,{index}")))
}

/// The frame that makes the receiver deliver a fix every `rate_ms`.
pub fn set_rate(rate_ms: u32) -> Vec<u8> {
    frame(&format!("PCAS02,{rate_ms}"))
}

/// Counts the NMEA sentences with a valid checksum, which is what separates a
/// receiver talking at this baud rate from noise read at the wrong one.
pub fn sentences(data: &[u8]) -> usize {
    data.split(|b| *b == b'\n')
        .filter(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let Some(body) = line.strip_prefix(b"$") else {
                return false;
            };
            let Some(star) = body.iter().rposition(|b| *b == b'*') else {
                return false;
            };
            let text = std::str::from_utf8(&body[..star]).unwrap_or("");
            let Some(given) = std::str::from_utf8(&body[star + 1..])
                .ok()
                .and_then(|s| u8::from_str_radix(s.trim(), 16).ok())
            else {
                return false;
            };
            text.bytes().fold(0u8, |acc, b| acc ^ b) == given
        })
        .count()
}

/// Counts the fixes in a stretch of data. The RMC sentence is the one every
/// receiver sends per fix.
pub fn fixes(data: &[u8]) -> usize {
    data.split(|b| *b == b'$')
        .filter(|part| part.starts_with(b"GNRMC") || part.starts_with(b"GPRMC"))
        .count()
}

/// The slowest baud rate that leaves room for `bytes_per_second`, or the
/// fastest the receiver offers when even that is too slow.
pub fn baud_for(bytes_per_second: f64, bauds: &[u32]) -> u32 {
    bauds
        .iter()
        .copied()
        .find(|baud| (*baud as f64 / 10.0) * LINE_USE >= bytes_per_second)
        .or_else(|| bauds.last().copied())
        .unwrap_or(BAUDS[0])
}

/// The result of a probe, kept on disk so the receiver is asked once instead of
/// on every start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cached {
    device: String,
    caps: Caps,
}

/// Reads what was found for this device before, if anything was.
pub fn cached(path: &Path, device: &str) -> Option<Caps> {
    let text = std::fs::read_to_string(path).ok()?;
    let cached: Cached = serde_json::from_str(&text).ok()?;
    (cached.device == device).then_some(cached.caps)
}

/// Remembers what was found, so the next start does not have to probe again.
pub fn remember(path: &Path, device: &str, caps: &Caps) {
    let cached = Cached {
        device: device.to_string(),
        caps: caps.clone(),
    };
    let Ok(text) = serde_json::to_string_pretty(&cached) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, text + "\n");
}

struct Link {
    port: Port,
    baud_rate: u32,
}

impl Link {
    fn open(device: &str, baud_rate: u32) -> Result<Self, String> {
        Port::open(device, baud_rate).map(|port| Self { port, baud_rate })
    }

    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.port.write(bytes).map_err(|e| e.to_string())
    }

    /// Everything that arrives within `window`.
    fn listen(&mut self, window: Duration) -> Vec<u8> {
        let until = Instant::now() + window;
        let mut data = Vec::new();
        while Instant::now() < until {
            match self.port.chunks.blocking_recv() {
                Some(Ok(chunk)) => data.extend_from_slice(&chunk),
                Some(Err(_)) | None => break,
            }
        }
        data
    }

    /// Throws away what is already buffered, so a measurement starts clean.
    fn drop_stale(&mut self) {
        while self.port.chunks.try_recv().is_ok() {}
    }
}

/// Opens the port again at `baud_rate`, after letting go of `link`. Two readers
/// on the same line would steal each other's bytes, so the old one has to be
/// closed first.
fn reopen(link: Link, device: &str, baud_rate: u32) -> Option<Link> {
    let mut previous = link;
    let _ = previous.send(&set_baud(baud_rate).unwrap_or_default());
    drop(previous);
    std::thread::sleep(SWITCH);
    Link::open(device, baud_rate).ok()
}

/// Looks for a receiver that answers on any of the known rates.
fn scan(device: &str) -> Option<Link> {
    for baud_rate in BAUDS {
        let Ok(mut link) = Link::open(device, baud_rate) else {
            continue;
        };
        if sentences(&link.listen(SETTLE)) > 0 {
            return Some(link);
        }
    }
    None
}

/// Asks the receiver what it can do. Leaves it at `restore_baud` and
/// `restore_rate_ms`.
pub fn detect(device: &str, restore_baud: u32, restore_rate_ms: u32) -> Result<Caps, String> {
    let mut link = scan(device).ok_or_else(|| format!("no sentences on {device}"))?;

    let mut baud_rates = Vec::new();
    for baud_rate in BAUDS {
        if baud_rate == link.baud_rate {
            baud_rates.push(baud_rate);
            continue;
        }
        let moved = reopen(link, device, baud_rate);
        match moved {
            Some(mut next) => {
                let heard = sentences(&next.listen(SETTLE)) > 0;
                if heard {
                    next.drop_stale();
                    baud_rates.push(baud_rate);
                    link = next;
                } else {
                    drop(next);
                    link = scan(device).ok_or_else(|| "receiver lost".to_string())?;
                }
            }
            None => link = scan(device).ok_or_else(|| "receiver lost".to_string())?,
        }
    }

    // The rates are measured at the fastest rate the line allows, so a rate is
    // not called unsupported only because the current baud rate is too slow.
    let fastest = baud_rates.iter().copied().max().unwrap_or(link.baud_rate);
    if fastest != link.baud_rate {
        match reopen(link, device, fastest) {
            Some(mut next) => {
                next.drop_stale();
                link = next;
            }
            None => return Err("receiver lost".to_string()),
        }
    }

    let mut rates = Vec::new();
    for period in PERIODS {
        if link.send(&set_rate(period)).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(400));
        link.drop_stale();
        let data = link.listen(MEASURE);
        let seconds = MEASURE.as_secs_f64();
        let per_second = fixes(&data) as f64 / seconds;
        let wanted = 1000.0 / f64::from(period);
        if per_second < wanted * 0.7 {
            continue;
        }
        let baud_rate = baud_for(data.len() as f64 / seconds, &baud_rates);
        rates.push(Rate {
            rate_ms: period,
            baud_rate,
        });
    }

    // Back to where the caller wants it, rate first: switching the baud rate
    // changes the line the next frame has to be sent on.
    let _ = link.send(&set_rate(restore_rate_ms));
    std::thread::sleep(Duration::from_millis(200));
    if link.baud_rate != restore_baud {
        match reopen(link, device, restore_baud) {
            Some(mut next) => {
                next.drop_stale();
                link = next;
            }
            None => return Err(format!("{restore_baud} baud does not answer")),
        }
    }
    link.drop_stale();
    if sentences(&link.listen(SETTLE)) == 0 && !baud_rates.contains(&restore_baud) {
        return Err(format!("{restore_baud} baud does not answer"));
    }

    Ok(Caps { baud_rates, rates })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_carry_the_checksum_the_protocol_expects() {
        assert_eq!(frame("PCAS01,1"), b"$PCAS01,1*1D\r\n");
        assert_eq!(frame("PCAS02,1000"), b"$PCAS02,1000*2E\r\n");
        assert_eq!(frame("PCAS06,0"), b"$PCAS06,0*1B\r\n");
        assert_eq!(set_baud(4800).as_deref(), Some(&b"$PCAS01,0*1C\r\n"[..]));
        assert_eq!(set_baud(115200).as_deref(), Some(&b"$PCAS01,5*19\r\n"[..]));
        assert_eq!(set_baud(12345), None);
        assert_eq!(set_rate(200), b"$PCAS02,200*1D\r\n");
    }

    #[test]
    fn only_sentences_with_a_valid_checksum_count() {
        let good = b"$GNGGA,,,,,,0,00,25.5,,,,,,*64\r\n";
        let bad = b"$GNGGA,,,,,,0,00,25.5,,,,,,*65\r\n";
        assert_eq!(sentences(good), 1);
        assert_eq!(sentences(bad), 0);
        assert_eq!(sentences(b"\x00\xff garbage"), 0);
        assert_eq!(sentences(&[good.as_slice(), good.as_slice()].concat()), 2);
    }

    #[test]
    fn fixes_are_counted_per_rmc() {
        let data = b"$GNGGA,x*00\r\n$GNRMC,1*00\r\n$GNRMC,2*00\r\n$GPRMC,3*00\r\n";
        assert_eq!(fixes(data), 3);
    }

    #[test]
    fn a_rate_gets_the_slowest_baud_rate_that_carries_it() {
        // 276 bytes per fix, the set an ATGM336H sends.
        let bauds = [4800, 9600, 19200, 38400, 57600, 115200];
        assert_eq!(baud_for(276.0, &bauds), 9600);
        assert_eq!(baud_for(552.0, &bauds), 19200);
        assert_eq!(baud_for(1380.0, &bauds), 38400);
        assert_eq!(baud_for(2760.0, &bauds), 57600);
        assert_eq!(baud_for(90_000.0, &bauds), 115200);
        assert_eq!(baud_for(276.0, &[9600, 19200]), 9600);
        assert_eq!(baud_for(276.0, &[4800]), 4800, "the only rate available");
    }
}
