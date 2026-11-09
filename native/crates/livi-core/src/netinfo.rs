// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

//! Publishes what the host is attached to: addresses, gateways, wireless links.
//!
//! Nothing here decides how the network is configured; it only reports. The
//! settings page under System reads it, so a device in the field can be asked
//! what address it has without a keyboard.

use std::collections::BTreeMap;
use std::time::Duration;

use livi_core_proto::state::{NetIface, NetKind, Network};
use serde_json::Value;

/// Slower than the pack monitor: addresses and routes change when a cable is
/// plugged in or a link comes up, not between two frames.
const POLL: Duration = Duration::from_secs(10);

/// The resolvers the host asks, in order and without repeats.
pub fn parse_dns(resolv_conf: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in resolv_conf.lines() {
        let Some(rest) = line.trim().strip_prefix("nameserver") else {
            continue;
        };
        let Some(server) = rest.split_whitespace().next() else {
            continue;
        };
        if !out.iter().any(|s| s == server) {
            out.push(server.to_string());
        }
    }
    out
}

/// One interface as `ip -j addr show` describes it, reduced to what is shown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Addr {
    pub name: String,
    pub mac: String,
    pub up: bool,
    pub ipv4: String,
    pub ipv6: Vec<String>,
    /// The kernel calls a wireless interface's link type `ether` too, so this
    /// comes from the sysfs `wireless` directory rather than from `link_type`.
    pub wireless: bool,
    /// The host offers this network instead of joining one.
    pub ap: bool,
    pub ssid: String,
    pub signal_dbm: Option<i32>,
    pub rate_mbps: Option<u32>,
}

pub fn parse_addrs(json: &str) -> Vec<Addr> {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    list.iter().filter_map(one_addr).collect()
}

fn one_addr(v: &Value) -> Option<Addr> {
    let name = v["ifname"].as_str()?.to_string();
    let is = |field: &str, want: &str| {
        v[field]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(want))
    };
    if is("link_type", "loopback") {
        return None;
    }
    let mut ipv4 = String::new();
    let mut ipv6: Vec<String> = Vec::new();
    for a in v["addr_info"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let (Some(family), Some(local)) = (a["family"].as_str(), a["local"].as_str()) else {
            continue;
        };
        let prefix = a["prefixlen"].as_u64().unwrap_or(0);
        match family {
            "inet" if ipv4.is_empty() => ipv4 = format!("{local}/{prefix}"),
            // A link-local address only reaches the neighbouring segment, so it
            // would answer "does this host have an address" with a yes that
            // means nothing.
            "inet6" if a["scope"].as_str() != Some("link") => {
                ipv6.push(format!("{local}/{prefix}"));
            }
            _ => {}
        }
    }
    Some(Addr {
        // iproute2 reports this as `UP`, not `up`, and the casing has varied
        // between releases, so neither spelling may decide the outcome.
        up: is("operstate", "up"),
        mac: v["address"]
            .as_str()
            .filter(|m| *m != "00:00:00:00:00:00")
            .unwrap_or_default()
            .to_uppercase(),
        name,
        ipv4,
        ipv6,
        ..Addr::default()
    })
}

/// The gateways by interface, from `ip -j route show default`. The first entry
/// for an interface wins, because `ip` orders them by metric.
pub fn parse_gateways(json: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(json) else {
        return out;
    };
    for route in list {
        let (Some(dev), Some(gateway)) = (route["dev"].as_str(), route["gateway"].as_str()) else {
            continue;
        };
        out.entry(dev.to_string()).or_insert(gateway.to_string());
    }
    out
}

/// Network name, receive level and rate from `iw dev <iface> link`.
///
/// The three vary by driver and kernel, so each is optional: a missing line
/// leaves its field empty rather than inventing a value.
pub fn parse_station(text: &str) -> (String, Option<i32>, Option<u32>) {
    let mut ssid = String::new();
    let mut signal = None;
    let mut rate = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("SSID:") {
            ssid = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("signal:") {
            signal = first_number(rest).map(|v| v as i32);
        } else if let Some(rest) = line.strip_prefix("rx bitrate:") {
            rate = first_number(rest).map(|v| v.round() as u32);
        }
    }
    (ssid, signal, rate)
}

fn first_number(text: &str) -> Option<f64> {
    let start = text.find(|c: char| c.is_ascii_digit() || c == '-')?;
    let rest = &text[start..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// The interfaces worth showing, with their kind and gateway attached.
///
/// An interface appears when it is up and carries at least one address. A cable
/// that is plugged in but has not been given one yet is nothing the driver can
/// act on, and an unconfigured `wlan0` would only be noise.
pub fn assemble(
    mut addrs: Vec<Addr>,
    gateways: &BTreeMap<String, String>,
    dns: Vec<String>,
) -> Network {
    addrs.retain(|a| a.up && (!a.ipv4.is_empty() || !a.ipv6.is_empty()));
    addrs.sort_by_key(|a| match kind_of(a) {
        NetKind::Ethernet => 0u8,
        NetKind::Wifi => 1,
        NetKind::Ap => 2,
    });
    let interfaces = addrs
        .into_iter()
        .map(|a| NetIface {
            kind: kind_of(&a),
            gateway: gateways.get(&a.name).cloned().unwrap_or_default(),
            name: a.name,
            ipv4: a.ipv4,
            ipv6: a.ipv6,
            mac: a.mac,
            ssid: a.ssid,
            signal_dbm: a.signal_dbm,
            rate_mbps: a.rate_mbps,
        })
        .collect();
    Network { interfaces, dns }
}

fn kind_of(a: &Addr) -> NetKind {
    match (a.ap, a.wireless) {
        (true, _) => NetKind::Ap,
        (false, true) => NetKind::Wifi,
        (false, false) => NetKind::Ethernet,
    }
}

pub async fn run(core: std::sync::Arc<crate::server::Core>) {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = core;
    }
    #[cfg(target_os = "linux")]
    linux::run(core).await;
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeMap;
    use std::process::Command;

    use livi_core_proto::state::Network;

    use super::{Addr, assemble, parse_addrs, parse_dns, parse_gateways, parse_station};

    const RESOLV_CONF: &str = "/etc/resolv.conf";

    /// `ip` and `iw` live in `/usr/sbin` or `/sbin`, which a service does not
    /// always have on its `PATH`.
    fn tool(name: &str) -> String {
        for dir in ["/usr/sbin", "/sbin", "/usr/bin", "/bin"] {
            let path = format!("{dir}/{name}");
            if std::path::Path::new(&path).exists() {
                return path;
            }
        }
        name.to_string()
    }

    fn out(tool_name: &str, args: &[&str]) -> String {
        Command::new(tool(tool_name))
            .args(args)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    }

    fn wireless(iface: &str) -> bool {
        std::path::Path::new(&format!("/sys/class/net/{iface}/wireless")).exists()
    }

    /// The negotiated rate of a wired link. The driver reports `-1` for a link
    /// that is down, and some do not report one at all.
    fn wired_rate(iface: &str) -> Option<u32> {
        let raw = std::fs::read_to_string(format!("/sys/class/net/{iface}/speed")).ok()?;
        raw.trim()
            .parse::<i64>()
            .ok()
            .filter(|v| *v > 0)
            .map(|v| v as u32)
    }

    /// Fills in what `ip` cannot tell: whether the interface is a radio, whether
    /// it is one this host offers, and what the link looks like.
    fn probe(addrs: &mut [Addr]) {
        for a in addrs.iter_mut() {
            a.wireless = wireless(&a.name);
            if !a.wireless {
                a.rate_mbps = wired_rate(&a.name);
                continue;
            }
            match livi_wifi::ap_state(&a.name) {
                Some(ap) => {
                    a.ap = true;
                    a.ssid = ap.ssid;
                }
                None => {
                    let link = out("iw", &["dev", &a.name, "link"]);
                    let (ssid, signal, rate) = parse_station(&link);
                    a.ssid = ssid;
                    a.signal_dbm = signal;
                    a.rate_mbps = rate;
                }
            }
        }
    }

    fn gather() -> Network {
        let mut addrs = parse_addrs(&out("ip", &["-j", "addr", "show"]));
        let gateways: BTreeMap<String, String> =
            parse_gateways(&out("ip", &["-j", "route", "show", "default"]));
        probe(&mut addrs);
        let dns = parse_dns(&std::fs::read_to_string(RESOLV_CONF).unwrap_or_default());
        assemble(addrs, &gateways, dns)
    }

    pub async fn run(core: std::sync::Arc<crate::server::Core>) {
        let mut tick = tokio::time::interval(super::POLL);
        loop {
            tick.tick().await;
            let Ok(net) = tokio::task::spawn_blocking(gather).await else {
                continue;
            };
            core.hub.update(|s| s.system.network = net);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDRS: &str = r#"[
      {"ifindex":1,"ifname":"lo","operstate":"UNKNOWN","link_type":"loopback",
       "address":"00:00:00:00:00:00",
       "addr_info":[{"family":"inet","local":"127.0.0.1","prefixlen":8,"scope":"host"}]},
      {"ifindex":2,"ifname":"eth0","operstate":"UP","link_type":"ether",
       "address":"2c:cf:67:3f:ac:54",
       "addr_info":[
         {"family":"inet","local":"192.168.0.20","prefixlen":24,"scope":"global"},
         {"family":"inet6","local":"fe80::2e0:4cff:fe36:88c","prefixlen":64,"scope":"link"},
         {"family":"inet6","local":"2a02:8071::1ca2","prefixlen":128,"scope":"global"}]},
      {"ifindex":3,"ifname":"wlan0","operstate":"UP","link_type":"ether",
       "address":"b8:27:eb:00:00:01",
       "addr_info":[{"family":"inet","local":"192.168.0.3","prefixlen":24,"scope":"global"}]},
      {"ifindex":4,"ifname":"wlan1","operstate":"DOWN","link_type":"ether",
       "address":"b8:27:eb:00:00:02","addr_info":[]},
      {"ifindex":5,"ifname":"usb0","operstate":"UP","link_type":"ether",
       "address":"00:00:00:00:00:00","addr_info":[]}
    ]"#;

    fn sample() -> Vec<Addr> {
        parse_addrs(ADDRS)
    }

    /// The same interfaces with the sysfs facts applied: `eth0` is wired,
    /// `wlan0` is a radio. `lo` is already gone by then.
    fn wired_and_radio() -> Vec<Addr> {
        let mut addrs = sample();
        assert_eq!(addrs[0].name, "eth0");
        assert_eq!(addrs[1].name, "wlan0");
        addrs[1].wireless = true;
        addrs
    }

    #[test]
    fn loopback_is_left_out() {
        assert!(sample().iter().all(|a| a.name != "lo"));
    }

    #[test]
    fn the_state_is_read_whatever_its_casing() {
        // iproute2 prints `UP`, and an older build printed `up`.
        let upper = r#"[{"ifname":"eth0","operstate":"UP","link_type":"ether","address":"aa:bb:cc:dd:ee:ff",
                        "addr_info":[{"family":"inet","local":"10.0.0.2","prefixlen":24,"scope":"global"}]}]"#;
        let lower = upper.replace(r#""UP""#, r#""up""#);
        for json in [upper, lower.as_str()] {
            let a = &parse_addrs(json)[0];
            assert!(a.up, "{json}");
            assert_eq!(
                assemble(parse_addrs(json), &BTreeMap::new(), Vec::new())
                    .interfaces
                    .len(),
                1
            );
        }
    }

    #[test]
    fn addresses_carry_their_prefix() {
        let a = &sample()[0];
        assert_eq!(a.name, "eth0");
        assert_eq!(a.ipv4, "192.168.0.20/24");
        assert!(a.up);
        assert_eq!(a.mac, "2C:CF:67:3F:AC:54");
    }

    #[test]
    fn link_local_ipv6_says_nothing_about_reach() {
        assert_eq!(sample()[0].ipv6, ["2a02:8071::1ca2/128"]);
    }

    #[test]
    fn only_interfaces_with_an_address_and_a_link_are_shown() {
        let net = assemble(wired_and_radio(), &BTreeMap::new(), Vec::new());
        assert_eq!(
            net.interfaces.len(),
            2,
            "wlan1 is down, usb0 has no address"
        );
        assert_eq!(net.interfaces[0].name, "eth0");
        assert_eq!(net.interfaces[0].kind, NetKind::Ethernet);
        assert_eq!(net.interfaces[1].kind, NetKind::Wifi);
    }

    #[test]
    fn an_access_point_is_listed_after_the_station() {
        // A host can offer its own network while the second radio is joined to
        // another, and the one it is joined to is the one to look at first.
        let mut addrs = wired_and_radio();
        addrs[0].wireless = true;
        addrs[0].ap = true;
        let net = assemble(addrs, &BTreeMap::new(), Vec::new());
        let kinds: Vec<NetKind> = net.interfaces.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, [NetKind::Wifi, NetKind::Ap]);
    }

    #[test]
    fn the_gateway_belongs_to_its_interface() {
        let gateways = parse_gateways(
            r#"[{"dst":"default","gateway":"192.168.0.1","dev":"eth0","metric":100},
                {"dst":"default","gateway":"192.168.0.251","dev":"eth0","metric":600},
                {"dst":"default","gateway":"10.10.0.1","dev":"wlan0","metric":100}]"#,
        );
        assert_eq!(gateways["eth0"], "192.168.0.1", "the best metric wins");
        assert_eq!(gateways["wlan0"], "10.10.0.1");
        let net = assemble(wired_and_radio(), &gateways, Vec::new());
        assert_eq!(net.interfaces[0].gateway, "192.168.0.1");
        assert_eq!(net.interfaces[1].gateway, "10.10.0.1");
    }

    #[test]
    fn a_station_link_is_read_off_iw() {
        let link = "Connected to 2c:cf:67:3f:ac:54 (on wlan0)\n\
                    \tSSID: Werkstatt\n\
                    \tfreq: 2412\n\
                    \tsignal: -47 dBm\n\
                    \trx bitrate: 72.2 MBit/s\n\
                    \ttx bitrate: 72.2 MBit/s\n";
        assert_eq!(
            parse_station(link),
            ("Werkstatt".into(), Some(-47), Some(72))
        );
    }

    #[test]
    fn an_interface_that_is_not_connected_yields_nothing() {
        assert_eq!(
            parse_station("Not connected.\n"),
            (String::new(), None, None)
        );
    }

    #[test]
    fn resolvers_are_kept_in_order_and_without_repeats() {
        let conf = "# Generated by NetworkManager\n\
                    nameserver 192.168.0.1\n\
                    nameserver 2a02:8071::1\n\
                    nameserver 192.168.0.1\n\
                    search lan\n";
        assert_eq!(parse_dns(conf), ["192.168.0.1", "2a02:8071::1"]);
    }

    #[test]
    fn junk_in_place_of_json_is_not_a_panic() {
        assert!(parse_addrs("").is_empty());
        assert!(parse_addrs("not json").is_empty());
        assert!(parse_gateways("<html>").is_empty());
    }
}
