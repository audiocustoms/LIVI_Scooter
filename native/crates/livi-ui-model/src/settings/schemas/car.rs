// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

use livi_core_proto::config::ScooterPack;

use crate::settings::node::{
    CustomPage, Label, Node, OptionSource, OptionValue, Shown, UnitSource, choice, number_choice,
    text_choice,
};

const CAR_TYPES: [(&str, &str, f64); 14] = [
    ("Gasoline", "settings.carTypeGasoline", 1.0),
    ("Diesel", "settings.carTypeDiesel", 4.0),
    ("Electric", "settings.carTypeElectric", 10.0),
    ("Scooter", "settings.carTypeScooter", 13.0),
    ("HybridGasoline", "settings.carTypeHybridGasoline", 101.0),
    ("HybridDiesel", "settings.carTypeHybridDiesel", 102.0),
    ("E85", "settings.carTypeE85", 6.0),
    ("Biodiesel", "settings.carTypeBiodiesel", 5.0),
    ("DieselWinter", "settings.carTypeDieselWinter", 3.0),
    ("LPG", "settings.carTypeLPG", 7.0),
    ("CNG", "settings.carTypeCNG", 8.0),
    ("LNG", "settings.carTypeLNG", 9.0),
    ("Hydrogen", "settings.carTypeHydrogen", 11.0),
    ("Other", "settings.carTypeOther", 12.0),
];

const HAND_LHD: f64 = 0.0;
const HAND_RHD: f64 = 1.0;

/// The packs a scooter can run on, with the label shown for each.
const SCOOTER_PACKS: [(ScooterPack, &str, &str); 5] = [
    (ScooterPack::V12, "settings.scooterVoltage12", "12V"),
    (ScooterPack::V36, "settings.scooterVoltage36", "36V"),
    (ScooterPack::V48, "settings.scooterVoltage48", "48V"),
    (ScooterPack::V60, "settings.scooterVoltage60", "60V"),
    (ScooterPack::V72, "settings.scooterVoltage72", "72V"),
];

/// Capacities packs are sold in, plus the bench pack used to bring up the
/// monitor. Fixed steps on purpose: a pack without a BMS has no exact figure to
/// type in anyway.
const SCOOTER_CAPACITIES: [(f64, &str, &str); 7] = [
    (4.6, "settings.scooterCapacity4p6", "4.6 Ah"),
    (10.0, "settings.scooterCapacity10", "10 Ah"),
    (15.0, "settings.scooterCapacity15", "15 Ah"),
    (20.0, "settings.scooterCapacity20", "20 Ah"),
    (30.0, "settings.scooterCapacity30", "30 Ah"),
    (40.0, "settings.scooterCapacity40", "40 Ah"),
    (50.0, "settings.scooterCapacity50", "50 Ah"),
];

fn carplay() -> Node {
    Node::route(
        "theme",
        Label::new("settings.carMakerTheme", "Car Maker Theme"),
        vec![
            Node::text(Label::new("settings.carPlayTheme", "Theme"), "carPlayTheme")
                .icon("uiColors")
                .display_value()
                .page(Label::new("settings.carPlayTheme", "Theme")),
            Node::text(
                Label::new("settings.carPlayVehicleModel", "Vehicle Model"),
                "carPlayVehicleModel",
            )
            .icon("car")
            .display_value()
            .page(Label::new("settings.carPlayVehicleModel", "Vehicle Model")),
        ],
    )
    .icon("wirelessCp")
}

/// The three marks a driver can put on the speed scale, each its own selection.
const SPEED_MARKERS: [(&str, &str, &str); 3] = [
    ("speedMarkerKph", "settings.speedMarker1", "Marker 1"),
    ("speedMarker2Kph", "settings.speedMarker2", "Marker 2"),
    ("speedMarker3Kph", "settings.speedMarker3", "Marker 3"),
];

fn speed_markers() -> Node {
    Node::route(
        "speedMarker",
        Label::new("settings.speedMarker", "Speed Marker"),
        SPEED_MARKERS
            .iter()
            .map(|&(path, key, text)| {
                Node::select(Label::new(key, text), path, Vec::new())
                    .icon("speedMarker")
                    .options_from(OptionSource::SpeedMarker(path))
                    .display_value()
                    .page(Label::new(key, text))
            })
            .collect(),
    )
    .icon("speedMarker")
}

pub fn schema() -> Node {
    Node::route(
        "car",
        Label::new("settings.car", "Car"),
        vec![
            Node::select(
                Label::new("settings.carType", "Fuel Type"),
                "carType",
                CAR_TYPES
                    .iter()
                    .map(|&(name, key, value)| {
                        choice(Label::new(key, name), OptionValue::Number(value))
                    })
                    .collect(),
            )
            .icon("carType")
            .display_value()
            .page(Label::new("settings.carType", "Fuel Type")),
            Node::select(
                Label::new("settings.steeringWheelPosition", "Steering wheel position"),
                "hand",
                vec![
                    choice(
                        Label::new("settings.lhdr", "LHD"),
                        OptionValue::Number(HAND_LHD),
                    ),
                    choice(
                        Label::new("settings.rhdr", "RHD"),
                        OptionValue::Number(HAND_RHD),
                    ),
                ],
            )
            .icon("steering")
            .display_value()
            .page(Label::new(
                "settings.steeringWheelPosition",
                "Steering wheel position",
            )),
            Node::number(Label::new("settings.maxSpeed", "Max Speed"), "maxSpeedKph")
                .icon("maxSpeed")
                .range(Some(60.0), Some(400.0), None)
                .unit(UnitSource::Speed)
                .display_value()
                .page(Label::new("settings.maxSpeed", "Max Speed")),
            Node::select(
                Label::new("settings.speedScaleStep", "Scale Step"),
                "speedScaleStep",
                vec![
                    number_choice("10", 10.0),
                    number_choice("20", 20.0),
                    number_choice("40", 40.0),
                ],
            )
            .icon("speedScaleStep")
            .display_value()
            .page(Label::new("settings.speedScaleStep", "Scale Step")),
            Node::number(Label::new("settings.maxRpm", "Max RPM"), "maxRpm")
                .icon("maxRpm")
                .range(Some(3000.0), Some(20000.0), Some(1000.0))
                .display_value()
                .shown(Shown::Suffix(" rpm"))
                .page(Label::new("settings.maxRpm", "Max RPM")),
            Node::number(Label::new("settings.redlineRpm", "Redline"), "redlineRpm")
                .icon("redlineRpm")
                .range(Some(0.0), Some(20000.0), Some(100.0))
                .display_value()
                .shown(Shown::Suffix(" rpm"))
                .page(Label::new("settings.redlineRpm", "Redline")),
            speed_markers(),
            Node::select(
                Label::new("settings.scooterBatteryVoltage", "Scooter Battery Voltage"),
                "scooterBatteryVoltage",
                SCOOTER_PACKS
                    .iter()
                    .map(|&(pack, key, text)| {
                        choice(
                            Label::new(key, text),
                            OptionValue::Number(f64::from(pack.volts())),
                        )
                    })
                    .collect(),
            )
            .icon("scooterBatteryVoltage")
            .display_value()
            .page(Label::new(
                "settings.scooterBatteryVoltage",
                "Scooter Battery Voltage",
            )),
            Node::select(
                Label::new(
                    "settings.scooterBatteryCapacity",
                    "Scooter Battery Capacity",
                ),
                "scooterBatteryCapacityAh",
                SCOOTER_CAPACITIES
                    .iter()
                    .map(|&(ah, key, text)| choice(Label::new(key, text), OptionValue::Number(ah)))
                    .collect(),
            )
            .icon("scooterBatteryCapacity")
            .display_value()
            .page(Label::new(
                "settings.scooterBatteryCapacity",
                "Scooter Battery Capacity",
            )),
            Node::select(
                Label::new("settings.speedUnit", "Speed Unit"),
                "speedUnit",
                vec![text_choice("km/h", "kmh"), text_choice("mph", "mph")],
            )
            .icon("speedUnit")
            .display_value()
            .page(Label::new("settings.speedUnit", "Speed Unit")),
            Node::select(
                Label::new("settings.temperatureUnit", "Temperature Unit"),
                "temperatureUnit",
                vec![
                    text_choice("°C", "celsius"),
                    text_choice("°F", "fahrenheit"),
                ],
            )
            .icon("temperatureUnit")
            .display_value()
            .page(Label::new("settings.temperatureUnit", "Temperature Unit")),
            Node::select(
                Label::new("settings.carPlayDockPosition", "CarPlay Dock"),
                "carPlayDockPosition",
                vec![
                    choice(
                        Label::new("settings.carPlayDockBottom", "Bottom"),
                        OptionValue::Text("bottom"),
                    ),
                    choice(
                        Label::new("settings.carPlayDockSide", "Side"),
                        OptionValue::Text("side"),
                    ),
                    choice(
                        Label::new("settings.carPlayDockAuto", "Automatic"),
                        OptionValue::Text("auto"),
                    ),
                ],
            )
            .icon("edgeBottom")
            .display_value()
            .page(Label::new("settings.carPlayDockPosition", "CarPlay Dock")),
            Node::checkbox(
                Label::new("settings.carPlayVehicleClimate", "CarPlay Climate Controls"),
                "carPlayVehicleClimate",
            )
            .icon("carPlayVehicleClimate"),
            Node::custom(
                Label::new("settings.carData", "Car Data"),
                "carType",
                CustomPage::CarReadings,
            ),
            carplay(),
        ],
    )
    .icon("car")
}
