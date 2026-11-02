#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod download;
mod event;
mod theme;
mod tools;
mod ui;
mod update;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_tools.rs"));
}

use std::sync::Arc;

use eframe::egui;

use crate::app::DownloaderApp;

const APP_TITLE: &str = "影片下載器";

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 500.0])
            .with_min_inner_size([320.0, 160.0])
            .with_title(APP_TITLE)
            .with_app_id("video-downloader")
            .with_icon(app_icon()),
        system_font_fallback: true,
        ..Default::default()
    };

    eframe::run_native(
        APP_TITLE,
        options,
        Box::new(|cc| Ok(Box::new(DownloaderApp::new(cc)))),
    )
}

fn app_icon() -> Arc<egui::IconData> {
    let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!(
        "../assets/app-icon.png"
    )));
    let mut reader = decoder.read_info().expect("read app icon PNG");
    let mut rgba = vec![0; reader.output_buffer_size().expect("icon too large")];
    let info = reader.next_frame(&mut rgba).expect("decode app icon PNG");
    rgba.truncate(info.buffer_size());

    Arc::new(egui::IconData {
        rgba,
        width: info.width,
        height: info.height,
    })
}
