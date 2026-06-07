//! Dashboard page and measurement application.
//!
//! This module builds the dashboard layout and owns the widget handles needed to
//! refresh values after each successful HTTP fetch.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use adw::Clamp;
use gtk4::{gdk, prelude::*};
use gtk4::{Align, Box as GtkBox, FlowBox, Label, Orientation, ScrolledWindow, SelectionMode};

use super::{
    aqi_widget::AQIWidget,
    humidity_widget::HumidityWidget,
    sensor_card::{PresentationStatus, SensorCard},
    temperature_widget::TemperatureWidget,
};
use crate::sensors::{
    thresholds::{
        aqi_status_color, co2_status_color, nox_status_color, pm25_status_color, tvoc_status_color,
        StatusColor,
    },
    AirMeasureSnapshot,
};

fn rgba_u8(r: u8, g: u8, b: u8) -> gdk::RGBA {
    gdk::RGBA::new(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        1.0,
    )
}

fn status_rgba(color: StatusColor) -> gdk::RGBA {
    match color {
        StatusColor::Green => rgba_u8(51, 209, 122),
        StatusColor::Yellow => rgba_u8(245, 194, 17),
        StatusColor::Orange => rgba_u8(255, 120, 0),
        StatusColor::Red => rgba_u8(237, 51, 59),
        StatusColor::Purple => rgba_u8(145, 65, 172),
        StatusColor::Gray => rgba_u8(94, 92, 100),
    }
}

fn fixed_status(class: &'static str, r: u8, g: u8, b: u8) -> PresentationStatus {
    PresentationStatus::new(class, rgba_u8(r, g, b))
}

fn presentation_status(
    value: Option<f32>,
    classify: impl FnOnce(f32) -> StatusColor,
) -> PresentationStatus {
    value
        .map(classify)
        .map(|color| match color {
            StatusColor::Green => PresentationStatus::green(status_rgba(color)),
            StatusColor::Yellow => PresentationStatus::yellow(status_rgba(color)),
            StatusColor::Orange => PresentationStatus::orange(status_rgba(color)),
            StatusColor::Red => PresentationStatus::red(status_rgba(color)),
            StatusColor::Purple | StatusColor::Gray => {
                PresentationStatus::unknown(status_rgba(color))
            }
        })
        .unwrap_or_else(|| PresentationStatus::unknown(rgba_u8(154, 153, 150)))
}

#[derive(Clone)]
pub struct DashboardPageWidgets {
    pub server_label: Label,
    pub fetch_status_label: Label,
    pub temperature_widget: TemperatureWidget,
    pub humidity_widget: HumidityWidget,
    pub aqi_widget: AQIWidget,
    pub co2_card: SensorCard,
    pub nox_card: SensorCard,
    pub tvoc_card: SensorCard,
    pub pm003_count_card: SensorCard,
    pub pm1_card: SensorCard,
    pub pm25_card: SensorCard,
    pub pm10_card: SensorCard,
    /// Last few measurements kept only in memory for trend calculations.
    history: Rc<RefCell<VecDeque<AirMeasureSnapshot>>>,
}

impl DashboardPageWidgets {
    /// Apply a parsed measurement snapshot to every dashboard widget.
    ///
    /// The dashboard compares the new snapshot with the previous one to show
    /// trend labels. GTK widgets are reference-counted objects, so this struct
    /// can be cloned into callbacks and still update the same visible widgets.
    pub fn apply_measurements(&self, snapshot: &AirMeasureSnapshot) {
        let previous = self.history.borrow().back().cloned();

        self.temperature_widget
            .refresh(snapshot.temperature, rgba_u8(53, 132, 228));
        self.temperature_widget.set_trend(
            snapshot.temperature,
            previous.as_ref().and_then(|snapshot| snapshot.temperature),
        );
        self.humidity_widget
            .refresh(snapshot.humidity, rgba_u8(38, 162, 105));
        self.humidity_widget.set_trend(
            snapshot.humidity,
            previous.as_ref().and_then(|snapshot| snapshot.humidity),
        );

        self.co2_card.refresh(
            snapshot.co2,
            Some("ppm"),
            presentation_status(snapshot.co2, co2_status_color),
        );
        self.co2_card.set_trend(
            snapshot.co2,
            previous.as_ref().and_then(|snapshot| snapshot.co2),
            "ppm",
        );
        self.tvoc_card.refresh(
            snapshot.tvoc,
            snapshot.tvoc_unit,
            presentation_status(snapshot.tvoc, tvoc_status_color),
        );
        self.tvoc_card.set_trend(
            snapshot.tvoc,
            previous.as_ref().and_then(|snapshot| snapshot.tvoc),
            snapshot.tvoc_unit.unwrap_or("index"),
        );
        self.nox_card.refresh(
            snapshot.nox,
            snapshot.nox_unit,
            presentation_status(snapshot.nox, nox_status_color),
        );
        self.nox_card.set_trend(
            snapshot.nox,
            previous.as_ref().and_then(|snapshot| snapshot.nox),
            snapshot.nox_unit.unwrap_or("index"),
        );
        self.pm003_count_card.refresh(
            snapshot.pm003_count,
            Some("count"),
            fixed_status("status-blue", 53, 132, 228),
        );
        self.pm003_count_card.set_trend(
            snapshot.pm003_count,
            previous.as_ref().and_then(|snapshot| snapshot.pm003_count),
            "count",
        );
        self.pm1_card.refresh(
            snapshot.pm1,
            Some("µg/m³"),
            fixed_status("status-blue", 98, 160, 234),
        );
        self.pm1_card.set_trend(
            snapshot.pm1,
            previous.as_ref().and_then(|snapshot| snapshot.pm1),
            "µg/m³",
        );
        self.pm25_card.refresh(
            snapshot.pm25,
            Some("µg/m³"),
            presentation_status(snapshot.pm25, pm25_status_color),
        );
        self.pm25_card.set_trend(
            snapshot.pm25,
            previous.as_ref().and_then(|snapshot| snapshot.pm25),
            "µg/m³",
        );
        self.pm10_card.refresh(
            snapshot.pm10,
            Some("µg/m³"),
            fixed_status("status-orange", 255, 163, 72),
        );
        self.pm10_card.set_trend(
            snapshot.pm10,
            previous.as_ref().and_then(|snapshot| snapshot.pm10),
            "µg/m³",
        );

        self.aqi_widget.refresh(
            snapshot.aqi,
            presentation_status(snapshot.aqi, aqi_status_color).color(),
        );
        self.aqi_widget.set_trend(
            snapshot.aqi,
            previous.as_ref().and_then(|snapshot| snapshot.aqi),
        );

        {
            let mut history = self.history.borrow_mut();
            history.push_back(snapshot.clone());
            // Keep a tiny rolling history. Today we need only the previous
            // value, but keeping five samples leaves room for "vs 5 min ago"
            // style labels later without changing this storage shape.
            while history.len() > 5 {
                history.pop_front();
            }
        }

        self.fetch_status_label
            .set_text("Latest measurements loaded.");
    }
}

pub fn build_dashboard_page() -> (GtkBox, DashboardPageWidgets) {
    let page = GtkBox::new(Orientation::Vertical, 0);
    page.set_vexpand(true);

    let scroller = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .build();
    // `Clamp` is a libadwaita layout helper. It keeps content readable on wide
    // screens instead of stretching cards across the entire window.
    let clamp = Clamp::builder()
        .maximum_size(960)
        .tightening_threshold(600)
        .hexpand(true)
        .build();
    let content = GtkBox::new(Orientation::Vertical, 18);
    content.add_css_class("dashboard-page");
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);

    let server_label = Label::builder()
        .label("Server URL: Not configured")
        .halign(Align::Start)
        .build();
    server_label.add_css_class("dim-label");

    let fetch_status_label = Label::builder()
        .label("Press refresh to load current measurements.")
        .halign(Align::Start)
        .build();
    fetch_status_label.add_css_class("dim-label");

    let top_row = GtkBox::new(Orientation::Horizontal, 8);
    top_row.add_css_class("dashboard-top");
    top_row.set_hexpand(true);
    top_row.set_homogeneous(true);
    let env_stack = GtkBox::new(Orientation::Vertical, 8);
    env_stack.add_css_class("environment-stack");
    env_stack.set_hexpand(true);
    let temperature_widget = TemperatureWidget::new();
    let humidity_widget = HumidityWidget::new();
    let aqi_widget = AQIWidget::new();
    top_row.append(&aqi_widget.widget());
    env_stack.append(&temperature_widget.widget());
    env_stack.append(&humidity_widget.widget());
    top_row.append(&env_stack);

    let co2_card = SensorCard::new("CO₂", "ppm", "airgradient-co2-symbolic");
    let tvoc_card = SensorCard::new("TVOC", "ppb", "airgradient-voc-symbolic");
    let nox_card = SensorCard::new("NOx", "ppb", "airgradient-nox-symbolic");
    for card in [&co2_card, &tvoc_card, &nox_card] {
        // Gas cards should fit three across on the second dashboard row.
        card.set_narrow();
    }
    let gas_row = build_card_flow(
        &[co2_card.widget(), tvoc_card.widget(), nox_card.widget()],
        3,
    );
    gas_row.add_css_class("gas-row");

    let pm1_card = SensorCard::new("PM₁.₀", "µg/m³", "airgradient-particles-symbolic");
    let pm25_card = SensorCard::new("PM₂.₅", "µg/m³", "airgradient-particles-symbolic");
    let pm10_card = SensorCard::new("PM₁₀", "µg/m³", "airgradient-particles-symbolic");
    let pm003_count_card =
        SensorCard::new("PM₀.₃ Count", "count", "airgradient-particles-symbolic");
    for card in [&pm003_count_card, &pm1_card, &pm25_card, &pm10_card] {
        // PM cards use the most compact variant so all particle metrics fit on
        // one row at the default window width.
        card.set_compact();
    }
    let particles_row = build_card_flow(
        &[
            pm003_count_card.widget(),
            pm1_card.widget(),
            pm25_card.widget(),
            pm10_card.widget(),
        ],
        4,
    );
    particles_row.add_css_class("particles-row");

    content.append(&server_label);
    content.append(&top_row);
    content.append(&gas_row);
    content.append(&particles_row);
    content.append(&fetch_status_label);
    clamp.set_child(Some(&content));
    scroller.set_child(Some(&clamp));
    page.append(&scroller);

    let widgets = DashboardPageWidgets {
        server_label,
        fetch_status_label,
        temperature_widget,
        humidity_widget,
        aqi_widget,
        co2_card,
        nox_card,
        tvoc_card,
        pm003_count_card,
        pm1_card,
        pm25_card,
        pm10_card,
        history: Rc::new(RefCell::new(VecDeque::with_capacity(5))),
    };

    (page, widgets)
}

fn build_card_flow(cards: &[GtkBox], max_per_line: u32) -> FlowBox {
    // FlowBox gives us a responsive grid-like row without manually calculating
    // columns. At narrow widths it can wrap cards instead of overflowing.
    let flow = FlowBox::builder()
        .row_spacing(12)
        .column_spacing(12)
        .hexpand(true)
        .halign(Align::Fill)
        .valign(Align::Start)
        .selection_mode(SelectionMode::None)
        .min_children_per_line(1)
        .max_children_per_line(max_per_line)
        .homogeneous(true)
        .build();

    for card in cards {
        flow.insert(card, -1);
    }
    flow
}
