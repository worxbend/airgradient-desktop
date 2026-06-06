use gtk4::{gdk, prelude::*};
use gtk4::{Align, Box as GtkBox, Image, Label, Orientation};

use super::sensor_card::{format_metric_value, update_trend_labels};

#[derive(Clone)]
pub struct AQIWidget {
    root: GtkBox,
    aqi_label: Label,
    level_label: Label,
    description_label: Label,
    trend_label: Label,
    trend_context_label: Label,
}

impl AQIWidget {
    pub fn new() -> Self {
        let root = GtkBox::new(Orientation::Horizontal, 18);
        root.add_css_class("card");
        root.add_css_class("aqi-card");
        root.add_css_class("aqi-good");
        root.set_hexpand(true);
        root.set_vexpand(true);

        let icon = Image::from_icon_name("airgradient-air-quality-symbolic");
        icon.set_pixel_size(56);
        icon.set_halign(Align::Center);
        icon.set_valign(Align::Center);
        let icon_box = GtkBox::new(Orientation::Vertical, 0);
        icon_box.add_css_class("aqi-icon");
        icon_box.set_valign(Align::Center);
        icon_box.append(&icon);

        let content = GtkBox::new(Orientation::Vertical, 4);
        content.set_hexpand(true);
        content.set_valign(Align::Center);

        let title = Label::builder()
            .label("Air Quality Index")
            .halign(Align::Start)
            .build();
        title.add_css_class("metric-title");

        let value_row = GtkBox::new(Orientation::Horizontal, 12);
        value_row.set_valign(Align::Baseline);

        let aqi_label = Label::new(Some("--"));
        aqi_label.add_css_class("aqi-value");
        aqi_label.set_halign(Align::Start);

        let level_label = Label::builder()
            .label("Unknown")
            .halign(Align::Start)
            .build();
        level_label.add_css_class("aqi-level");

        value_row.append(&aqi_label);
        value_row.append(&level_label);

        let description_label = Label::builder()
            .label("Waiting for a measurement.")
            .halign(Align::Start)
            .wrap(true)
            .build();
        description_label.add_css_class("metric-unit");

        let trend_label = Label::builder()
            .label("No previous reading")
            .halign(Align::Start)
            .build();
        trend_label.add_css_class("trend-value");
        trend_label.add_css_class("trend-neutral");
        let trend_context_label = Label::builder()
            .label("from last reading")
            .halign(Align::Start)
            .build();
        trend_context_label.add_css_class("metric-unit");
        trend_context_label.add_css_class("trend-context");

        let trend_box = GtkBox::new(Orientation::Horizontal, 8);
        trend_box.set_margin_top(6);
        trend_box.append(&trend_label);
        trend_box.append(&trend_context_label);

        content.append(&title);
        content.append(&value_row);
        content.append(&description_label);
        content.append(&trend_box);

        root.append(&icon_box);
        root.append(&content);

        Self {
            root,
            aqi_label,
            level_label,
            description_label,
            trend_label,
            trend_context_label,
        }
    }

    pub fn widget(&self) -> GtkBox {
        self.root.clone()
    }

    pub fn set_value(&self, value: Option<f32>) {
        match value {
            Some(v) => {
                let level = aqi_level(v);
                self.aqi_label.set_text(&format_metric_value(v));
                self.level_label.set_text(level);
                self.description_label.set_text(aqi_description(v));
            }
            None => {
                self.aqi_label.set_text("--");
                self.level_label.set_text("Unknown");
                self.description_label
                    .set_text("Waiting for a measurement.");
            }
        }
    }

    pub fn set_trend(&self, current: Option<f32>, previous: Option<f32>) {
        update_trend_labels(
            &self.trend_label,
            &self.trend_context_label,
            current,
            previous,
            "AQI",
            true,
        );
    }

    pub fn update_status(&self, value: Option<f32>) {
        const LEVEL_CLASSES: [&str; 6] = [
            "aqi-good",
            "aqi-moderate",
            "aqi-sensitive",
            "aqi-unhealthy",
            "aqi-very-unhealthy",
            "aqi-hazardous",
        ];
        for class in LEVEL_CLASSES {
            self.root.remove_css_class(class);
        }
        let level = value.unwrap_or(0.0);
        let cls = if level <= 50.0 {
            "aqi-good"
        } else if level <= 100.0 {
            "aqi-moderate"
        } else if level <= 150.0 {
            "aqi-sensitive"
        } else if level <= 200.0 {
            "aqi-unhealthy"
        } else if level <= 300.0 {
            "aqi-very-unhealthy"
        } else {
            "aqi-hazardous"
        };
        self.root.add_css_class(cls);
    }

    pub fn refresh(&self, value: Option<f32>, _status_color: gdk::RGBA) {
        self.set_value(value);
        self.update_status(value);
    }
}

fn aqi_level(value: f32) -> &'static str {
    if value <= 50.0 {
        "Good"
    } else if value <= 100.0 {
        "Moderate"
    } else if value <= 150.0 {
        "Unhealthy for Sensitive Groups"
    } else if value <= 200.0 {
        "Unhealthy"
    } else if value <= 300.0 {
        "Very Unhealthy"
    } else {
        "Hazardous"
    }
}

fn aqi_description(value: f32) -> &'static str {
    if value <= 50.0 {
        "Air quality is satisfactory, and air pollution poses little or no risk."
    } else if value <= 100.0 {
        "Air quality is acceptable, but unusually sensitive people may notice effects."
    } else if value <= 150.0 {
        "Sensitive groups may experience health effects."
    } else if value <= 200.0 {
        "Some members of the general public may experience health effects."
    } else if value <= 300.0 {
        "Health alert: risk of health effects is increased for everyone."
    } else {
        "Health warning: everyone is more likely to be affected."
    }
}
