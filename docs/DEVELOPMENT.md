# Development Guide

This guide explains common tasks and the Rust/GTK patterns used in this project.

## Common Commands

```bash
cargo fmt
cargo test
cargo build
cargo build --release
cargo run
```

Run these before committing behavior changes:

```bash
cargo fmt
cargo test
cargo build --release
```

## Adding A New Sensor Value

1. Add a field to `AirMeasureSnapshot` in `src/sensors/air_quality.rs`.
2. Extend `parse_air_measurements()` with possible JSON key names.
3. Add or reuse a threshold function in `src/sensors/thresholds.rs`.
4. Add a card or widget in `src/ui/dashboard.rs`.
5. Update the parser test with a sample payload.

Use `Option<f32>` unless the value is guaranteed to exist in every supported device payload.

## Adding A New Dashboard Card

Most pollutant metrics should use `SensorCard`:

```rust
let card = SensorCard::new("CO₂", "ppm", "airgradient-co2-symbolic");
card.refresh(snapshot.co2, Some("ppm"), co2_status_color(value));
```

Special layout cards, such as AQI, temperature, and humidity, have their own widget structs.

## GTK Widget Ownership

GTK widgets are GObjects under the hood. In gtk-rs, cloning a widget usually clones a reference to the same widget. It does not duplicate the visual tree.

That is why code can do:

```rust
let label = label.clone();
button.connect_clicked(move |_| {
    label.set_text("Clicked");
});
```

The callback owns a reference to the same label, so it can update it later.

## Shared State In Callbacks

GTK callbacks can outlive the function that created them. To share state with those callbacks, the app uses:

```rust
Rc<RefCell<AppState>>
```

- `Rc` allows multiple callbacks to hold the same state.
- `RefCell` allows mutation at runtime with borrow checks.

This is appropriate here because GTK callbacks run on the main thread. Do not use this pattern for multi-threaded data sharing; use `Arc<Mutex<T>>` or another thread-safe type in that case.

## Blocking Work

GTK should not be blocked by network requests. The app uses:

```rust
gio::spawn_blocking(move || fetch_current_measurements(&base_url))
```

The closure runs away from the GTK main loop. When it finishes, the async continuation updates the UI on the main thread.

## CSS And Styling

Dashboard CSS lives in `assets/dashboard.css`.

The CSS is loaded with `include_str!`, which embeds it into the Rust binary at compile time. After editing CSS, rebuild or rerun the app.

The UI intentionally uses libadwaita native widgets where possible. Keep Settings pages in `PreferencesPage`, `PreferencesGroup`, and preference row widgets.

## Icons

Symbolic icons live in:

```text
resources/icons/scalable/status/
```

They should be monochrome SVGs and use `currentColor` when possible. GTK recolors symbolic icons for light, dark, and high contrast themes.

After adding an icon:

1. Add the SVG file.
2. Add it to `resources/airgradient.gresource.xml`.
3. Add it to the `rerun-if-changed` list in `build.rs`.
4. Use it by icon name without the `.svg` suffix.

## Configuration

Config is JSON and lives at:

```text
$XDG_CONFIG_HOME/airgradient-desktop/config.json
```

or:

```text
$HOME/.config/airgradient-desktop/config.json
```

Avoid storing runtime-only values there. Values such as last measurement history should stay in memory unless there is a clear product reason to persist them.
