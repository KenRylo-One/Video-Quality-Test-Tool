//! The tool. One window, two columns.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod files;
mod fonts;
mod frame_viewer;
mod metrics;
mod notes;
mod plot;
mod right;
mod run;
mod settings_panel;
mod theme;
mod widgets;

use app::{APP_NAME, VqttApp};

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1040.0, 660.0])
            .with_title(APP_NAME),
        ..Default::default()
    };

    eframe::run_native(
        "vqtt",
        options,
        Box::new(|context| Ok(Box::new(VqttApp::new(&context.egui_ctx)))),
    )
}
