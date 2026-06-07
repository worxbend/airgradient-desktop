//! Reusable metric card for pollutant sensors.
//!
//! `SensorCard` is used for CO2, TVOC, NOx, and particulate matter. It wraps a
//! small GTK widget tree and exposes methods such as `refresh()` so the
//! dashboard can update values without knowing the card's internal labels.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, DrawingArea, Image, Label, Orientation};

const TREND_CLASSES: [&str; 3] = ["trend-improved", "trend-worse", "trend-neutral"];

#[derive(Clone)]
pub struct SensorCard {
    root: GtkBox,
    icon: Image,
    value_label: Label,
    unit_label: Label,
    trend_label: Label,
    trend_context_label: Label,
    status_dot: DrawingArea,
    status_color: Rc<RefCell<gdk::RGBA>>,
}

impl SensorCard {
    /// Build a card with an icon, title, value, unit, status dot, and trend.
    pub fn new(name: &str, unit: &str, icon_name: &str) -> Self {
        let root = GtkBox::new(Orientation::Horizontal, 16);
        root.set_halign(Align::Fill);
        root.set_valign(Align::Fill);
        root.add_css_class("card");
        root.add_css_class("metric-card");
        root.add_css_class("sensor-card");

        let icon = Image::from_icon_name(icon_name);
        icon.set_pixel_size(48);
        icon.set_halign(Align::Center);
        icon.set_valign(Align::Center);
        let icon_box = GtkBox::new(Orientation::Vertical, 0);
        icon_box.add_css_class("metric-icon");
        icon_box.set_valign(Align::Center);
        icon_box.append(&icon);

        let content = GtkBox::new(Orientation::Vertical, 4);
        content.set_hexpand(true);
        content.set_valign(Align::Center);

        let header = GtkBox::new(Orientation::Horizontal, 6);
        header.set_hexpand(true);

        let title = Label::builder().label(name).halign(Align::Start).build();
        title.add_css_class("metric-title");

        let status_dot = DrawingArea::new();
        status_dot.set_size_request(8, 8);
        status_dot.add_css_class("status-dot");
        let status_color = Rc::new(RefCell::new(gdk::RGBA::new(0.73, 0.73, 0.73, 1.0)));
        let color = status_color.clone();
        // DrawingArea lets the card draw a tiny status dot using the exact
        // current threshold color. The color is stored in `Rc<RefCell<_>>`
        // because GTK calls this draw function later, after `new()` returns.
        status_dot.set_draw_func(move |widget, cr, width, height| {
            let color = color.borrow();
            cr.set_source_rgba(
                f64::from(color.red()),
                f64::from(color.green()),
                f64::from(color.blue()),
                f64::from(color.alpha()),
            );
            let radius = (width.min(height) as f64) * 0.42;
            cr.arc(
                f64::from(width) / 2.0,
                f64::from(height) / 2.0,
                radius,
                0.0,
                std::f64::consts::PI * 2.0,
            );
            let _ = cr.fill();
            widget.set_size_request(8, 8);
        });

        header.append(&title);
        header.append(&status_dot);

        let value_label = Label::builder().label("--").halign(Align::Start).build();
        value_label.add_css_class("metric-value");

        let unit_label = Label::builder().label(unit).halign(Align::Start).build();
        unit_label.add_css_class("metric-unit");

        let trend_label = Label::builder()
            .label("No previous reading")
            .halign(Align::Start)
            .build();
        trend_label.add_css_class("trend-neutral");
        trend_label.add_css_class("trend-value");

        let trend_context_label = Label::builder()
            .label("from last reading")
            .halign(Align::Start)
            .build();
        trend_context_label.add_css_class("metric-unit");
        trend_context_label.add_css_class("trend-context");

        let trend_box = GtkBox::new(Orientation::Horizontal, 8);
        trend_box.set_margin_top(8);
        trend_box.append(&trend_label);
        trend_box.append(&trend_context_label);

        content.append(&header);
        content.append(&value_label);
        content.append(&unit_label);
        content.append(&trend_box);

        root.append(&icon_box);
        root.append(&content);

        Self {
            root,
            icon,
            value_label,
            unit_label,
            trend_label,
            trend_context_label,
            status_dot,
            status_color,
        }
    }

    pub fn widget(&self) -> GtkBox {
        // Cloning a GTK widget is cheap: it clones a reference to the same
        // underlying GObject, not a duplicate UI subtree.
        self.root.clone()
    }

    /// Make the card small enough for the PM row.
    pub fn set_compact(&self) {
        self.root.add_css_class("compact-sensor-card");
        self.root.set_spacing(8);
        self.icon.set_pixel_size(28);
        self.trend_context_label.set_visible(false);
    }

    /// Make the card small enough for the gas row while keeping more visual
    /// weight than the PM cards.
    pub fn set_narrow(&self) {
        self.root.add_css_class("narrow-sensor-card");
        self.root.set_spacing(10);
        self.icon.set_pixel_size(36);
        self.trend_context_label.set_visible(false);
    }

    pub fn set_value(&self, value: Option<f32>, unit: Option<&str>) {
        match value {
            Some(value) => {
                self.value_label.set_text(&format_metric_value(value));
                if let Some(unit) = unit {
                    self.unit_label.set_text(unit);
                }
            }
            None => self.value_label.set_text("--"),
        }
    }

    pub fn set_trend(&self, current: Option<f32>, previous: Option<f32>, unit: &str) {
        update_trend_labels(
            &self.trend_label,
            &self.trend_context_label,
            current,
            previous,
            unit,
            true,
        );
    }

    pub fn update_status(&self, color: gdk::RGBA) {
        let class = status_class(color);
        let classes = [
            "status-green",
            "status-blue",
            "status-yellow",
            "status-orange",
            "status-red",
            "status-unknown",
        ];
        classes
            .iter()
            .for_each(|css_class| self.root.remove_css_class(css_class));
        // The CSS class controls the card accent/gradient. The DrawingArea dot
        // uses the exact same semantic color.
        self.root.add_css_class(class);

        *self.status_color.borrow_mut() = color;
        self.status_dot.queue_draw();
    }

    pub fn refresh(&self, value: Option<f32>, unit: Option<&str>, status_color: gdk::RGBA) {
        self.set_value(value, unit);
        self.update_status(status_color);
    }
}

pub fn update_trend_labels(
    trend_label: &Label,
    trend_context_label: &Label,
    current: Option<f32>,
    previous: Option<f32>,
    unit: &str,
    lower_is_better: bool,
) {
    // Remove old trend classes first. GTK CSS classes are additive, so leaving
    // an old class would make the final color depend on selector order.
    for class in TREND_CLASSES {
        trend_label.remove_css_class(class);
    }

    let Some(current) = current else {
        trend_label.set_text("No reading");
        trend_label.add_css_class("trend-neutral");
        trend_context_label.set_text("from last reading");
        return;
    };
    let Some(previous) = previous else {
        trend_label.set_text("No previous reading");
        trend_label.add_css_class("trend-neutral");
        trend_context_label.set_text("from last reading");
        return;
    };

    let delta = current - previous;
    if delta.abs() < 0.05 {
        trend_label.set_text(&format!("→ 0 {unit}"));
        trend_label.add_css_class("trend-neutral");
    } else {
        let improves = if lower_is_better {
            delta < 0.0
        } else {
            delta > 0.0
        };
        let arrow = if delta > 0.0 { "↑" } else { "↓" };
        let sign = if delta > 0.0 { "+" } else { "" };
        trend_label.set_text(&format!("{arrow} {sign}{} {unit}", format_delta(delta)));
        trend_label.add_css_class(if improves {
            "trend-improved"
        } else {
            "trend-worse"
        });
    }
    trend_context_label.set_text("from last reading");
}

pub fn format_metric_value(value: f32) -> String {
    if value.abs() >= 100.0 || value.fract().abs() < 0.05 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn format_delta(value: f32) -> String {
    if value.abs() >= 10.0 || value.fract().abs() < 0.05 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn status_class(color: gdk::RGBA) -> &'static str {
    const GREEN: (f32, f32, f32) = (51.0 / 255.0, 209.0 / 255.0, 122.0 / 255.0);
    const BLUE: (f32, f32, f32) = (53.0 / 255.0, 132.0 / 255.0, 228.0 / 255.0);
    const YELLOW: (f32, f32, f32) = (245.0 / 255.0, 194.0 / 255.0, 17.0 / 255.0);
    const ORANGE: (f32, f32, f32) = (1.0, 120.0 / 255.0, 0.0);
    const RED: (f32, f32, f32) = (237.0 / 255.0, 51.0 / 255.0, 59.0 / 255.0);

    let candidates = [
        ("status-green", GREEN),
        ("status-blue", BLUE),
        ("status-yellow", YELLOW),
        ("status-orange", ORANGE),
        ("status-red", RED),
        (
            "status-unknown",
            (154.0 / 255.0, 153.0 / 255.0, 150.0 / 255.0),
        ),
    ];

    let (r, g, b) = (color.red(), color.green(), color.blue());
    let mut best = ("status-unknown", f64::MAX);
    // Threshold helpers return concrete RGBA values. CSS needs class names, so
    // choose the closest known palette color.
    for (name, (cr, cg, cb)) in candidates {
        let distance = ((f64::from(r) - f64::from(cr)).powi(2)
            + (f64::from(g) - f64::from(cg)).powi(2)
            + (f64::from(b) - f64::from(cb)).powi(2))
        .sqrt();
        if distance < best.1 {
            best = (name, distance);
        }
    }
    best.0
}
