# Air Monitor

GTK dashboard for an AirGradient local-server device (`/measures/current`) built with `gtk4-rs` and `libadwaita`.

## References

- [gtk-rs documentation](https://gtk-rs.org/)
- [gtk4-rs book (GUI with Rust and GTK 4)](https://gtk-rs.org/gtk4-rs/git/book/)
- [gtk-rs-core repository](https://github.com/gtk-rs/gtk-rs-core/)
- [gtk4-rs repository](https://github.com/gtk-rs/gtk4-rs/)

## Requirements

- Rust toolchain (stable)
- GTK 4 development files
- Libadwaita 1 development files
- `pkg-config`

Debian/Ubuntu example:

```bash
sudo apt install pkg-config libgtk-4-dev libadwaita-1-dev build-essential
```

## Project Setup

Install dependencies and run the app:

```bash
cargo build
cargo run
```

Run a release build locally:

```bash
cargo build --release
```

### Development layout

- `src/main.rs` wires modules.
- `src/app.rs` owns app bootstrap and activation.
- `src/ui/` contains page modules (`app`, `dashboard`, and reusable widgets).
- `src/sensors/` parses and normalizes `/measures/current` payloads.
- `src/state.rs` stores mutable window/page state (`current_page`, action counter).
- `assets/` contains `.desktop`, icon, and CSS used by the dashboard widgets.

### Dependency compatibility note

`libadwaita` and `gtk4` must come from a compatible pair to avoid a native `links = "gtk-4"` conflict:

- `adw (libadwaita) = "0.7"` pairs with `gtk4 = "0.9"` in this setup.
- If you bump either crate, bump both together from the same gtk-rs release train.

## Project files

- `Cargo.toml` — crate metadata and dependencies for `adw` (libadwaita) and `gtk4`
- `src/main.rs` — minimal app startup with `adw::Application` and a window

## Notes

If you get native link errors, verify:

- `pkg-config` can find GTK 4 and libadwaita
- Your system provides matching major versions for both libraries

If you later add `.ui` files or translations, we can extend this base setup with:

- GTK resource bundling (`glib::MainContext` + build helper)
- app metadata (`.desktop`, icon assets, AppStream files)
- structured window/panel modules and signal wiring

### Release packaging

The repository includes a GitHub Actions release pipeline at `.github/workflows/release.yml` that:

- installs GTK/libadwaita system dependencies,
- builds `airgradient-desktop` in `--release`,
- packages the binary and packaging assets,
- uploads a release archive and attaches it to version tags (`v*`).

### Theme support

- App startup uses `ColorScheme::Default`, so GTK/adwaita follows the desktop theme initially.
- Settings page theme control toggles `System → Light → Dark → System`.
- `System` uses GTK/adwaita native theme when not overriding.

## Install Notes

After building (or from a release archive), install locally:

```bash
sudo install -Dm755 target/release/airgradient-desktop /usr/local/bin/airgradient-desktop
sudo install -Dm644 assets/airgradient-desktop.svg /usr/share/icons/hicolor/256x256/apps/airgradient-desktop.svg
sudo install -Dm644 assets/com.airgradient.desktop /usr/share/applications/com.airgradient.desktop
sudo update-desktop-database /usr/share/applications
```

You can also install a release package from the workflow artifact by extracting the tarball and copying the same paths under `/usr`.

## Dashboard behavior

- Welcome route is shown until a server URL is configured.
- Settings validates URLs in forms like:
  - `http://192.168.1.201/`
  - `http://192.168.1.201`
  - `192.168.1.201`
  - `http://192.168.1.201:80`
- Auto-refresh interval is configurable in Settings and also manual refresh is available from header icons.
