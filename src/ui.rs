//! 所有 egui 繪圖程式碼。狀態與邏輯在 `app.rs`。

use eframe::egui::{self, Button, Frame, Margin, RichText, Stroke, Vec2};

use crate::{
    app::DownloaderApp,
    download::DownloadMode,
    embedded,
    theme::PALETTE,
};

/// 依視窗大小計算的版面參數。
struct Metrics {
    compact: bool,
    scale: f32,
    outer_margin: i8,
    card_margin: i8,
}

impl Metrics {
    fn new(size: Vec2) -> Self {
        let compact = size.x < 600.0 || size.y < 450.0;

        let scale = if compact {
            (size.x / 580.0).clamp(0.72, 1.0)
        } else {
            (size.x / 720.0).min(size.y / 540.0).clamp(0.62, 1.35)
        };

        let (outer, card) = if compact { (16.0, 14.0) } else { (30.0, 20.0) };

        Self {
            compact,
            scale,
            outer_margin: (outer * scale).round() as i8,
            card_margin: (card * scale).round() as i8,
        }
    }

    /// 將基準尺寸依比例縮放。
    fn s(&self, value: f32) -> f32 {
        value * self.scale
    }
}

pub fn render(app: &mut DownloaderApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let m = Metrics::new(ui.available_size());

    egui::CentralPanel::default()
        .frame(
            Frame::NONE
                .fill(PALETTE.background)
                .inner_margin(Margin::same(m.outer_margin)),
        )
        .show(ui, |ui| {
            let size = egui::vec2(ui.available_width().min(900.0), ui.available_height());

            ui.allocate_ui_with_layout(
                size,
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    if m.compact {
                        compact_view(app, ui, &m);
                    } else {
                        full_view(app, ui, &m);
                    }
                },
            );
        });

    if app.versions_open && !m.compact {
        versions_window(app, &ctx);
    }
    uninstall_window(app, &ctx);
    error_window(app, &ctx);
}

// ------------------------------------------------------------------ 共用元件

fn card(m: &Metrics) -> Frame {
    Frame::NONE
        .fill(PALETTE.surface)
        .stroke(Stroke::new(1.0, PALETTE.border))
        .inner_margin(Margin::same(m.card_margin))
}

fn progress_bar(progress: f32, height: f32) -> egui::ProgressBar {
    egui::ProgressBar::new((progress / 100.0).clamp(0.0, 1.0))
        .fill(PALETTE.accent)
        .desired_height(height)
        .text(format!("{progress:.1}%"))
}

fn url_input(
    app: &mut DownloaderApp,
    ui: &mut egui::Ui,
    hint: &str,
    font_size: f32,
    margin: Vec2,
) -> egui::Response {
    let entry = ui.add(
        egui::TextEdit::singleline(&mut app.url)
            .hint_text(hint)
            .font(egui::FontId::proportional(font_size))
            .desired_width(f32::INFINITY)
            .margin(margin),
    );

    entry.context_menu(|ui| {
        if ui.button("貼上網址").clicked() {
            app.paste_url();
            ui.close();
        }
    });

    entry
}

fn enter_submitted(entry: &egui::Response, ui: &egui::Ui) -> bool {
    entry.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
}

// ------------------------------------------------------------------ 精簡版面

fn compact_view(app: &mut DownloaderApp, ui: &mut egui::Ui, m: &Metrics) {
    card(m).show(ui, |ui| {
        if app.busy {
            ui.add(progress_bar(app.progress, m.s(18.0)));
        } else {
            ui.label(RichText::new("網址").size(m.s(14.0)).strong());
            ui.add_space(m.s(4.0));

            let entry = url_input(app, ui, "https://…", m.s(12.0), egui::vec2(m.s(9.0), m.s(8.0)));
            if enter_submitted(&entry, ui) {
                app.start();
            }
        }

        ui.add_space(m.s(10.0));

        let (label, color) = if app.busy {
            ("取消下載", PALETTE.danger)
        } else {
            ("開始下載", PALETTE.accent)
        };

        let button = Button::new(RichText::new(label).size(m.s(14.0)).strong())
            .fill(color)
            .min_size(egui::vec2(ui.available_width(), m.s(40.0)));

        if ui.add(button).clicked() {
            app.toggle_download();
        }
    });
}

// ------------------------------------------------------------------ 完整版面

fn full_view(app: &mut DownloaderApp, ui: &mut egui::Ui, m: &Metrics) {
    header(app, ui, m);
    ui.add_space(m.s(24.0));

    card(m).show(ui, |ui| {
        let heading = if app.busy { "下載進度" } else { "影片網址" };
        ui.label(RichText::new(heading).size(m.s(14.0)).strong());
        ui.add_space(m.s(6.0));

        let entry = if app.busy {
            ui.add(progress_bar(app.progress, m.s(40.0)))
        } else {
            let entry = url_input(
                app,
                ui,
                "https://www.youtube.com/watch?v=…",
                m.s(13.0),
                egui::vec2(m.s(12.0), m.s(12.0)),
            );
            ui.label(
                RichText::new("提示：在網址欄按右鍵可貼上剪貼簿內容")
                    .size(m.s(11.0))
                    .color(PALETTE.muted),
            );
            entry
        };

        ui.add_space(m.s(12.0));
        save_path_row(app, ui, m);

        ui.add_space(m.s(18.0));

        ui.horizontal(|ui| {
            ui.vertical(|ui| controls(app, ui, m));

            let info_width = ui.available_width();
            if info_width > m.s(100.0) {
                ui.vertical(|ui| status_panel(app, ui, m, info_width));
            }
        });

        if enter_submitted(&entry, ui) && !app.busy {
            app.start();
        }
    });
}

fn header(app: &mut DownloaderApp, ui: &mut egui::Ui, m: &Metrics) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.heading(RichText::new("影片下載器").size(m.s(24.0)).strong());
            ui.label(
                RichText::new("貼上連結，快速下載影片或音訊")
                    .size(m.s(12.0))
                    .color(PALETTE.muted),
            );
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = format!("版本 v{}", embedded::APP_VERSION);
            if ui.small_button(label).clicked() {
                app.open_versions();
            }
        });
    });
}

/// 保存路徑：顯示目前實際使用的資料夾，可選擇或還原預設。
fn save_path_row(app: &mut DownloaderApp, ui: &mut egui::Ui, m: &Metrics) {
    ui.label(RichText::new("保存路徑").size(m.s(14.0)).strong());
    ui.add_space(m.s(4.0));

    let is_custom = app.settings.download_dir.is_some();
    let path = app.download_dir();
    let enabled = !app.busy;

    ui.horizontal(|ui| {
        ui.add_enabled_ui(enabled, |ui| {
            if ui.button("選擇資料夾…").clicked() {
                app.choose_download_dir();
            }
            if is_custom && ui.button("還原預設").clicked() {
                app.reset_download_dir();
            }
        });

        let suffix = if is_custom { "" } else { "（預設）" };
        ui.add(
            egui::Label::new(
                RichText::new(format!("{}{suffix}", path.display()))
                    .size(m.s(12.0))
                    .color(PALETTE.muted),
            )
            .truncate(),
        );
    });
}

fn controls(app: &mut DownloaderApp, ui: &mut egui::Ui, m: &Metrics) {
    ui.label(RichText::new("下載類型").size(m.s(14.0)).strong());
    ui.add_space(m.s(4.0));

    ui.horizontal(|ui| {
        ui.add_enabled_ui(!app.busy, |ui| {
            mode_button(ui, m, &mut app.mode, DownloadMode::Video, "影片　MP4");
            mode_button(ui, m, &mut app.mode, DownloadMode::Audio, "音樂　MP3");
        });
    });

    ui.add_space(m.s(20.0));

    ui.horizontal(|ui| {
        let start = Button::new(RichText::new("開始下載").size(m.s(14.0)).strong())
            .fill(PALETTE.accent)
            .min_size(egui::vec2(m.s(170.0), m.s(46.0)));
        if ui.add_enabled(!app.busy, start).clicked() {
            app.start();
        }

        let cancel = Button::new(RichText::new("取消下載").size(m.s(13.0)))
            .min_size(egui::vec2(m.s(110.0), m.s(46.0)));
        if ui.add_enabled(app.busy, cancel).clicked() {
            app.cancel();
        }
    });
}

fn mode_button(
    ui: &mut egui::Ui,
    m: &Metrics,
    current: &mut DownloadMode,
    mode: DownloadMode,
    label: &str,
) {
    let selected = *current == mode;

    let button = Button::new(label)
        .min_size(egui::vec2(m.s(132.0), m.s(38.0)))
        .fill(if selected { PALETTE.selected } else { PALETTE.idle })
        .stroke(Stroke::new(
            1.0,
            if selected { PALETTE.accent } else { PALETTE.idle_border },
        ));

    if ui.add(button).clicked() {
        *current = mode;
    }
}

fn status_panel(app: &DownloaderApp, ui: &mut egui::Ui, m: &Metrics, width: f32) {
    // 面板頂端對齊格式按鈕，底端延伸到下載按鈕下緣。
    ui.add_space(m.s(30.0));

    Frame::NONE
        .fill(PALETTE.panel)
        .stroke(Stroke::new(1.0, PALETTE.panel_border))
        .inner_margin(Margin::symmetric(
            m.s(10.0).round() as i8,
            m.s(8.0).round() as i8,
        ))
        .show(ui, |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            ui.set_min_height(m.s(100.0));

            ui.vertical(|ui| {
                let status = RichText::new(&app.status)
                    .size(m.s(13.0))
                    .color(PALETTE.accent);

                if app.busy {
                    ui.label(status);
                    if !app.title.is_empty() {
                        ui.label(
                            RichText::new(&app.title)
                                .size(m.s(13.0))
                                .color(PALETTE.title),
                        );
                    }
                } else {
                    ui.add_space((ui.available_height() / 4.0).max(0.0));
                    ui.label(status);
                    ui.add_space((ui.available_height() / 3.0).max(0.0));

                    let (text, color) = if app.title.is_empty() {
                        ("等待影片資訊…", PALETTE.muted)
                    } else {
                        (app.title.as_str(), PALETTE.title)
                    };
                    ui.label(RichText::new(text).size(m.s(13.0)).color(color));
                }
            });
        });
}

// ------------------------------------------------------------------ 視窗

fn versions_window(app: &mut DownloaderApp, ctx: &egui::Context) {
    let mut open = true;

    egui::Window::new("版本資訊")
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(&app.versions);
            ui.separator();
            ui.label(app.update.label());

            if app.update.installable().is_some() && ui.button("立即更新").clicked() {
                app.install_update();
            }
            if let Some(progress) = app.update.install_progress() {
                ui.add(progress_bar(progress, 16.0).desired_width(240.0));
            }

            ui.separator();
            if ui
                .add_enabled(!app.busy, Button::new("解除安裝…"))
                .clicked()
            {
                app.request_uninstall();
            }
        });

    app.versions_open = open;
}

fn uninstall_window(app: &mut DownloaderApp, ctx: &egui::Context) {
    let Some(targets) = app.uninstall_targets.clone() else {
        return;
    };

    let mut open = true;
    let mut decision: Option<bool> = None;

    egui::Window::new("確認解除安裝")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_max_width(440.0);

            ui.label("即將永久刪除以下資料夾及其內容：");
            for path in &targets {
                ui.label(RichText::new(path.display().to_string()).monospace());
            }
            ui.label(
                RichText::new("此動作無法復原。")
                    .color(PALETTE.danger),
            );

            ui.horizontal(|ui| {
                let yes = Button::new("是，移除").fill(PALETTE.danger);
                if ui.add(yes).clicked() {
                    decision = Some(true);
                }
                if ui.button("否").clicked() {
                    decision = Some(false);
                }
            });
        });

    match decision {
        Some(true) => app.confirm_uninstall(),
        Some(false) => app.cancel_uninstall(),
        None if !open => app.cancel_uninstall(),
        None => {}
    }
}

fn error_window(app: &mut DownloaderApp, ctx: &egui::Context) {
    let Some(message) = app.error.clone() else {
        return;
    };

    let mut open = true;

    egui::Window::new("提示")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(message);

            if ui.button("確定").clicked() {
                app.error = None;
            }
        });

    if !open {
        app.error = None;
    }
}
