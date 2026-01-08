#![cfg_attr(
    all(target_os = "windows", not(debug_assertions), feature = "gui"),
    windows_subsystem = "windows"
)]

mod api;
mod app_core;
mod config;
#[cfg(feature = "gui")]
mod gui;
#[cfg(not(feature = "gui"))]
mod tui;
mod types;
#[cfg(feature = "gui")]
use eframe::egui;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "gui")]
    {
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([500.0, 500.0]),
            ..Default::default()
        };
        return eframe::run_native(
            "bili-live",
            options,
            Box::new(|cc| Ok(Box::new(gui::BiliLiveApp::new(cc)))),
        )
        .map_err(|e| e.into());
    }

    #[cfg(not(feature = "gui"))]
    {
        return tui::run_tui();
    }
}
