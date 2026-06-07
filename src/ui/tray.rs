//! StatusNotifier tray integration.
//!
//! This module owns the Linux tray menu and command bridge. The main window
//! module only installs it and handles the resulting high-level commands.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;

use adw::prelude::*;
use gtk4::Stack;

use crate::app_info::APP_NAME;
use crate::state::{AppState, Page};

#[derive(Debug, Clone, Copy)]
enum TrayCommand {
    ShowDashboard,
    HideWindow,
    Quit,
}

struct AirMonitorTray {
    sender: mpsc::Sender<TrayCommand>,
}

impl ksni::Tray for AirMonitorTray {
    fn id(&self) -> String {
        "airgradient-desktop".into()
    }

    fn title(&self) -> String {
        APP_NAME.into()
    }

    fn icon_name(&self) -> String {
        "airgradient-desktop".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        static TRAY_ICON: OnceLock<ksni::Icon> = OnceLock::new();

        let icon = TRAY_ICON
            .get_or_init(|| {
                let pixbuf = gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(
                    include_bytes!("../../assets/airgradient-tray.png").as_slice(),
                ))
                .expect("embedded tray icon should be a valid PNG");
                let width = pixbuf.width();
                let height = pixbuf.height();
                let channels = pixbuf.n_channels() as usize;
                let rowstride = pixbuf.rowstride() as usize;
                let pixels = pixbuf.read_pixel_bytes();
                let pixels = pixels.as_ref();
                let mut data = Vec::with_capacity(width as usize * height as usize * 4);

                for y in 0..height as usize {
                    let row_start = y * rowstride;
                    let row_end = row_start + width as usize * channels;
                    for pixel in pixels[row_start..row_end].chunks_exact(channels) {
                        let red = pixel[0];
                        let green = pixel[1];
                        let blue = pixel[2];
                        let alpha = if channels >= 4 { pixel[3] } else { 255 };
                        data.extend_from_slice(&[alpha, red, green, blue]);
                    }
                }

                ksni::Icon {
                    width,
                    height,
                    data,
                }
            })
            .clone();

        vec![icon]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.sender.send(TrayCommand::ShowDashboard);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{MenuItem, StandardItem};

        vec![
            StandardItem {
                label: "Show Dashboard".into(),
                icon_name: "go-home-symbolic".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayCommand::ShowDashboard);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Hide Window".into(),
                icon_name: "window-close-symbolic".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayCommand::HideWindow);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayCommand::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub(super) struct SystemTrayRuntime {
    handle: ksni::blocking::Handle<AirMonitorTray>,
    command_source: Option<glib::SourceId>,
}

impl Drop for SystemTrayRuntime {
    fn drop(&mut self) {
        if let Some(source) = self.command_source.take() {
            source.remove();
        }
        self.handle.shutdown().wait();
    }
}

pub(super) fn install_system_tray(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    stack: &Stack,
    state: Rc<RefCell<AppState>>,
) -> Option<SystemTrayRuntime> {
    let (sender, receiver) = mpsc::channel();
    let tray = AirMonitorTray { sender };
    let handle = match ksni::blocking::TrayMethods::assume_sni_available(tray, true).spawn() {
        Ok(handle) => handle,
        Err(err) => {
            eprintln!("System tray unavailable: {err}");
            return None;
        }
    };

    let app = app.clone();
    let window = window.clone();
    let stack = stack.clone();
    let command_source = glib::timeout_add_local(Duration::from_millis(250), move || {
        while let Ok(command) = receiver.try_recv() {
            match command {
                TrayCommand::ShowDashboard => {
                    let target = if state.borrow().has_server_url() {
                        Page::Dashboard
                    } else {
                        Page::Settings
                    };
                    state.borrow_mut().set_page(target);
                    stack.set_visible_child_name(target.id());
                    window.present();
                }
                TrayCommand::HideWindow => window.hide(),
                TrayCommand::Quit => app.quit(),
            }
        }
        glib::ControlFlow::Continue
    });

    Some(SystemTrayRuntime {
        handle,
        command_source: Some(command_source),
    })
}
