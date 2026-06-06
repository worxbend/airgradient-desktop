use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Image, Label, Orientation};

use super::sensor_card::update_trend_labels;

#[derive(Clone)]
pub struct HumidityWidget {
    root: GtkBox,
    value_label: Label,
    comfort_label: Label,
    trend_label: Label,
    trend_context_label: Label,
    icon: Image,
}

impl HumidityWidget {
    pub fn new() -> Self {
        let root = GtkBox::new(Orientation::Horizontal, 16);
        root.add_css_class("card");
        root.add_css_class("metric-card");
        root.add_css_class("humidity-widget");
        root.set_hexpand(true);
        root.set_vexpand(true);

        let icon = Image::from_icon_name("airgradient-humidity-symbolic");
        icon.set_pixel_size(40);
        icon.set_tooltip_text(Some("Humidity"));
        icon.set_halign(Align::Center);
        icon.set_valign(Align::Center);

        let icon_wrapper = GtkBox::new(Orientation::Vertical, 0);
        icon_wrapper.add_css_class("metric-icon");
        icon_wrapper.set_valign(Align::Center);
        icon_wrapper.append(&icon);

        let content = GtkBox::new(Orientation::Vertical, 4);
        content.set_hexpand(true);
        content.set_valign(Align::Center);

        let title = Label::builder()
            .label("Humidity")
            .halign(Align::Start)
            .build();
        title.add_css_class("metric-title");

        let value_label = Label::builder().label("--").halign(Align::Start).build();
        value_label.add_css_class("large-value");

        let comfort_label = Label::builder()
            .label("Comfort: --")
            .halign(Align::Start)
            .build();
        comfort_label.add_css_class("metric-unit");

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
        trend_box.set_margin_top(4);
        trend_box.append(&trend_label);
        trend_box.append(&trend_context_label);

        content.append(&title);
        content.append(&value_label);
        content.append(&comfort_label);
        content.append(&trend_box);

        root.append(&icon_wrapper);
        root.append(&content);

        Self {
            root,
            value_label,
            comfort_label,
            trend_label,
            trend_context_label,
            icon,
        }
    }

    pub fn widget(&self) -> GtkBox {
        self.root.clone()
    }

    pub fn set_value(&self, value: Option<f32>) {
        match value {
            Some(value) => {
                self.value_label.set_text(&format!("{value:.0}%"));
                let comfort = if value < 40.0 {
                    "Dry"
                } else if value <= 60.0 {
                    "Comfortable"
                } else {
                    "Humid"
                };
                self.comfort_label.set_text(&format!("Comfort: {comfort}"));
            }
            None => {
                self.value_label.set_text("--");
                self.comfort_label.set_text("Comfort: --");
            }
        }
    }

    pub fn set_trend(&self, current: Option<f32>, previous: Option<f32>) {
        update_trend_labels(
            &self.trend_label,
            &self.trend_context_label,
            current,
            previous,
            "%",
            true,
        );
    }

    pub fn update_status(&self, _color: gdk::RGBA) {
        self.icon.set_pixel_size(40);
    }

    pub fn refresh(&self, value: Option<f32>, status_color: gdk::RGBA) {
        self.set_value(value);
        self.update_status(status_color);
    }
}
