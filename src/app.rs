//! 應用程式狀態與業務邏輯（不含繪圖）。

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

use eframe::egui;

use crate::{
    config::{self, Settings},
    download::{self, DownloadMode},
    embedded,
    event::Event,
    theme, tools, ui,
    update::{self, Updater},
};

const MAX_LOG_LINES: usize = 12;

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
    pub update: Updater,
    pub settings: Settings,
    /// `Some` 代表解除安裝確認視窗開啟中，內容為即將刪除的資料夾。
    pub uninstall_targets: Option<Vec<PathBuf>>,
    /// 剛下載完成時的保存路徑；有值時介面會切換為「下載完成」版面。
    pub completed_dir: Option<PathBuf>,

    child: Option<std::process::Child>,
    logs: Vec<String>,
    should_exit: bool,
    rx: Receiver<Event>,
    tx: Sender<Event>,
}

impl DownloaderApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);

        // 背景清理更新殘留檔與舊版本的內嵌工具，不阻塞啟動。
        std::thread::spawn(|| {
            update::cleanup_stale_files();
            tools::cleanup_old_embedded_dirs();
        });

        // 啟動時靜默檢查更新：失敗或已是最新都不打擾使用者。
        let mut updater = Updater::new(&cc.egui_ctx);
        updater.check(true);

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
            versions: version_summary(),
            update: updater,
            settings: Settings::load(),
            uninstall_targets: None,
            completed_dir: None,
            child: None,
            logs: Vec::new(),
            should_exit: false,
            rx,
            tx,
        }
    }

    // ---------------------------------------------------------------- 設定

    /// 實際使用的下載資料夾：自訂路徑存在就用，否則退回預設位置。
    pub fn download_dir(&self) -> PathBuf {
        self.settings
            .download_dir
            .clone()
            .filter(|path| path.is_dir())
            .unwrap_or_else(tools::default_download_dir)
    }

    pub fn choose_download_dir(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("選擇下載資料夾")
            .set_directory(self.download_dir())
            .pick_folder();

        if let Some(path) = picked {
            self.settings.download_dir = Some(path);
            self.save_settings();
        }
    }

    pub fn reset_download_dir(&mut self) {
        self.settings.download_dir = None;
        self.save_settings();
    }

    fn save_settings(&mut self) {
        if let Err(error) = self.settings.save() {
            self.error = Some(format!("無法儲存設定：{error}"));
        }
    }

    // ---------------------------------------------------------------- 解除安裝

    /// 收集將被刪除的資料夾並開啟確認視窗。
    pub fn request_uninstall(&mut self) {
        if self.busy {
            self.error = Some("下載進行中，請先取消下載再解除安裝。".into());
            return;
        }

        if self.update_in_progress() {
            self.error = Some("正在更新中，請等更新結束後再解除安裝。".into());
            return;
        }

        let mut targets: Vec<PathBuf> = config::config_dir()
            .into_iter()
            .filter(|path| path.is_dir())
            .collect();
        targets.extend(tools::embedded_dirs());

        if targets.is_empty() {
            self.error = Some("沒有找到需要移除的資料。".into());
        } else {
            self.uninstall_targets = Some(targets);
        }
    }

    pub fn cancel_uninstall(&mut self) {
        self.uninstall_targets = None;
    }

    pub fn confirm_uninstall(&mut self) {
        let Some(targets) = self.uninstall_targets.take() else {
            return;
        };

        let failures: Vec<String> = targets
            .iter()
            .filter_map(|path| {
                std::fs::remove_dir_all(path)
                    .err()
                    .map(|error| format!("{}：{error}", path.display()))
            })
            .collect();

        // 記憶體中的設定一併重置，避免之後又被寫回磁碟。
        self.settings = Settings::default();

        self.error = Some(if failures.is_empty() {
            "已移除所有應用程式資料。程式本身（exe）不會被刪除，可手動刪除。".into()
        } else {
            format!("部分資料夾無法移除：\n{}", failures.join("\n"))
        });
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

        let output = self.download_dir();
        let ffmpeg_dir = ffmpeg.parent().unwrap_or(Path::new("."));
        let args = download::build_args(&url, self.mode, &output, ffmpeg_dir, deno.as_deref());

        self.completed_dir = None;
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
        self.completed_dir = None;
        self.progress = 0.0;
        self.status = "已取消下載".into();
    }

    /// 精簡版面的主按鈕：下載中取消、完成後開啟資料夾、其餘開始下載。
    pub fn toggle_download(&mut self) {
        if self.busy {
            self.cancel();
        } else if self.completed_dir.is_some() {
            self.open_download_dir();
        } else {
            self.start();
        }
    }

    pub fn open_download_dir(&self) {
        if let Some(dir) = &self.completed_dir {
            tools::open_directory(dir);
        }
    }

    /// 離開「下載完成」版面，回到可輸入新網址的狀態。
    pub fn new_download(&mut self) {
        self.completed_dir = None;
        self.url.clear();
        self.title.clear();
        self.status.clear();
        self.progress = 0.0;
    }

    pub fn paste_url(&mut self) {
        match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
            Ok(text) => {
                self.url = text.trim().to_owned();
                self.completed_dir = None;
            }
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

        // 啟動時的靜默檢查若沒有新版會回到 Idle，此時開啟視窗就重新檢查並顯示結果。
        if matches!(self.update.state, update::State::Idle) {
            self.update.check(false);
        }
    }

    pub fn install_update(&mut self) {
        self.update.install();
    }

    /// 更新正在下載、安裝，或已安裝完成準備重新啟動。
    fn update_in_progress(&self) -> bool {
        self.update.busy() || self.update.installed() || self.should_exit
    }

    fn poll_update(&mut self) {
        self.update.poll();

        // 安裝完成後自動以新版重新啟動，再關閉目前視窗。
        // 重新啟動失敗時狀態會變成 Failed，不會重複嘗試。
        if self.update.installed() && !self.should_exit && self.update.restart() {
            self.should_exit = true;
        }
    }

    // ---------------------------------------------------------------- 事件

    fn poll(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            self.handle_event(event);
        }
        self.check_child_exit();
        self.poll_update();
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
        }
    }

    fn on_finished(&mut self, result: Result<(), String>) {
        self.busy = false;
        self.child = None;

        match result {
            Ok(()) => {
                self.progress = 100.0;
                self.status = "下載完成".into();
                self.completed_dir = Some(self.download_dir());
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

/// 版本視窗顯示的應用程式與各工具版本。
fn version_summary() -> String {
    format!(
        "應用程式：v{}\nFFmpeg：{}\nyt-dlp：{}\nDeno：{}",
        embedded::APP_VERSION,
        tools::tool_version("ffmpeg"),
        tools::tool_version("yt-dlp"),
        tools::tool_version("deno"),
    )
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
