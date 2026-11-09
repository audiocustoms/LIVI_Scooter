// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

//! The Network page under System: what the host is attached to.
//!
//! Every field comes from the core's `system.network`, which the host measures.
//! A field with nothing to show is left out entirely rather than given a dash,
//! so a wired-only host does not display empty wireless rows.

use livi_core_proto::state::{NetKind, Network};
use serde_json::Value;

use super::{DASH, info, mono};
use crate::i18n::Strings;
use crate::settings::view::Row;

pub fn rows(state: &Value, t: &Strings) -> Vec<Row> {
    let net: Network =
        serde_json::from_value(state["system"]["network"].clone()).unwrap_or_default();
    if net.interfaces.is_empty() {
        return vec![info(
            t.t("settings.networkNone", "Not connected"),
            Some("network"),
            String::new(),
        )];
    }
    let mut out = Vec::new();
    for iface in &net.interfaces {
        out.push(info(
            t.t(kind_key(iface.kind), kind_text(iface.kind)).to_string(),
            Some("network"),
            join(&[iface.name.clone(), iface.ssid.clone()]),
        ));
        if !iface.ipv4.is_empty() {
            out.push(mono(
                t.t("settings.networkIpv4", "IPv4"),
                iface.ipv4.clone(),
            ));
        }
        if !iface.ipv6.is_empty() {
            out.push(mono(
                t.t("settings.networkIpv6", "IPv6"),
                iface.ipv6.join("\n"),
            ));
        }
        if !iface.gateway.is_empty() {
            out.push(mono(
                t.t("settings.networkGateway", "Gateway"),
                iface.gateway.clone(),
            ));
        }
        // Level and rate answer the same question — how good is this link — and
        // a small display has no room for two rows of it.
        let link = join_parts(&[
            iface.signal_dbm.map(|dbm| format!("{dbm} dBm")),
            iface.rate_mbps.map(|rate| format!("{rate} Mbit/s")),
        ]);
        if !link.is_empty() {
            out.push(mono(t.t("settings.networkRate", "Link"), link));
        }
        if !iface.mac.is_empty() {
            out.push(mono(t.t("settings.networkMac", "MAC"), iface.mac.clone()));
        }
    }
    if !net.dns.is_empty() {
        out.push(mono(t.t("settings.networkDns", "DNS"), net.dns.join("\n")));
    }
    if out.is_empty() {
        out.push(mono(t.t("settings.networkIpv4", "IPv4"), DASH.into()));
    }
    out
}

/// Joins the parts that are there, so an interface without a name to show does
/// not leave a stray separator.
fn join(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ")
}

fn join_parts(parts: &[Option<String>]) -> String {
    let present: Vec<String> = parts.iter().flatten().cloned().collect();
    present.join(" · ")
}

fn kind_key(kind: NetKind) -> &'static str {
    match kind {
        NetKind::Ethernet => "settings.networkEthernet",
        NetKind::Wifi => "settings.networkWifi",
        NetKind::Ap => "settings.networkAp",
    }
}

fn kind_text(kind: NetKind) -> &'static str {
    match kind {
        NetKind::Ethernet => "Ethernet",
        NetKind::Wifi => "Wi-Fi",
        NetKind::Ap => "Access Point",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::settings::view::RowKind;

    fn rows_of(network: Value) -> Vec<Row> {
        rows(
            &json!({ "system": { "network": network } }),
            &Strings::new("en"),
        )
    }

    #[test]
    fn a_wired_host_shows_its_address_and_gateway() {
        let rows = rows_of(json!({
            "interfaces": [{
                "name": "eth0", "kind": "ethernet", "ipv4": "192.168.0.20/24",
                "ipv6": [], "gateway": "192.168.0.1", "mac": "2C:CF:67:3F:AC:54"
            }],
            "dns": ["192.168.0.1"]
        }));
        let values: Vec<&str> = rows.iter().map(|r| r.value.as_str()).collect();
        assert_eq!(
            values,
            [
                "eth0",
                "192.168.0.20/24",
                "192.168.0.1",
                "2C:CF:67:3F:AC:54",
                "192.168.0.1"
            ]
        );
        assert_eq!(rows[0].label, "Ethernet");
        assert_eq!(rows[0].kind, RowKind::Info);
        assert_eq!(rows[1].kind, RowKind::Info);
    }

    #[test]
    fn a_joined_wireless_link_also_shows_the_network_and_the_level() {
        let rows = rows_of(json!({
            "interfaces": [{
                "name": "wlan0", "kind": "wifi", "ipv4": "192.168.0.3/24",
                "ssid": "Werkstatt", "signalDbm": -47, "rateMbps": 72,
                "gateway": "192.168.0.1", "mac": "B8:27:EB:00:00:01"
            }],
            "dns": []
        }));
        assert_eq!(rows[0].label, "Wi-Fi");
        assert_eq!(rows[0].value, "wlan0 · Werkstatt");
        let link = rows.iter().find(|r| r.label == "Link").unwrap();
        assert_eq!(link.value, "-47 dBm · 72 Mbit/s");
    }

    #[test]
    fn a_wired_link_shows_its_rate_without_a_level() {
        let rows = rows_of(json!({
            "interfaces": [{
                "name": "eth0", "kind": "ethernet", "ipv4": "192.168.0.20/24",
                "rateMbps": 1000
            }],
            "dns": []
        }));
        let link = rows.iter().find(|r| r.label == "Link").unwrap();
        assert_eq!(link.value, "1000 Mbit/s");
    }

    #[test]
    fn an_access_point_is_named_as_such() {
        let rows = rows_of(json!({
            "interfaces": [{
                "name": "wlan0", "kind": "ap", "ipv4": "10.10.0.1/24",
                "ssid": "LIVI", "mac": "B8:27:EB:00:00:01"
            }],
            "dns": []
        }));
        assert_eq!(rows[0].label, "Access Point");
        assert_eq!(rows[0].value, "wlan0 · LIVI");
    }

    #[test]
    fn nothing_to_show_says_so_instead_of_listing_empty_rows() {
        let rows = rows_of(json!({ "interfaces": [], "dns": [] }));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "Not connected");
    }

    #[test]
    fn an_interface_without_a_gateway_leaves_the_row_out() {
        let rows = rows_of(json!({
            "interfaces": [{ "name": "eth0", "kind": "ethernet", "ipv4": "10.0.0.2/8" }],
            "dns": []
        }));
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Ethernet", "IPv4"],
            "no gateway, no MAC, no DNS row"
        );
    }
}
