#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use eframe::egui::{self, Color32, RichText};
use std::{
    env, fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_tools.rs"));
}

#[derive(Debug)]
enum Event {
    Title(String),
    Progress(f32),
    Log(String),
    Finished(Result<(), String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DownloadMode {
    Video,
    Audio,
}

struct DownloaderApp {
    url: String,
    mode: DownloadMode,
    title: String,
    status: String,
    progress: f32,
    busy: bool,
    child: Option<Child>,
    rx: Receiver<Event>,
    tx: Sender<Event>,
    error: Option<String>,
    logs: Vec<String>,
    versions_open: bool,
    versions: String,
}

impl Default for DownloaderApp {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();

        Self {
            url: String::new(),
            mode: DownloadMode::Video,
            title: String::new(),
            status: "貼上影片網址即可開始下載".into(),
            progress: 0.0,
            busy: false,
            child: None,
            rx,
            tx,
            error: None,
            logs: Vec::new(),
            versions_open: false,
            versions: format!(
                "影片下載器 v{}\nyt-dlp：{}\nFFmpeg：{}\nDeno：{}",
                embedded::APP_VERSION,
                tool_version("yt-dlp"),
                tool_version("ffmpeg"),
                tool_version("deno"),
            ),
        }
    }
}

impl DownloaderApp {
    fn start(&mut self) {
        let url = self.url.trim().to_owned();

        if url.is_empty() {
            self.error = Some("請輸入影片網址。".into());
            return;
        }

        let Some(ytdlp) = tool_path("yt-dlp") else {
            self.error = Some(format!(
                "找不到 yt-dlp。請將 {} 放在程式所在目錄，\
                 或確認工具已安裝並位於 PATH 中。",
                tool_filename("yt-dlp"),
            ));
            return;
        };

        let Some(ffmpeg) = tool_path("ffmpeg") else {
            self.error = Some(format!(
                "找不到 FFmpeg。請將 {} 放在程式所在目錄，\
                 或確認工具已安裝並位於 PATH 中。",
                tool_filename("ffmpeg"),
            ));
            return;
        };

        let deno = tool_path("deno");

        if deno.is_none() {
            self.error =
                Some("找不到 Deno；YouTube 的可用格式或畫質可能受限。下載仍會繼續。".into());
        }

        let output = app_dir();
        let ffmpeg_dir = ffmpeg.parent().unwrap_or(Path::new("."));

        let mut args = vec![
            "-o".into(),
            output
                .join("%(title).80B [%(id)s].%(ext)s")
                .to_string_lossy()
                .into_owned(),
        ];

        match self.mode {
            DownloadMode::Video => {
                args.extend(["-f".into(), "bv*[ext=mp4]+ba[ext=m4a]/bv*+ba/b".into()])
            }
            DownloadMode::Audio => args.extend([
                "-x".into(),
                "--audio-format".into(),
                "mp3".into(),
                "--audio-quality".into(),
                "0".into(),
            ]),
        }

        // Windows 專用檔名處理參數。
        #[cfg(target_os = "windows")]
        args.push("--windows-filenames".into());

        // GUI 程式的 stdout/stderr 會接到管線；指定 UTF-8 可避免
        // Windows 系統地區編碼（例如 cp950）無法輸出日文等字元。
        #[cfg(target_os = "windows")]
        args.extend(["--encoding".into(), "utf-8".into()]);

        args.extend([
            "--no-playlist".into(),
            "--user-agent".into(),
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
             AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/122.0.0.0 Safari/537.36"
                .into(),
            "--referer".into(),
            url.clone(),
            "--ffmpeg-location".into(),
            ffmpeg_dir.to_string_lossy().into_owned(),
        ]);

        if let Some(deno) = deno {
            args.extend(["--js-runtimes".into(), format!("deno:{}", deno.display())]);
        }

        args.extend(["--newline".into(), "--".into(), url]);

        self.title.clear();
        self.progress = 0.0;
        self.logs.clear();
        self.status = "正在解析媒體資訊…".into();
        self.busy = true;

        let tx = self.tx.clone();

        let mut command = Command::new(ytdlp);
        command
            .args(args)
            .current_dir(&output)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        command.creation_flags(0x08000000);

        match command.spawn() {
            Ok(mut child) => {
                if let Some(stdout) = child.stdout.take() {
                    let tx = tx.clone();

                    thread::spawn(move || {
                        read_output_lines(BufReader::new(stdout), &tx, false);
                    });
                }

                if let Some(stderr) = child.stderr.take() {
                    let tx = self.tx.clone();

                    thread::spawn(move || {
                        read_output_lines(BufReader::new(stderr), &tx, true);
                    });
                }

                self.child = Some(child);
            }

            Err(error) => {
                self.busy = false;
                self.status = "無法啟動下載工具".into();
                self.error = Some(error.to_string());
            }
        }
    }

    fn cancel(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }

        self.busy = false;
        self.progress = 0.0;
        self.status = "已取消下載".into();
    }

    fn poll(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Title(title) => {
                    self.title = title;
                }

                Event::Progress(progress) => {
                    self.progress = progress;
                    self.status = "正在下載…".into();
                }

                Event::Log(line) => {
                    if self.logs.len() == 12 {
                        self.logs.remove(0);
                    }

                    self.logs.push(line);
                }

                Event::Finished(result) => {
                    self.busy = false;
                    self.child = None;

                    match result {
                        Ok(()) => {
                            self.progress = 100.0;
                            self.status = "下載完成".into();
                            open_directory(&app_dir());
                        }

                        Err(error) => {
                            self.status = "下載失敗".into();

                            let details = self.logs.join("\n");

                            self.error = Some(if details.is_empty() {
                                error
                            } else {
                                format!("{error}\n\n{details}")
                            });
                        }
                    }
                }
            }
        }

        // 只在子程序結束時送出一次 Finished 事件。
        let finished = self
            .child
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten());

        if let Some(status) = finished {
            self.child = None;

            let result = if status.success() {
                Ok(())
            } else {
                Err("yt-dlp 執行失敗，請檢查網址或錯誤訊息。".into())
            };

            let _ = self.tx.send(Event::Finished(result));
        }
    }
}

fn read_output_lines<R: BufRead>(mut reader: R, tx: &Sender<Event>, log: bool) {
    let mut bytes = Vec::new();

    loop {
        bytes.clear();

        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&bytes);
                process_output_line(
                    &tx,
                    line.trim_end_matches(|character| character == '\r' || character == '\n'),
                    log,
                );
            }
        }
    }
}

fn process_output_line(tx: &Sender<Event>, line: &str, log: bool) {
    if let Some(rest) = line
        .strip_prefix("[download] Destination: ")
        .or_else(|| line.strip_prefix("Destination: "))
    {
        let title = Path::new(rest.trim())
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        let _ = tx.send(Event::Title(title));
    }

    if let Some(progress) = line
        .find('%')
        .and_then(|index| line[..index].split_whitespace().last())
        .and_then(|value| value.parse::<f32>().ok())
    {
        let _ = tx.send(Event::Progress(progress.clamp(0.0, 100.0)));
    }

    if log {
        let _ = tx.send(Event::Log(line.chars().take(500).collect()));
    }
}

fn compact_ui(
    app: &mut DownloaderApp,
    ui: &mut egui::Ui,
    scale: f32,
    accent: Color32,
    surface: Color32,
    card_margin: i8,
) {
    egui::Frame::NONE
        .fill(surface)
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(42, 51, 66)))
        .inner_margin(egui::Margin::same(card_margin))
        .show(ui, |ui| {
            ui.label(RichText::new("網址").size(14.0 * scale).strong());
            ui.add_space(4.0 * scale);

            let entry = ui.add_enabled(
                !app.busy,
                egui::TextEdit::singleline(&mut app.url)
                    .hint_text("https://…")
                    .font(egui::FontId::proportional(12.0 * scale))
                    .desired_width(f32::INFINITY)
                    .margin(egui::vec2(9.0 * scale, 8.0 * scale)),
            );

            entry.context_menu(|ui| {
                if ui.button("貼上網址").clicked() {
                    match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
                        Ok(text) => app.url = text.trim().to_owned(),
                        Err(error) => app.error = Some(format!("無法讀取剪貼簿：{error}")),
                    }

                    ui.close();
                }
            });

            if entry.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter))
                && !app.busy
            {
                app.start();
            }

            ui.add_space(10.0 * scale);
            ui.add(
                egui::ProgressBar::new(app.progress / 100.0)
                    .fill(accent)
                    .desired_height(12.0 * scale)
                    .text(if app.busy {
                        RichText::new(format!("{:.1}%", app.progress)).size(11.0 * scale)
                    } else {
                        RichText::new("").size(11.0 * scale)
                    }),
            );

            ui.add_space(10.0 * scale);
            let button_label = if app.busy {
                "取消下載"
            } else {
                "開始下載"
            };
            let button_color = if app.busy {
                Color32::from_rgb(159, 69, 76)
            } else {
                accent
            };

            if ui
                .add(
                    egui::Button::new(RichText::new(button_label).size(14.0 * scale).strong())
                        .fill(button_color)
                        .min_size(egui::vec2(ui.available_width(), 40.0 * scale)),
                )
                .clicked()
            {
                if app.busy {
                    app.cancel();
                } else {
                    app.start();
                }
            }
        });
}

impl eframe::App for DownloaderApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        ctx.request_repaint_after(Duration::from_millis(120));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let viewport_size = ui.available_size();
        let compact = viewport_size.x < 600.0 || viewport_size.y < 450.0;
        let scale = (viewport_size.x / 720.0)
            .min(viewport_size.y / 540.0)
            .clamp(0.62, 1.0);
        let outer_margin = ((if compact { 16.0 } else { 30.0 }) * scale).round() as i8;
        let card_margin = ((if compact { 14.0 } else { 20.0 }) * scale).round() as i8;

        let accent = Color32::from_rgb(45, 183, 211);
        let muted = Color32::from_rgb(151, 164, 183);
        let surface = Color32::from_rgb(24, 30, 41);
        let background = Color32::from_rgb(14, 18, 26);

        let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
        style.visuals.dark_mode = true;
        style.visuals.panel_fill = background;
        style.visuals.window_fill = surface;
        style.visuals.override_text_color = Some(Color32::from_rgb(232, 238, 246));
        style.visuals.selection.bg_fill = accent;
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(39, 47, 61);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(53, 65, 83);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(37, 151, 178);
        style.spacing.item_spacing = egui::vec2(12.0, 12.0);
        ctx.set_style_of(egui::Theme::Dark, style);
        ctx.set_theme(egui::Theme::Dark);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(background)
                    .inner_margin(egui::Margin::same(outer_margin)),
            )
            .show(ui, |ui| {
                let content_width = ui.available_width().min(900.0);
                let content_height = ui.available_height();
                ui.allocate_ui_with_layout(
                    egui::vec2(content_width, content_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        if compact {
                            compact_ui(self, ui, scale, accent, surface, card_margin);
                        } else {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.heading(
                                        RichText::new("影片下載器").size(24.0 * scale).strong(),
                                    );
                                    ui.label(
                                        RichText::new("貼上連結，快速下載影片或音訊")
                                            .size(12.0 * scale)
                                            .color(muted),
                                    );
                                });

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button(format!(
                                                "v{}　版本資訊",
                                                embedded::APP_VERSION
                                            ))
                                            .clicked()
                                        {
                                            self.versions_open = true;
                                        }
                                    },
                                );
                            });

                            ui.add_space(24.0 * scale);

                            egui::Frame::NONE
                                .fill(surface)
                                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(42, 51, 66)))
                                .inner_margin(egui::Margin::same(card_margin))
                                .show(ui, |ui| {
                                    ui.label(RichText::new("影片網址").size(14.0 * scale).strong());
                                    ui.add_space(6.0 * scale);

                                    let entry = ui.add_enabled(
                                        !self.busy,
                                        egui::TextEdit::singleline(&mut self.url)
                                            .hint_text("https://www.youtube.com/watch?v=…")
                                            .font(egui::FontId::proportional(13.0 * scale))
                                            .desired_width(f32::INFINITY)
                                            .margin(egui::vec2(12.0 * scale, 12.0 * scale)),
                                    );

                                    entry.context_menu(|ui| {
                                        if ui.button("貼上網址").clicked() {
                                            match arboard::Clipboard::new()
                                                .and_then(|mut clipboard| clipboard.get_text())
                                            {
                                                Ok(text) => self.url = text.trim().to_owned(),
                                                Err(error) => {
                                                    self.error =
                                                        Some(format!("無法讀取剪貼簿：{error}"));
                                                }
                                            }

                                            ui.close();
                                        }
                                    });

                                    ui.label(
                                        RichText::new("提示：在網址欄按右鍵可貼上剪貼簿內容")
                                            .small()
                                            .size(11.0 * scale)
                                            .color(muted),
                                    );

                                    ui.add_space(18.0 * scale);
                                    ui.label(RichText::new("下載類型").size(14.0 * scale).strong());
                                    ui.add_space(4.0 * scale);

                                    ui.horizontal(|ui| {
                                        ui.add_enabled_ui(!self.busy, |ui| {
                                            let video_selected = self.mode == DownloadMode::Video;
                                            let audio_selected = self.mode == DownloadMode::Audio;

                                            if ui
                                                .add(
                                                    egui::Button::new("影片　MP4")
                                                        .min_size(egui::vec2(
                                                            132.0 * scale,
                                                            38.0 * scale,
                                                        ))
                                                        .fill(if video_selected {
                                                            Color32::from_rgb(31, 112, 137)
                                                        } else {
                                                            Color32::from_rgb(34, 41, 54)
                                                        })
                                                        .stroke(egui::Stroke::new(
                                                            1.0,
                                                            if video_selected {
                                                                accent
                                                            } else {
                                                                Color32::from_rgb(54, 64, 80)
                                                            },
                                                        )),
                                                )
                                                .clicked()
                                            {
                                                self.mode = DownloadMode::Video;
                                            }

                                            if ui
                                                .add(
                                                    egui::Button::new("音樂　MP3")
                                                        .min_size(egui::vec2(
                                                            132.0 * scale,
                                                            38.0 * scale,
                                                        ))
                                                        .fill(if audio_selected {
                                                            Color32::from_rgb(31, 112, 137)
                                                        } else {
                                                            Color32::from_rgb(34, 41, 54)
                                                        })
                                                        .stroke(egui::Stroke::new(
                                                            1.0,
                                                            if audio_selected {
                                                                accent
                                                            } else {
                                                                Color32::from_rgb(54, 64, 80)
                                                            },
                                                        )),
                                                )
                                                .clicked()
                                            {
                                                self.mode = DownloadMode::Audio;
                                            }
                                        });
                                    });

                                    if entry.lost_focus()
                                        && ui.input(|input| input.key_pressed(egui::Key::Enter))
                                        && !self.busy
                                    {
                                        self.start();
                                    }

                                    ui.add_space(20.0 * scale);

                                    ui.horizontal(|ui| {
                                        if ui
                                            .add_enabled(
                                                !self.busy,
                                                egui::Button::new(
                                                    RichText::new("開始下載")
                                                        .size(14.0 * scale)
                                                        .strong(),
                                                )
                                                .fill(accent)
                                                .min_size(egui::vec2(170.0 * scale, 46.0 * scale)),
                                            )
                                            .clicked()
                                        {
                                            self.start();
                                        }

                                        if ui
                                            .add_enabled(
                                                self.busy,
                                                egui::Button::new(
                                                    RichText::new("取消下載").size(13.0 * scale),
                                                )
                                                .min_size(egui::vec2(110.0 * scale, 46.0 * scale)),
                                            )
                                            .clicked()
                                        {
                                            self.cancel();
                                        }
                                    });
                                });

                            ui.add_space(16.0 * scale);

                            egui::Frame::NONE
                                .fill(Color32::from_rgb(19, 25, 35))
                                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(36, 45, 59)))
                                .inner_margin(egui::Margin::same(card_margin))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(match self.mode {
                                                DownloadMode::Video => "下載影片",
                                                DownloadMode::Audio => "下載音樂",
                                            })
                                            .size(14.0 * scale)
                                            .strong(),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(&self.status)
                                                        .size(12.0 * scale)
                                                        .color(accent),
                                                );
                                            },
                                        );
                                    });

                                    ui.add_space(8.0 * scale);
                                    ui.label(
                                        RichText::new(if self.title.is_empty() {
                                            "等待影片資訊…"
                                        } else {
                                            &self.title
                                        })
                                        .size(13.0 * scale)
                                        .color(
                                            if self.title.is_empty() {
                                                muted
                                            } else {
                                                Color32::from_rgb(218, 226, 238)
                                            },
                                        ),
                                    );
                                    ui.add_space(12.0 * scale);
                                    ui.add(
                                        egui::ProgressBar::new(self.progress / 100.0)
                                            .fill(accent)
                                            .desired_height(14.0 * scale)
                                            .text(if self.busy {
                                                RichText::new(format!("{:.1}%", self.progress))
                                                    .size(11.0 * scale)
                                            } else {
                                                RichText::new("").size(11.0 * scale)
                                            }),
                                    );
                                });
                        }
                    },
                );
            });

        if self.versions_open && !compact {
            egui::Window::new("版本資訊")
                .open(&mut self.versions_open)
                .show(&ctx, |ui| {
                    ui.label(&self.versions);
                });
        }

        if let Some(message) = self.error.clone() {
            let mut open = true;

            egui::Window::new("提示")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(&ctx, |ui| {
                    ui.label(message);

                    if ui.button("確定").clicked() {
                        self.error = None;
                    }
                });

            if !open {
                self.error = None;
            }
        }
    }
}

impl Drop for DownloaderApp {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// -----------------------------------------------------------------------------
// 跨平台工具檔名與路徑
// -----------------------------------------------------------------------------

fn tool_filename(name: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        format!("{name}.exe")
    }

    #[cfg(not(target_os = "windows"))]
    {
        name.to_owned()
    }
}

fn app_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .or_else(|| env::current_dir().ok())
        .unwrap_or_default()
}

fn tool_path(name: &str) -> Option<PathBuf> {
    let filename = tool_filename(name);
    let mut candidates = vec![app_dir().join(&filename)];

    if let Some(one_drive) = env::var_os("ONEDRIVE") {
        candidates.push(PathBuf::from(one_drive).join("bin").join(&filename));
    }

    if let Some(path) = embedded_tool(name) {
        candidates.push(path);
    }

    for path in candidates {
        if path.is_file() {
            return Some(path);
        }
    }

    env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(&filename))
        .find(|path| path.is_file())
}

fn embedded_tool(name: &str) -> Option<PathBuf> {
    let data: &[u8] = match name {
        "yt-dlp" => embedded::YTDLP?,
        "ffmpeg" => embedded::FFMPEG?,
        "deno" => embedded::DENO?,
        _ => return None,
    };

    let directory = env::temp_dir().join(format!("video-downloader-{}", embedded::APP_VERSION));

    fs::create_dir_all(&directory).ok()?;

    let path = directory.join(tool_filename(name));

    // 比對內容而非僅比對檔案大小，避免舊版本工具被誤用。
    let needs_write = fs::read(&path)
        .map(|existing| existing != data)
        .unwrap_or(true);

    if needs_write {
        fs::write(&path, data).ok()?;
    }

    // macOS/Linux 執行外部工具需要執行權限。
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&path).ok()?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).ok()?;
    }

    Some(path)
}

fn tool_version(name: &str) -> String {
    tool_path(name)
        .and_then(|path| {
            let mut command = Command::new(path);
            command.arg("--version");

            #[cfg(windows)]
            command.creation_flags(0x08000000);

            command.output().ok()
        })
        .and_then(|output| {
            // FFmpeg 主要將版本資訊寫入 stderr；
            // yt-dlp 與 Deno 通常則可從 stdout 取得。
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            stdout
                .lines()
                .chain(stderr.lines())
                .find(|line| {
                    let line = line.trim();
                    !line.is_empty()
                })
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "missing".into())
}

fn open_directory(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("explorer").arg(path).spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(path).spawn();
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 580.0])
            .with_min_inner_size([480.0, 360.0])
            .with_title("影片下載器"),
        system_font_fallback: true,
        ..Default::default()
    };

    eframe::run_native(
        "影片下載器",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            Ok(Box::<DownloaderApp>::default())
        }),
    )
}
