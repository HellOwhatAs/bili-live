#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app_core;
mod config;
mod gui;
mod types;

use eframe::egui;

fn main() -> eframe::Result<()> {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([500.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "bili-live",
        options,
        Box::new(|cc| Ok(Box::new(gui::BiliLiveApp::new(cc)))),
    )
}

