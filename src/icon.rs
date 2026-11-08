//! 視窗圖示：解碼內嵌的 PNG。

use std::{io::Cursor, sync::Arc};

use eframe::egui;

const ICON_PNG: &[u8] = include_bytes!("../assets/app-icon.png");

/// 解碼 `assets/app-icon.png`（需為 8-bit RGBA）；任何一步失敗都回傳 None。
pub fn load() -> Option<Arc<egui::IconData>> {
    let mut reader = png::Decoder::new(Cursor::new(ICON_PNG)).read_info().ok()?;
    let mut rgba = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut rgba).ok()?;

    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    rgba.truncate(info.buffer_size());

    Some(Arc::new(egui::IconData {
        rgba,
        width: info.width,
        height: info.height,
    }))
}
