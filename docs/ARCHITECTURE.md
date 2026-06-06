# Architecture

This document explains how Air Monitor is structured and how data moves through the app. It is written for contributors who may be new to Rust, GTK, or libadwaita.

## Runtime Overview

```text
main.rs
  -> app::run()
    -> creates adw::Application
      -> ui::build_main_window()
        -> creates pages, header bar, settings, dashboard
        -> starts timers
        -> fetches measurements when a server URL exists
```

The application runs on GTK's main thread. UI updates must happen on that main thread. Network work is blocking, so it is executed with `gio::spawn_blocking` and then the result is applied back to GTK widgets.

## Main Modules

### `src/app.rs`

Owns application startup. It creates `adw::Application`, reads the config file, creates `AppState`, and asks the UI module to build the main window.

### `src/config.rs`

Reads and writes `config.json` under the XDG config directory. It stores only user preferences that should survive app restarts, such as the server URL and refresh interval.

### `src/state.rs`

Stores in-memory UI state:

- current page
- theme mode
- configured server URL
- refresh interval
- an action counter used for simple user actions

This state is wrapped in `Rc<RefCell<AppState>>` when shared with GTK callbacks.

`Rc` means "reference counted". Multiple callbacks can own a pointer to the same state.

`RefCell` means "checked at runtime". Rust normally enforces borrowing at compile time. GTK signal callbacks are dynamic, so `RefCell` lets callbacks temporarily borrow or mutate shared state while still checking that borrows do not overlap incorrectly.

### `src/sensors/`

Normalizes JSON from AirGradient into `AirMeasureSnapshot`.

The app uses `Option<f32>` for sensor values because local-server payloads can vary by model and firmware. A missing value is represented as `None`, not as `0`.

### `src/ui/`

Builds GTK widgets and connects signals.

- `ui/app.rs` owns the window, navigation, settings page, timers, and fetching.
- `ui/dashboard.rs` builds the dashboard layout and applies parsed measurements.
- widget files such as `sensor_card.rs` and `aqi_widget.rs` wrap repeated GTK controls behind small Rust structs.

## Data Flow

```text
Settings URL
  -> config::write_config()
  -> AppState::set_server_url()
  -> trigger_fetch_current_measures()
  -> fetch_current_measurements()
  -> HTTP GET /measures/current
  -> parse_air_measurements()
  -> DashboardPageWidgets::apply_measurements()
  -> individual GTK labels/cards are updated
```

## Fetching

`trigger_fetch_current_measures()` starts the fetch and updates status labels.

`fetch_current_measurements()` performs the HTTP request and JSON parsing. It is intentionally not async by itself; it uses `reqwest::blocking` and is called inside `gio::spawn_blocking`.

This avoids freezing the GTK interface while the device responds.

## Dashboard History

The dashboard keeps the last five `AirMeasureSnapshot` values in memory. The current implementation uses only the most recent previous value to display trends such as:

```text
↓ -3 AQI
↑ +42 ppm
```

The history is not written to disk. Restarting the app clears it.

## Theme Handling

The Settings page changes libadwaita's `ColorScheme`.

The app also listens to `StyleManager::is_dark()` and toggles a CSS class on the root widget. That class gives the dark theme a slightly darker shell background while preserving libadwaita's native theme behavior.

## GTK Resource Handling

The symbolic SVG icons live in `resources/icons/`. `build.rs` compiles them into a `.gresource` file at build time. At runtime, `ui::register_resources()` registers the compiled resource and adds it to GTK's icon theme path.

That is why widgets can use icon names such as:

```rust
Image::from_icon_name("airgradient-co2-symbolic")
```

instead of loading SVG files manually.
