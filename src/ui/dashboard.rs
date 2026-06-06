use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use adw::Clamp;
use gtk4::{gdk, prelude::*};
use gtk4::{Align, Box as GtkBox, FlowBox, Label, Orientation, ScrolledWindow, SelectionMode};

use super::{
    aqi_widget::AQIWidget, humidity_widget::HumidityWidget, sensor_card::SensorCard,
    temperature_widget::TemperatureWidget,
};
use crate::sensors::{
    thresholds::{
        aqi_status_color, co2_status_color, nox_status_color, pm25_status_color, tvoc_status_color,
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
    history: Rc<RefCell<VecDeque<AirMeasureSnapshot>>>,
}

impl DashboardPageWidgets {
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
            snapshot
                .co2
                .map(co2_status_color)
                .unwrap_or(rgba_u8(154, 153, 150)),
        );
        self.co2_card.set_trend(
            snapshot.co2,
            previous.as_ref().and_then(|snapshot| snapshot.co2),
            "ppm",
        );
        self.tvoc_card.refresh(
            snapshot.tvoc,
            snapshot.tvoc_unit,
            snapshot
                .tvoc
                .map(tvoc_status_color)
                .unwrap_or(rgba_u8(154, 153, 150)),
        );
        self.tvoc_card.set_trend(
            snapshot.tvoc,
            previous.as_ref().and_then(|snapshot| snapshot.tvoc),
            snapshot.tvoc_unit.unwrap_or("index"),
        );
        self.nox_card.refresh(
            snapshot.nox,
            snapshot.nox_unit,
            snapshot
                .nox
                .map(nox_status_color)
                .unwrap_or(rgba_u8(154, 153, 150)),
        );
        self.nox_card.set_trend(
            snapshot.nox,
            previous.as_ref().and_then(|snapshot| snapshot.nox),
            snapshot.nox_unit.unwrap_or("index"),
        );
        self.pm003_count_card
            .refresh(snapshot.pm003_count, Some("count"), rgba_u8(53, 132, 228));
        self.pm003_count_card.set_trend(
            snapshot.pm003_count,
            previous.as_ref().and_then(|snapshot| snapshot.pm003_count),
            "count",
        );
        self.pm1_card
            .refresh(snapshot.pm1, Some("µg/m³"), rgba_u8(98, 160, 234));
        self.pm1_card.set_trend(
            snapshot.pm1,
            previous.as_ref().and_then(|snapshot| snapshot.pm1),
            "µg/m³",
        );
        self.pm25_card.refresh(
            snapshot.pm25,
            Some("µg/m³"),
            snapshot
                .pm25
                .map(pm25_status_color)
                .unwrap_or(rgba_u8(154, 153, 150)),
        );
        self.pm25_card.set_trend(
            snapshot.pm25,
            previous.as_ref().and_then(|snapshot| snapshot.pm25),
            "µg/m³",
        );
        self.pm10_card
            .refresh(snapshot.pm10, Some("µg/m³"), rgba_u8(255, 163, 72));
        self.pm10_card.set_trend(
            snapshot.pm10,
            previous.as_ref().and_then(|snapshot| snapshot.pm10),
            "µg/m³",
        );

        self.aqi_widget.refresh(
            snapshot.aqi,
            snapshot
                .aqi
                .map(aqi_status_color)
                .unwrap_or(rgba_u8(154, 153, 150)),
        );
        self.aqi_widget.set_trend(
            snapshot.aqi,
            previous.as_ref().and_then(|snapshot| snapshot.aqi),
        );

        {
            let mut history = self.history.borrow_mut();
            history.push_back(snapshot.clone());
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
