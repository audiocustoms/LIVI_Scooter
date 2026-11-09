// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

use std::rc::Rc;

use livi_core_client::Client;
use livi_core_proto::message::Action;
use livi_core_proto::state::{Screen, State};
use livi_ui_model::dashboard::gauge::{self, Drawing, Scale, Spec, draw, groups, label_overflow};
use livi_ui_model::dashboard::layout::{Center, pages, spec};
use livi_ui_model::dashboard::nav::{Nav, nav};
use livi_ui_model::dashboard::readings::{Readings, Turn, pack_text, readings};
use livi_ui_model::mask::{Mask, Stream, mask};
use slint::{Color, ComponentHandle, Image, ModelRc, SharedString, VecModel};

use crate::media::{Pixels, decode};
use crate::{AppWindow, CapData, DashState, GaugeBand, GaugeData, LabelData, MaskData, TrailData};

pub fn set_texts(d: &DashState, tr: impl Fn(&str, &str) -> String) {
    d.set_gear_label(tr("dashboard.gear", "GEAR").into());
    d.set_charge_label(tr("dashboard.charge", "CHARGE").into());
    d.set_placeholder(tr("dashboard.noDashboard", "No dashboard on this screen").into());
}

impl DashView {
    pub fn decode(&mut self) {
        if let Some(raw) = self.nav_image_raw.take() {
            self.nav_image = Some(raw.as_deref().and_then(decode));
        }
    }

    pub fn map(&self) -> bool {
        self.readings.enabled && self.map
    }
}

const RING_W: f64 = 470.0;
const RING_H: f64 = 600.0;
const LEFT_RING_LEFT: f64 = -32.0;
const RIGHT_RING_LEFT: f64 = 850.0;
const BASE_W: f64 = 1280.0;

pub struct GaugeView {
    drawing: Drawing,
    groups: gauge::Groups,
    mirror: bool,
    redline_index: f64,
    shadow: Pixels,
    shadow_half: f64,
}

pub struct DashView {
    readings: Readings,
    speed_gauge: Option<GaugeView>,
    rpm_gauge: Option<GaugeView>,
    ring_shift: f64,
    gauges: bool,
    map: bool,
    center: Center,
    page_count: usize,
    page_index: Option<usize>,
    accent: Option<String>,
    nav: Nav,
    nav_image: Option<Option<Pixels>>,
    nav_image_raw: Option<Option<String>>,
    mask: Option<Mask>,
}

#[derive(Default)]
pub struct DashReader {
    speed_key: Option<(Scale, usize)>,
    rpm_key: Option<(Scale, usize)>,
    image: Option<String>,
}

/// Which colour bands an arc is split into. The tones are named in `Signal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zones {
    Plain,
    Redline,
    Charge,
}

fn gauge_view(scale: &Scale, mirror: bool, zones: Zones, redline: f64) -> GaugeView {
    let spec = Spec {
        mirror,
        ..Spec::default()
    };
    let drawing = draw(&spec, scale, RING_W, RING_H);
    let redline_index = if redline > 0.0 {
        (redline / scale.max.max(1.0)).clamp(0.0, 1.0) * (drawing.arc_ticks - 1) as f64
    } else {
        f64::MAX
    };
    let last = (drawing.arc_ticks - 1).max(1) as f64;
    let groups = groups(
        &drawing,
        |i| band_tone(zones, redline_index, last, i),
        band_dim(zones),
    );
    let s = gauge::shadow(&drawing, mirror);
    let mut shadow = Pixels::new(s.size as u32, s.size as u32);
    for (px, a) in shadow.make_mut_slice().iter_mut().zip(&s.alpha) {
        *px = slint::Rgba8Pixel::new(255, 255, 255, *a);
    }
    GaugeView {
        groups,
        drawing,
        mirror,
        redline_index,
        shadow,
        shadow_half: s.half,
    }
}

/// How strong the short ticks of a coloured band are drawn.
fn band_dim(zones: Zones) -> f64 {
    match zones {
        Zones::Charge => 0.85,
        _ => 0.55,
    }
}

/// Maps a tick to its band. Charge runs red below 20, dark orange below 40,
/// yellow below 60 and green above, which matches where the pack gets critical.
fn band_tone(zones: Zones, redline_index: f64, last: f64, i: usize) -> usize {
    match zones {
        Zones::Plain => 0,
        Zones::Redline => (i as f64 >= redline_index) as usize,
        Zones::Charge => match (i as f64 / last * 100.0) as i32 {
            0..=19 => 1,
            20..=39 => 2,
            40..=59 => 3,
            _ => 4,
        },
    }
}

/// The centre readout on a scooter shows the charge as whole percent, no decimals.
fn charge_text(fuel: i32) -> String {
    format!("{}%", fuel.clamp(0, 100))
}

/// The right-hand gauge is the rev counter, except on a scooter, where the pack's
/// state of charge takes its place. A scooter has no redline, so none is drawn.
fn right_gauge(r: &Readings) -> (&Scale, f64, f64) {
    if r.scooter {
        (&r.battery_scale, r.battery_level, 0.0)
    } else {
        (&r.rpm_scale, r.rpm, r.redline)
    }
}

impl DashReader {
    pub fn read(&mut self, state: &State, screen: Screen, dark: bool) -> DashView {
        let r = readings(state, screen);
        let speed_key = (r.speed_scale.clone(), 0);
        let speed_gauge = (self.speed_key.as_ref() != Some(&speed_key)).then(|| {
            self.speed_key = Some(speed_key);
            gauge_view(&r.speed_scale, false, Zones::Plain, 0.0)
        });
        let (gauge_scale, _, gauge_redline) = right_gauge(&r);
        let zones = if r.scooter {
            Zones::Charge
        } else {
            Zones::Redline
        };
        let rpm_key = (gauge_scale.clone(), zones as usize);
        let rpm_gauge = (self.rpm_key.as_ref() != Some(&rpm_key)).then(|| {
            self.rpm_key = Some(rpm_key);
            gauge_view(gauge_scale, true, zones, gauge_redline)
        });
        let base = Spec::default();
        let ring_shift = label_overflow(&base, &r.speed_scale, RING_W, RING_H, -LEFT_RING_LEFT)
            .max(label_overflow(
                &base,
                gauge_scale,
                RING_W,
                RING_H,
                RIGHT_RING_LEFT + RING_W - BASE_W,
            ));
        let p = pages(state, dark);
        let layout = spec(p.active.as_deref().unwrap_or(""));
        let n = nav(state);
        let nav_image_raw = (n.image != self.image).then(|| {
            self.image = n.image.clone();
            n.image.clone()
        });
        DashView {
            readings: r,
            speed_gauge,
            rpm_gauge,
            ring_shift,
            gauges: layout.gauges,
            map: layout.map,
            center: layout.center,
            page_count: p.count,
            page_index: p.index,
            accent: p.accent,
            nav: n,
            nav_image: None,
            nav_image_raw,
            mask: mask(state, Stream::Cluster),
        }
    }
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(items)))
}

fn gauge_data(v: &GaugeView) -> GaugeData {
    let d = &v.drawing;
    GaugeData {
        box_w: RING_W as f32,
        box_h: RING_H as f32,
        bands: model(
            v.groups
                .bands
                .iter()
                .map(|b| GaugeBand {
                    minor: b.minor.clone().into(),
                    major: b.major.clone().into(),
                    tone: b.tone as i32,
                    minor_opacity: b.minor_opacity as f32,
                })
                .collect::<Vec<_>>(),
        ),
        caps: model(
            v.groups
                .caps
                .iter()
                .map(|(path, opacity)| CapData {
                    path: path.clone().into(),
                    opacity: *opacity as f32,
                })
                .collect::<Vec<_>>(),
        ),
        trail: model(
            d.trail
                .iter()
                .map(|b| TrailData {
                    path: b.path().into(),
                    index: b.arc_index.unwrap_or(0) as i32,
                })
                .collect::<Vec<_>>(),
        ),
        labels: model(
            d.labels
                .iter()
                .map(|l| LabelData {
                    x: l.x as f32,
                    y: l.y as f32,
                    text: l.text.clone().into(),
                })
                .collect::<Vec<_>>(),
        ),
        label_size: d.label_size as f32,
        cx: d.center_x as f32,
        cy: d.center_y as f32,
        radius: d.radius as f32,
        gap: Spec::default().gap_deg as f32,
        mirror: v.mirror,
        fit: d.fit as f32,
        tick_w: (Spec::default().tick_w * d.fit) as f32,
        tick_h: (Spec::default().tick_h * d.fit) as f32,
        major_h: (Spec::default().major_h * d.fit) as f32,
        ticks: d.arc_ticks as i32,
        redline_index: v.redline_index.min(1e6) as f32,
        shadow: Image::from_rgba8(v.shadow.clone()),
        shadow_half: v.shadow_half as f32,
    }
}

fn hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).ok()?;
    (s.len() == 6).then(|| Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub fn mask_data(mask: Option<Mask>) -> MaskData {
    let m = mask.unwrap_or_default();
    let int = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
    MaskData {
        shown: mask.is_some(),
        width: int(m.width),
        height: int(m.height),
        top: int(m.top),
        bottom: int(m.bottom),
        left: int(m.left),
        right: int(m.right),
    }
}

pub fn map_shown(ui: &AppWindow) -> bool {
    let d = ui.global::<DashState>();
    d.get_enabled() && d.get_map()
}

pub fn apply(ui: &AppWindow, v: DashView) {
    fill(&ui.global::<DashState>(), v);
}

pub fn fill(d: &DashState<'_>, mut v: DashView) {
    let r = &v.readings;
    d.set_enabled(r.enabled);
    d.set_gauges(v.gauges);
    d.set_map(v.map);
    d.set_dev_fake_map(r.dev_fake_map);
    d.set_mask(mask_data(v.mask));
    d.set_center(match v.center {
        Center::None => 0,
        Center::NavMini => 1,
        Center::NavFull => 2,
    });
    if let Some(g) = &v.speed_gauge {
        d.set_speed_gauge(gauge_data(g));
    }
    if let Some(g) = &v.rpm_gauge {
        d.set_rpm_gauge(gauge_data(g));
    }
    d.set_speed_t((r.speed / r.speed_scale.max.max(1.0)).clamp(0.0, 1.0) as f32);
    // A mark past the end of the scale has no place on it, so it is left out
    // rather than drawn somewhere it does not belong.
    let marks: Vec<f32> = r
        .speed_markers
        .iter()
        .map(|m| m / r.speed_scale.max.max(1.0))
        .filter(|t| *t > 0.0 && *t <= 1.0)
        .map(|t| t as f32)
        .collect();
    d.set_speed_markers(ModelRc::from(Rc::new(VecModel::from(marks))));
    let (gauge_scale, gauge_value, _) = right_gauge(r);
    d.set_rpm_t((gauge_value / gauge_scale.max.max(1.0)).clamp(0.0, 1.0) as f32);
    d.set_ring_shift(v.ring_shift as f32);
    d.set_speed_text(r.speed_text.clone().into());
    d.set_speed_label(r.speed_label.into());
    d.set_speed_note(r.odometer.clone().into());
    d.set_drive_level(i32::from(r.drive_level));
    d.set_gear(r.gear.clone().into());
    d.set_charge_text(charge_text(r.fuel).into());
    // Without a measurement there is nothing to show but the label, which is
    // what a scooter reads until its pack sensor reports.
    let energy = pack_text(r.battery_wh, r.pack_voltage);
    d.set_charge_energy(energy.into());
    d.set_turn_left(r.turn == Turn::Left);
    d.set_turn_right(r.turn == Turn::Right);
    d.set_hazards(r.hazards);
    d.set_lights(r.lights);
    d.set_high_beam(r.high_beam);
    d.set_parking_brake(r.parking_brake);
    d.set_ambient(r.ambient.clone().unwrap_or_default().into());
    d.set_fuel(r.fuel);
    d.set_fuel_low(r.fuel_low);
    d.set_battery(r.battery);
    d.set_scooter(r.scooter);
    d.set_oil_text(r.oil_text.clone().into());
    d.set_oil_ratio(r.oil_ratio as f32);
    d.set_oil_hot(r.oil_hot);
    let accent = v.accent.as_deref().and_then(hex);
    d.set_has_accent(accent.is_some());
    d.set_accent(accent.unwrap_or_default());
    d.set_page_count(v.page_count as i32);
    d.set_page_index(v.page_index.map_or(-1, |i| i as i32));
    let n = &v.nav;
    d.set_nav_active(n.active);
    d.set_nav_icon(crate::icons::icon(n.icon).unwrap_or_default());
    if let Some(img) = v.nav_image.take() {
        d.set_nav_has_image(img.is_some());
        d.set_nav_image(img.map(Image::from_rgba8).unwrap_or_default());
    }
    d.set_nav_exit(n.roundabout_exit.map_or(0, |e| e as i32));
    let s = |t: &str| SharedString::from(t);
    d.set_nav_distance(s(&n.distance));
    d.set_nav_maneuver(s(&n.maneuver));
    d.set_nav_road(s(&n.road));
    d.set_nav_after_road(s(&n.after_road));
    d.set_nav_time_left(s(&n.time_left));
    d.set_nav_destination_distance(s(&n.destination_distance));
    d.set_nav_destination(s(&n.destination));
    d.set_nav_app(s(&n.app));
}

pub fn wire(d: &DashState<'_>, client: &Client) {
    d.on_step({
        let c = client.clone();
        move |by| c.act(Action::StepClusterLayout { by })
    });
    d.on_select_page({
        let c = client.clone();
        move |i| {
            let layout = usize::try_from(i)
                .ok()
                .and_then(|i| c.mirror().state.cluster.shown.get(i).cloned());
            if let Some(layout) = layout {
                c.act(Action::SelectClusterLayout { layout });
            }
        }
    });
}
