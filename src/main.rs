#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use eframe::egui::{self, Color32, RichText};
use std::{
    env,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_tools.rs"));
}

#[derive(Debug)]
enum Event { Title(String), Progress(f32), Log(String), Finished(Result<(), String>) }

struct DownloaderApp {
    url: String,
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
        Self { url: String::new(), title: String::new(), status: "貼上影片網址即可開始下載".into(), progress: 0.0, busy: false, child: None, rx, tx, error: None, logs: Vec::new(), versions_open: false,
            versions: format!("影片下載器 v{}\nyt-dlp：{}\nFFmpeg：{}\nDeno：{}", embedded::APP_VERSION, tool_version("yt-dlp"), tool_version("ffmpeg"), tool_version("deno")) }
    }
}

impl DownloaderApp {
    fn start(&mut self) {
        let url = self.url.trim().to_owned();
        if url.is_empty() { self.error = Some("請輸入影片網址。".into()); return; }
        let Some(ytdlp) = tool_path("yt-dlp") else { self.error = Some("找不到 yt-dlp。請將 yt-dlp.exe 放在程式旁，或重新下載包含工具的 Release。".into()); return; };
        let Some(ffmpeg) = tool_path("ffmpeg") else { self.error = Some("找不到 FFmpeg。請將 ffmpeg.exe 放在程式旁，或重新下載包含工具的 Release。".into()); return; };
        let deno = tool_path("deno");
        if deno.is_none() { self.error = Some("找不到 Deno；YouTube 的可用格式或畫質可能受限。下載仍會繼續。".into()); }

        let output = app_dir();
        let ffmpeg_dir = ffmpeg.parent().unwrap_or(Path::new("."));
        let mut args = vec!["-o".into(), output.join("%(title).80B [%(id)s].%(ext)s").to_string_lossy().into_owned(), "-f".into(), "bv*[ext=mp4]+ba[ext=m4a]/bv*+ba/b".into(), "--windows-filenames".into(), "--no-playlist".into(), "--user-agent".into(), "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36".into(), "--referer".into(), url.clone(), "--ffmpeg-location".into(), ffmpeg_dir.to_string_lossy().into_owned()];
        if let Some(deno) = deno { args.extend(["--js-runtimes".into(), format!("deno:{}", deno.display())]); }
        args.extend(["--newline".into(), "--".into(), url]);
        self.title.clear(); self.progress = 0.0; self.logs.clear(); self.status = "正在解析影片資訊…".into(); self.busy = true;
        let tx = self.tx.clone();
        let mut command = Command::new(ytdlp);
        command.args(args).current_dir(&output).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)] command.creation_flags(0x08000000);
        match command.spawn() {
            Ok(mut child) => {
                if let Some(stdout) = child.stdout.take() { let tx = tx.clone(); thread::spawn(move || for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Some(rest) = line.strip_prefix("[download] Destination: ").or_else(|| line.strip_prefix("Destination: ")) { let title = Path::new(rest.trim()).file_stem().unwrap_or_default().to_string_lossy().into_owned(); let _ = tx.send(Event::Title(title)); }
                    if let Some(pct) = line.find('%').and_then(|i| line[..i].split_whitespace().last()).and_then(|s| s.parse::<f32>().ok()) { let _ = tx.send(Event::Progress(pct.clamp(0.0, 100.0))); }
                }); }
                if let Some(stderr) = child.stderr.take() { let tx = self.tx.clone(); thread::spawn(move || for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if let Some(rest) = line.strip_prefix("[download] Destination: ").or_else(|| line.strip_prefix("Destination: ")) { let title = Path::new(rest.trim()).file_stem().unwrap_or_default().to_string_lossy().into_owned(); let _ = tx.send(Event::Title(title)); }
                    if let Some(pct) = line.find('%').and_then(|i| line[..i].split_whitespace().last()).and_then(|s| s.parse::<f32>().ok()) { let _ = tx.send(Event::Progress(pct.clamp(0.0, 100.0))); }
                    let _ = tx.send(Event::Log(line.chars().take(500).collect()));
                }); }
                self.child = Some(child);
            }
            Err(e) => { self.busy = false; self.status = "無法啟動下載工具".into(); self.error = Some(e.to_string()); }
        }
    }

    fn cancel(&mut self) {
        if let Some(mut child) = self.child.take() { let _ = child.kill(); let _ = child.wait(); }
        self.busy = false; self.progress = 0.0; self.status = "已取消下載".into();
    }

    fn poll(&mut self) {
        while let Ok(event) = self.rx.try_recv() { match event { Event::Title(t) => self.title = t, Event::Progress(p) => { self.progress = p; self.status = "正在下載…".into(); }, Event::Log(line) => { if self.logs.len() == 12 { self.logs.remove(0); } self.logs.push(line); }, Event::Finished(r) => { self.busy = false; self.child = None; match r { Ok(()) => { self.progress = 100.0; self.status = "下載完成".into(); open_directory(&app_dir()); }, Err(e) => { self.status = "下載失敗".into(); let details = self.logs.join("\n"); self.error = Some(if details.is_empty() { e } else { format!("{e}\n\n{details}") }); } } } } }
        if let Some(child) = &mut self.child { if let Ok(Some(status)) = child.try_wait() { let success = status.success(); let _ = self.tx.send(Event::Finished(if success { Ok(()) } else { Err("yt-dlp 執行失敗，請檢查網址或錯誤訊息。".into()) })); } }
    }
}

impl eframe::App for DownloaderApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        ctx.request_repaint_after(Duration::from_millis(120));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::from_rgb(17, 19, 24)).inner_margin(egui::Margin::same(26))).show(ui, |ui| {
            ui.set_max_width(620.0);
            ui.horizontal(|ui| { ui.heading(RichText::new("影片下載器").size(25.0).strong()); if ui.small_button(format!("v{}　版本資訊", embedded::APP_VERSION)).clicked() { self.versions_open = true; } });
            ui.add_space(20.0);
            ui.label(RichText::new("影片網址").color(Color32::from_rgb(174, 184, 201)));
            let entry = ui.add_enabled(!self.busy, egui::TextEdit::singleline(&mut self.url).hint_text("https://…").desired_width(f32::INFINITY));
            if entry.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.busy { self.start(); }
            ui.add_space(12.0);
            ui.horizontal(|ui| { if ui.add_enabled(!self.busy, egui::Button::new("開始下載").min_size(egui::vec2(115.0, 36.0))).clicked() { self.start(); } if ui.add_enabled(self.busy, egui::Button::new("取消").min_size(egui::vec2(85.0, 36.0))).clicked() { self.cancel(); } });
            ui.add_space(22.0);
            ui.label(RichText::new("影片").color(Color32::from_rgb(174, 184, 201)));
            ui.label(if self.title.is_empty() { "等待影片資訊…" } else { &self.title });
            ui.add_space(12.0);
            ui.add(egui::ProgressBar::new(self.progress / 100.0).text(if self.busy { format!("{:.1}%", self.progress) } else { String::new() }));
            ui.add_space(8.0); ui.label(RichText::new(&self.status).color(Color32::from_rgb(91, 166, 255)));
        });
        ctx.set_theme(egui::Theme::Dark);
        if self.versions_open { egui::Window::new("版本資訊").open(&mut self.versions_open).show(&ctx, |ui| { ui.label(&self.versions); }); }
        if let Some(message) = self.error.clone() { let mut open = true; egui::Window::new("提示").collapsible(false).resizable(false).open(&mut open).show(&ctx, |ui| { ui.label(message); if ui.button("確定").clicked() { self.error = None; } }); if !open { self.error = None; } }
    }
}

impl Drop for DownloaderApp {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child { let _ = child.kill(); let _ = child.wait(); }
    }
}

fn app_dir() -> PathBuf { env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).or_else(|| env::current_dir().ok()).unwrap_or_default() }
fn tool_path(name: &str) -> Option<PathBuf> {
    let exe = format!("{name}.exe"); let mut candidates = vec![app_dir().join(&exe)];
    if let Some(one_drive) = env::var_os("ONEDRIVE") { candidates.push(PathBuf::from(one_drive).join("bin").join(&exe)); }
    if let Some(temp) = embedded_tool(name) { candidates.push(temp); }
    for p in candidates { if p.is_file() { return Some(p); } }
    env::var_os("PATH").into_iter().flat_map(|s| env::split_paths(&s).map(|p| p.join(&exe)).collect::<Vec<_>>()).find(|p| p.is_file())
}
fn embedded_tool(name: &str) -> Option<PathBuf> {
    let (data, file) = match name { "yt-dlp" => (embedded::YTDLP?, "yt-dlp.exe"), "ffmpeg" => (embedded::FFMPEG?, "ffmpeg.exe"), "deno" => (embedded::DENO?, "deno.exe"), _ => return None };
    let dir = env::temp_dir().join(format!("video-downloader-{}", embedded::APP_VERSION)); fs::create_dir_all(&dir).ok()?; let path = dir.join(file);
    if !path.is_file() || fs::metadata(&path).ok()?.len() != data.len() as u64 { fs::write(&path, data).ok()?; }
    Some(path)
}
fn tool_version(name: &str) -> String { tool_path(name).and_then(|p| { let mut c = Command::new(p); c.arg("--version"); #[cfg(windows)] c.creation_flags(0x08000000); c.output().ok() }).map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("unknown").to_owned()).unwrap_or_else(|| "missing".into()) }
fn open_directory(path: &Path) { #[cfg(windows)] { let _ = Command::new("explorer").arg(path).spawn(); } #[cfg(target_os="macos")] { let _ = Command::new("open").arg(path).spawn(); } #[cfg(all(unix, not(target_os="macos")))] { let _ = Command::new("xdg-open").arg(path).spawn(); } }

fn main() -> eframe::Result {
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([680.0, 430.0]).with_min_inner_size([480.0, 360.0]).with_title("影片下載器"), system_font_fallback: true, ..Default::default() };
    eframe::run_native("影片下載器", options, Box::new(|cc| { cc.egui_ctx.set_theme(egui::Theme::Dark); Ok(Box::<DownloaderApp>::default()) }))
}
