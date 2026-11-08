// 只在 release 隱藏主控台；debug 保留，方便看 println!/panic 訊息。
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod app;
mod config;
mod download;
mod event;
mod icon;
mod theme;
mod tools;
mod ui;
mod update;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_tools.rs"));
}

use eframe::egui;

use crate::app::DownloaderApp;

const APP_TITLE: &str = "影片下載器";
const APP_ID: &str = "video-downloader";

fn main() -> eframe::Result {
    eframe::run_native(
        APP_TITLE,
        native_options(),
        Box::new(|cc| Ok(Box::new(DownloaderApp::new(cc)))),
    )
}

fn native_options() -> eframe::NativeOptions {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([760.0, 540.0])
        .with_min_inner_size([320.0, 160.0])
        .with_title(APP_TITLE)
        .with_app_id(APP_ID);

    // 圖示解碼失敗時不要讓整個程式 panic，沒有圖示照樣能用。
    if let Some(icon) = icon::load() {
        viewport = viewport.with_icon(icon);
    }

    eframe::NativeOptions {
        viewport,
        system_font_fallback: true,
        ..Default::default()
    }
}
