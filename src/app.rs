//! 應用程式狀態與業務邏輯（不含繪圖）。

use std::{
    path::Path,
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

use eframe::egui;

use crate::{
    download::{self, DownloadMode},
    embedded,
    event::{Event, UpdateInfo},
    theme, tools, ui, update,
};

const MAX_LOG_LINES: usize = 12;

/// 版本檢查與更新流程的狀態。
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    CheckFailed(String),
    Available(UpdateInfo),
    Installing(UpdateInfo),
    InstallFailed(UpdateInfo, String),
    Restarting,
}

impl UpdateState {
    pub fn label(&self) -> String {
        match self {
            Self::Idle => "尚未檢查更新".into(),
            Self::Checking => "正在檢查更新…".into(),
            Self::UpToDate => "目前已是最新版本".into(),
            Self::CheckFailed(error) => format!("檢查失敗：{error}"),
            Self::Available(info) => format!("發現新版本 v{}", info.version),
            Self::Installing(_) => "正在下載並安裝更新…".into(),
            Self::InstallFailed(_, error) => format!("更新失敗：{error}"),
            Self::Restarting => "更新已完成，正在重新啟動…".into(),
        }
    }

    /// 目前可以按下「立即更新」的版本。
    pub fn installable(&self) -> Option<&UpdateInfo> {
        match self {
            Self::Available(info) | Self::InstallFailed(info, _) => Some(info),
            _ => None,
        }
    }

    pub fn is_installing(&self) -> bool {
        matches!(self, Self::Installing(_))
    }
}

pub struct DownloaderApp {
    pub url: String,
    pub mode: DownloadMode,
    pub title: String,
    pub status: String,
    pub progress: f32,
    pub busy: bool,
    pub error: Option<String>,
    pub versions_open: bool,
    pub versions: String,
    pub update: UpdateState,

    child: Option<std::process::Child>,
    logs: Vec<String>,
    should_exit: bool,
    rx: Receiver<Event>,
    tx: Sender<Event>,
}

impl DownloaderApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);

        let (tx, rx) = mpsc::channel();

        Self {
            url: String::new(),
            mode: DownloadMode::Video,
            title: String::new(),
            status: String::new(),
            progress: 0.0,
            busy: false,
            error: None,
            versions_open: false,
            versions: format!(
                "應用程式：v{}\nFFmpeg：{}\nyt-dlp：{}\nDeno：{}",
                embedded::APP_VERSION,
                tools::tool_version("ffmpeg"),
                tools::tool_version("yt-dlp"),
                tools::tool_version("deno"),
            ),
            update: UpdateState::Idle,
            child: None,
            logs: Vec::new(),
            should_exit: false,
            rx,
            tx,
        }
    }

    // ---------------------------------------------------------------- 下載

    pub fn start(&mut self) {
        if self.busy {
            return;
        }

        let url = self.url.trim().to_owned();
        if url.is_empty() {
            self.error = Some("請輸入影片網址。".into());
            return;
        }

        let Some(ytdlp) = tools::tool_path("yt-dlp") else {
            self.error = Some(missing_tool_message("yt-dlp", "yt-dlp"));
            return;
        };
        let Some(ffmpeg) = tools::tool_path("ffmpeg") else {
            self.error = Some(missing_tool_message("FFmpeg", "ffmpeg"));
            return;
        };

        let deno = tools::tool_path("deno");
        if deno.is_none() {
            self.error =
                Some("找不到 Deno；YouTube 的可用格式或畫質可能受限。下載仍會繼續。".into());
        }

        let output = tools::download_dir();
        let ffmpeg_dir = ffmpeg.parent().unwrap_or(Path::new("."));
        let args = download::build_args(&url, self.mode, &output, ffmpeg_dir, deno.as_deref());

        self.title.clear();
        self.progress = 0.0;
        self.logs.clear();

        match download::spawn(&ytdlp, &args, &output, &self.tx) {
            Ok(child) => {
                self.child = Some(child);
                self.busy = true;
                self.status = "正在解析媒體資訊…".into();
            }
            Err(error) => {
                self.status = "無法啟動下載工具".into();
                self.error = Some(error.to_string());
            }
        }
    }

    pub fn cancel(&mut self) {
        self.kill_child();
        self.busy = false;
        self.progress = 0.0;
        self.status = "已取消下載".into();
    }

    pub fn toggle_download(&mut self) {
        if self.busy {
            self.cancel();
        } else {
            self.start();
        }
    }

    pub fn paste_url(&mut self) {
        match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
            Ok(text) => self.url = text.trim().to_owned(),
            Err(error) => self.error = Some(format!("無法讀取剪貼簿：{error}")),
        }
    }

    fn kill_child(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    // ---------------------------------------------------------------- 更新

    pub fn open_versions(&mut self) {
        self.versions_open = true;

        if matches!(self.update, UpdateState::Idle) {
            update::check_for_update(self.tx.clone());
            self.update = UpdateState::Checking;
        }
    }

    pub fn install_update(&mut self) {
        if let Some(info) = self.update.installable().cloned() {
            self.update = UpdateState::Installing(info.clone());
            update::download_update(info, self.tx.clone());
        }
    }

    // ---------------------------------------------------------------- 事件

    fn poll(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            self.handle_event(event);
        }
        self.check_child_exit();
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            // 取消或結束後才抵達的殘留事件不應覆蓋狀態。
            Event::Title(title) if self.busy => self.title = title,
            Event::Progress(progress) if self.busy => {
                self.progress = progress;
                self.status = "正在下載…".into();
            }
            Event::Title(_) | Event::Progress(_) => {}
            Event::Log(line) => {
                if self.logs.len() >= MAX_LOG_LINES {
                    self.logs.remove(0);
                }
                self.logs.push(line);
            }
            Event::Finished(result) => self.on_finished(result),
            Event::UpdateChecked(result) => {
                self.update = match result {
                    Ok(Some(info)) => UpdateState::Available(info),
                    Ok(None) => UpdateState::UpToDate,
                    Err(error) => UpdateState::CheckFailed(error),
                };
            }
            Event::UpdateReady(result) => self.on_update_ready(result),
        }
    }

    fn on_finished(&mut self, result: Result<(), String>) {
        self.busy = false;
        self.child = None;

        match result {
            Ok(()) => {
                self.progress = 100.0;
                self.status = "下載完成".into();
                tools::open_directory(&tools::download_dir());
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

    fn on_update_ready(&mut self, result: Result<(), String>) {
        let UpdateState::Installing(info) = &self.update else {
            return;
        };

        self.update = match result {
            Ok(()) => {
                self.should_exit = true;
                UpdateState::Restarting
            }
            Err(error) => UpdateState::InstallFailed(info.clone(), error),
        };
    }

    /// 只在子程序結束時送出一次 Finished 事件。
    fn check_child_exit(&mut self) {
        let Some(status) = self
            .child
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten())
        else {
            return;
        };

        self.child = None;

        let result = if status.success() {
            Ok(())
        } else {
            Err("yt-dlp 執行失敗，請檢查網址或錯誤訊息。".into())
        };

        let _ = self.tx.send(Event::Finished(result));
    }
}

fn missing_tool_message(display: &str, tool: &str) -> String {
    format!(
        "找不到 {display}。請將 {} 放在程式所在目錄，\
         或確認工具已安裝並位於 PATH 中。",
        tools::tool_filename(tool),
    )
}

impl eframe::App for DownloaderApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();

        if self.should_exit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        ctx.request_repaint_after(Duration::from_millis(120));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui::render(self, ui);
    }
}

impl Drop for DownloaderApp {
    fn drop(&mut self) {
        self.kill_child();
    }
}
