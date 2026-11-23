//! 從 GitHub Releases 檢查新版本，並下載、驗證、替換目前執行檔。
//!
//! 全程只使用 github.com 的一般網頁網址，**不呼叫 api.github.com**，
//! 因此不受 API 每小時 60 次（未驗證）的速率限制：
//!
//! 1. `GET /releases/latest` 會轉址到 `/releases/tag/<tag>`，從最終網址取得最新版本號。
//! 2. `GET /releases/expanded_assets/<tag>` 回傳該版本的資產清單（HTML 片段），解析出檔名。
//! 3. 直接從 `/releases/download/<tag>/<檔名>` 下載。
//!
//! 發佈資產命名（規則見 [`platform::matches`]）：
//!   Windows x86_64         : `video_downloader.exe`
//!   Linux x86_64 / aarch64 : 例如 `video_downloader_omarchy_wsl_x86_64.tar.gz`
//!                            （名稱含 linux/omarchy/wsl 與架構；也接受 .tgz、amd64/x64/arm64）
//!                            內含名為 `video-downloader` 的執行檔，或資產本身就是執行檔
//! 其他平台（macOS 等）沒有對應檔案，只提示新版並引導到下載頁。

use eframe::egui;
use std::{
    path::PathBuf,
    process::Command,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use crate::embedded;

// ───────────────────────── 常數 ─────────────────────────

/// 目前版本：與 UI 標題列顯示的版本同一個來源。
const CURRENT_VERSION: &str = embedded::APP_VERSION;
const REPO: &str = "cloverdefa/video-downloader";
/// 發佈資產與壓縮檔內執行檔的名稱。
const BIN_NAME: &str = "video-downloader";
/// 更新用暫存資料夾的名稱前綴（後面接 process id）。
const TEMP_PREFIX: &str = "video-downloader-update-";

type Res<T> = Result<T, String>;

// ───────────────────────── 公開型別 ─────────────────────────

#[derive(Clone, Debug)]
pub struct AssetInfo {
    pub name: String,
    pub url: String,
    /// 網頁清單不提供大小，為 0；下載時以 Content-Length 為準。
    pub size: u64,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ReleaseInfo {
    pub version: String,
    /// 更新內容。不走 API 取不到 release body，目前為空字串。
    pub notes: String,
    pub page: String,
    /// 目前平台可自動安裝的檔案；None 代表只能手動下載。
    pub asset: Option<AssetInfo>,
    /// 這個 release 上傳的所有檔案名稱，找不到安裝檔時用來協助排查。
    pub asset_names: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available(ReleaseInfo),
    Installing { done: u64, total: u64 },
    /// 已安裝完成，內含新版本號；重新啟動後生效。
    Installed(String),
    Failed(String),
}

// ───────────────────────── Updater ─────────────────────────

enum Msg {
    Checked(Res<Option<ReleaseInfo>>),
    Progress { done: u64, total: u64 },
    Installed(Res<()>),
}

/// 背景執行緒回報結果用：送出訊息後順便要求重繪。
struct Notifier {
    tx: Sender<Msg>,
    ctx: egui::Context,
}

impl Notifier {
    fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }
}

pub struct Updater {
    pub state: State,
    ctx: egui::Context,
    /// 啟動時記下執行檔路徑，替換後仍可用來重新啟動。
    exe: PathBuf,
    rx: Option<Receiver<Msg>>,
    /// 靜默檢查（啟動時）：失敗或已是最新都回到 Idle，不顯示任何訊息。
    silent: bool,
    pending_version: Option<String>,
    /// 最近一次嘗試安裝的版本；安裝失敗時可直接重試，不必重新檢查。
    retry: Option<ReleaseInfo>,
}

impl Updater {
    pub fn new(ctx: &egui::Context) -> Self {
        Self {
            state: State::Idle,
            ctx: ctx.clone(),
            exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from(BIN_NAME)),
            rx: None,
            silent: false,
            pending_version: None,
            retry: None,
        }
    }

    // ───────────── 給 UI 用的查詢介面 ─────────────

    pub fn busy(&self) -> bool {
        matches!(self.state, State::Checking | State::Installing { .. })
    }

    /// 目前狀態的顯示文字。
    pub fn label(&self) -> String {
        match &self.state {
            State::Idle => "尚未檢查更新".into(),
            State::Checking => "正在檢查更新…".into(),
            State::UpToDate => "已是最新版本".into(),
            State::Available(info) if info.asset.is_some() => {
                format!("發現新版本 v{}", info.version)
            }
            State::Available(info) => {
                format!("發現新版本 v{}（找不到此平台的安裝檔，請手動下載）", info.version)
            }
            State::Installing { .. } => "正在下載並安裝更新…".into(),
            State::Installed(v) if v.is_empty() => "更新完成，正在重新啟動…".into(),
            State::Installed(v) => format!("已更新至 v{v}，正在重新啟動…"),
            State::Failed(e) => format!("更新失敗：{e}"),
        }
    }

    /// 可以按下「立即更新」時，回傳對應資產（有新版，或上次安裝失敗可重試）。
    pub fn installable(&self) -> Option<&AssetInfo> {
        match &self.state {
            State::Available(info) => info.asset.as_ref(),
            State::Failed(_) => self.retry.as_ref().and_then(|info| info.asset.as_ref()),
            _ => None,
        }
    }

    /// 安裝中的進度百分比（0–100）。
    pub fn install_progress(&self) -> Option<f32> {
        match self.state {
            State::Installing { done, total } if total > 0 => {
                Some((done as f64 / total as f64 * 100.0) as f32)
            }
            State::Installing { .. } => Some(0.0),
            _ => None,
        }
    }

    /// 是否有新版可更新（含只能手動下載、或上次安裝失敗可重試），供標題列顯示按鈕。
    pub fn has_update(&self) -> bool {
        match self.state {
            State::Available(_) => true,
            State::Failed(_) => self.retry.is_some(),
            _ => false,
        }
    }

    /// 新版資訊（版本、更新內容、下載頁）。
    pub fn release(&self) -> Option<&ReleaseInfo> {
        match &self.state {
            State::Available(info) => Some(info),
            _ => None,
        }
    }

    /// 已安裝完成、等待重新啟動。
    pub fn installed(&self) -> bool {
        matches!(self.state, State::Installed(_))
    }

    // ───────────── 動作 ─────────────

    /// `silent = true`：啟動時自動檢查，只有「有新版」才會改變狀態。
    pub fn check(&mut self, silent: bool) {
        if self.busy() {
            return;
        }
        self.silent = silent;
        self.retry = None;
        self.state = State::Checking;
        self.spawn(|n| n.send(Msg::Checked(github::fetch_latest())));
    }

    /// 下載並安裝目前 `Available` 的版本（或重試上次失敗的安裝）。
    pub fn install(&mut self) {
        let info = match &self.state {
            State::Available(info) => info.clone(),
            State::Failed(_) => match &self.retry {
                Some(info) => info.clone(),
                None => return,
            },
            _ => return,
        };
        let Some(asset) = info.asset.clone() else {
            return;
        };
        self.pending_version = Some(info.version.clone());
        self.retry = Some(info);
        self.state = State::Installing {
            done: 0,
            total: asset.size,
        };
        self.spawn(move |n| {
            let result = installer::install(&asset, |done, total| {
                n.send(Msg::Progress { done, total });
            });
            n.send(Msg::Installed(result));
        });
    }

    /// 以更新後的執行檔啟動新程序；成功回傳 true，呼叫端再關閉目前視窗。
    pub fn restart(&mut self) -> bool {
        let mut cmd = Command::new(&self.exe);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        match cmd.spawn() {
            Ok(_) => true,
            Err(e) => {
                self.state = State::Failed(format!("無法重新啟動：{e}"));
                false
            }
        }
    }

    /// 每個 frame 呼叫一次，收取背景執行緒的結果。
    pub fn poll(&mut self) {
        let Some(rx) = &self.rx else {
            return;
        };
        let msgs: Vec<Msg> = rx.try_iter().collect();
        for msg in msgs {
            self.apply(msg);
        }
        if !self.busy() {
            self.rx = None;
        }
    }

    // ───────────── 內部 ─────────────

    /// 開一條背景執行緒，並接上新的訊息通道。
    fn spawn(&mut self, work: impl FnOnce(Notifier) + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let notifier = Notifier {
            tx,
            ctx: self.ctx.clone(),
        };
        thread::spawn(move || work(notifier));
    }

    fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Checked(Ok(Some(info))) => self.state = State::Available(info),
            Msg::Checked(Ok(None)) => {
                self.state = if self.silent { State::Idle } else { State::UpToDate };
            }
            Msg::Checked(Err(e)) => {
                self.state = if self.silent { State::Idle } else { State::Failed(e) };
            }
            Msg::Progress { done, total } => {
                if matches!(self.state, State::Installing { .. }) {
                    self.state = State::Installing { done, total };
                }
            }
            Msg::Installed(Ok(())) => {
                self.retry = None;
                self.state = State::Installed(self.pending_version.take().unwrap_or_default());
            }
            Msg::Installed(Err(e)) => self.state = State::Failed(e),
        }
    }
}

// ───────────────────────── 網址 ─────────────────────────

fn repo_url() -> String {
    format!("https://github.com/{REPO}")
}

fn latest_url() -> String {
    format!("{}/releases/latest", repo_url())
}

fn assets_url(tag: &str) -> String {
    format!("{}/releases/expanded_assets/{tag}", repo_url())
}

fn tag_page_url(tag: &str) -> String {
    format!("{}/releases/tag/{tag}", repo_url())
}

fn download_url(tag: &str, name: &str) -> String {
    format!("{}{tag}/{name}", download_prefix())
}

/// 只信任本 repo 的 release 下載網址，避免被導向其他來源。
fn download_prefix() -> String {
    format!("{}/releases/download/", repo_url())
}

/// 網址中的 tag／檔名片段只允許這些字元（含 %20 之類的百分比編碼）。
fn is_url_safe(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+' | '%' | '~')
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .user_agent(concat!("video-downloader/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .build()
}

fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(403 | 429, _) => "GitHub 請求過於頻繁，請稍後再試".into(),
        ureq::Error::Status(404, _) => "找不到發佈版本".into(),
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(t) => t.to_string(),
    }
}

// ───────────────────────── 版本比較 ─────────────────────────

mod version {
    pub fn parse(s: &str) -> Option<(u64, u64, u64)> {
        let core = s.trim().trim_start_matches(['v', 'V']);
        let core = core.split(['-', '+']).next()?;
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().unwrap_or("0").parse().ok()?;
        let patch = parts.next().unwrap_or("0").parse().ok()?;
        Some((major, minor, patch))
    }

    pub fn is_newer(remote: &str, local: &str) -> bool {
        match (parse(remote), parse(local)) {
            (Some(r), Some(l)) => r > l,
            _ => false,
        }
    }
}

// ───────────────────────── 平台判斷 ─────────────────────────

mod platform {
    use super::BIN_NAME;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Os {
        Windows,
        Linux,
        Other,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Arch {
        X86_64,
        Aarch64,
        Other,
    }

    pub const HOST_OS: Os = if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "linux") {
        Os::Linux
    } else {
        Os::Other
    };

    pub const HOST_ARCH: Arch = if cfg!(target_arch = "x86_64") {
        Arch::X86_64
    } else if cfg!(target_arch = "aarch64") {
        Arch::Aarch64
    } else {
        Arch::Other
    };

    /// 資產名稱中代表 Linux 版的字樣（名稱已轉小寫）。
    const LINUX_TAGS: [&str; 3] = ["linux", "omarchy", "wsl"];
    /// 不可能是執行檔的附屬檔案（檢查碼、簽章等），挑選資產時排除。
    const IGNORED_SUFFIXES: [&str; 8] = [
        ".sha256", ".sha256sum", ".sig", ".asc", ".txt", ".zip", ".deb", ".rpm",
    ];

    pub fn is_tar_gz(name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        name.ends_with(".tar.gz") || name.ends_with(".tgz")
    }

    fn arch_tags(arch: Arch) -> &'static [&'static str] {
        match arch {
            Arch::X86_64 => &["x86-64", "amd64", "x64"],
            Arch::Aarch64 => &["aarch64", "arm64"],
            Arch::Other => &[],
        }
    }

    /// 這個資產是否是「目前平台」可自動安裝的執行檔。
    pub fn matches(name: &str) -> bool {
        matches_on(name, HOST_OS, HOST_ARCH)
    }

    /// 與 [`matches`] 相同，但平台可指定，方便在任何機器上測試。
    pub fn matches_on(name: &str, os: Os, arch: Arch) -> bool {
        // 底線與空白視為連字號；GitHub 上傳時也可能把空白換成句點或 %20。
        let name = name
            .to_ascii_lowercase()
            .replace("%20", "-")
            .replace(['_', ' '], "-");
        if !name.starts_with(BIN_NAME) {
            return false;
        }
        match os {
            Os::Windows => arch == Arch::X86_64 && name.ends_with(".exe"),
            Os::Linux => {
                // 發佈檔可能以 linux，或以發行版／環境名稱（omarchy、wsl）標示 Linux 版。
                LINUX_TAGS.iter().any(|t| name.contains(t))
                    && arch_tags(arch).iter().any(|a| name.contains(a))
                    && (is_tar_gz(&name) || !IGNORED_SUFFIXES.iter().any(|s| name.ends_with(s)))
            }
            Os::Other => false,
        }
    }

    /// 從資產中挑出目前平台的檔案；同時有壓縮檔與裸執行檔時優先用壓縮檔。
    pub fn pick<'a, T>(assets: &'a [T], name_of: impl Fn(&T) -> &str) -> Option<&'a T> {
        assets
            .iter()
            .filter(|a| matches(name_of(a)))
            .min_by_key(|a| u8::from(!is_tar_gz(name_of(a))))
    }
}

// ───────────────────────── 查詢最新版 ─────────────────────────

mod github {
    use super::*;

    /// 資產清單頁解析出的一筆資產。
    pub struct RemoteAsset {
        pub name: String,
        /// 頁面上若有顯示 "sha256:<hex>" 就取用；沒有就略過驗證。
        pub sha256: Option<String>,
    }

    pub fn fetch_latest() -> Res<Option<ReleaseInfo>> {
        let tag = fetch_latest_tag()?;
        if !version::is_newer(&tag, CURRENT_VERSION) {
            return Ok(None);
        }
        let assets = fetch_assets(&tag)?;
        let asset_names = assets.iter().map(|a| a.name.clone()).collect();
        let asset = platform::pick(&assets, |a| &a.name).map(|a| AssetInfo {
            name: a.name.clone(),
            url: download_url(&tag, &a.name),
            size: 0,
            sha256: a.sha256.clone(),
        });
        Ok(Some(ReleaseInfo {
            version: tag.trim_start_matches(['v', 'V']).to_owned(),
            notes: String::new(),
            page: tag_page_url(&tag),
            asset,
            asset_names,
        }))
    }

    /// `/releases/latest` 會轉址到 `/releases/tag/<tag>`，從最終網址取出 tag。
    fn fetch_latest_tag() -> Res<String> {
        let resp = agent().get(&latest_url()).call().map_err(describe)?;
        tag_from_url(resp.get_url())
            .map(str::to_owned)
            .ok_or_else(|| "找不到發佈版本".to_owned())
    }

    pub fn tag_from_url(url: &str) -> Option<&str> {
        let (_, rest) = url.rsplit_once("/releases/tag/")?;
        let tag = rest.split(['?', '#', '/']).next()?;
        (!tag.is_empty() && tag.chars().all(is_url_safe)).then_some(tag)
    }

    fn fetch_assets(tag: &str) -> Res<Vec<RemoteAsset>> {
        let html = agent()
            .get(&assets_url(tag))
            .call()
            .map_err(describe)?
            .into_string()
            .map_err(|e| format!("回應格式錯誤：{e}"))?;
        Ok(parse_assets(&html, tag))
    }

    /// 解析 expanded_assets 的 HTML，取出每個資產的檔名與（若有）sha256。
    pub fn parse_assets(html: &str, tag: &str) -> Vec<RemoteAsset> {
        let marker = format!("/{REPO}/releases/download/{tag}/");
        let mut out: Vec<RemoteAsset> = Vec::new();
        let mut rest = html;
        while let Some(pos) = rest.find(&marker) {
            let after = &rest[pos + marker.len()..];
            let name_end = after
                .find(|c: char| !is_url_safe(c))
                .unwrap_or(after.len());
            let name = &after[..name_end];

            // 這筆資產的區段：到 </li> 或下一個資產連結為止，從中找 sha256。
            let tail = &after[name_end..];
            let seg_end = [tail.find("</li>"), tail.find(&marker)]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(tail.len());

            if !name.is_empty() && !out.iter().any(|a| a.name == name) {
                out.push(RemoteAsset {
                    name: name.to_owned(),
                    sha256: find_sha256(&tail[..seg_end]),
                });
            }
            rest = tail;
        }
        out
    }

    fn find_sha256(segment: &str) -> Option<String> {
        let start = segment.find("sha256:")? + "sha256:".len();
        let hex = segment.get(start..start + 64)?;
        hex.chars()
            .all(|c| c.is_ascii_hexdigit())
            .then(|| hex.to_ascii_lowercase())
    }
}

// ───────────────────────── 下載與安裝 ─────────────────────────

mod installer {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        fs::File,
        io::{Read, Write},
        path::Path,
    };

    /// 離開作用域時自動刪除的暫存資料夾。
    struct TempDir(PathBuf);

    impl TempDir {
        fn create() -> Res<Self> {
            let dir = std::env::temp_dir().join(format!("{TEMP_PREFIX}{}", std::process::id()));
            std::fs::create_dir_all(&dir).map_err(|e| format!("無法建立暫存資料夾：{e}"))?;
            Ok(Self(dir))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub fn install(asset: &AssetInfo, mut progress: impl FnMut(u64, u64)) -> Res<()> {
        if !asset.url.starts_with(&download_prefix()) {
            return Err("下載網址不在信任範圍內".into());
        }
        let dir = TempDir::create()?;
        // 本機檔名固定，不採用網頁上的檔名，避免路徑穿越。
        let local = dir.0.join(if platform::is_tar_gz(&asset.name) {
            "asset.tar.gz"
        } else {
            "asset.bin"
        });

        let digest = download(asset, &local, &mut progress)?;
        if let Some(expected) = &asset.sha256 {
            if !digest.eq_ignore_ascii_case(expected) {
                return Err("檢查碼不符，已中止更新".into());
            }
        }

        let new_exe = prepare_binary(&local, &dir.0)?;
        check_magic(&new_exe)?;
        self_replace::self_replace(&new_exe).map_err(|e| format!("無法替換執行檔：{e}"))
    }

    /// 下載到 `dest`，確認完整後回傳 sha256（小寫十六進位）。
    fn download(
        asset: &AssetInfo,
        dest: &Path,
        progress: &mut impl FnMut(u64, u64),
    ) -> Res<String> {
        let response = agent().get(&asset.url).call().map_err(describe)?;
        let total = response
            .header("Content-Length")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(asset.size);
        let mut reader = response.into_reader();
        let mut file = File::create(dest).map_err(|e| format!("無法寫入檔案：{e}"))?;

        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let (mut done, mut last_report) = (0u64, 0u64);
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("下載中斷：{e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .map_err(|e| format!("無法寫入檔案：{e}"))?;
            hasher.update(&buf[..n]);
            done += n as u64;
            if done - last_report >= 512 * 1024 {
                last_report = done;
                progress(done, total);
            }
        }
        file.flush().map_err(|e| format!("無法寫入檔案：{e}"))?;
        progress(done, total);

        if done == 0 || (total > 0 && done != total) {
            return Err("下載不完整".into());
        }
        Ok(format!("{:x}", hasher.finalize()))
    }

    /// Linux：資產若是 .tar.gz 就取出 `video-downloader`，否則視為裸執行檔；最後補上執行權限。
    #[cfg(unix)]
    fn prepare_binary(download: &Path, dir: &Path) -> Res<PathBuf> {
        use std::os::unix::fs::PermissionsExt;

        let is_archive = download
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(platform::is_tar_gz);
        let binary = if is_archive {
            extract_binary(download, dir)?
        } else {
            download.to_path_buf()
        };
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("無法設定執行權限：{e}"))?;
        Ok(binary)
    }

    /// Windows：發佈檔本身就是 exe。
    #[cfg(not(unix))]
    fn prepare_binary(download: &Path, _dir: &Path) -> Res<PathBuf> {
        Ok(download.to_path_buf())
    }

    #[cfg(unix)]
    fn extract_binary(archive: &Path, dir: &Path) -> Res<PathBuf> {
        let file = File::open(archive).map_err(|e| format!("無法開啟壓縮檔：{e}"))?;
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        let entries = tar.entries().map_err(|e| format!("壓縮檔格式錯誤：{e}"))?;

        let mut seen = Vec::new();
        for entry in entries {
            let mut entry = entry.map_err(|e| format!("壓縮檔格式錯誤：{e}"))?;
            if !entry.header().entry_type().is_file() {
                continue;
            }
            let path = entry.path().map_err(|e| e.to_string())?.into_owned();
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_owned();
            // 底線與連字號視為相同，容許 video_downloader 與 video-downloader。
            let is_binary = file_name.to_ascii_lowercase().replace('_', "-") == BIN_NAME;
            seen.push(file_name);
            if is_binary {
                let out = dir.join(format!("{BIN_NAME}.new"));
                entry
                    .unpack(&out)
                    .map_err(|e| format!("無法解壓縮執行檔：{e}"))?;
                return Ok(out);
            }
        }
        Err(format!("壓縮檔內找不到 {BIN_NAME}（內容：{}）", seen.join("、")))
    }

    /// 簡單檢查檔頭（Windows 為 PE "MZ"、Linux 為 ELF），避免把錯誤頁面或損毀檔案換成執行檔。
    fn check_magic(path: &Path) -> Res<()> {
        let mut head = [0u8; 4];
        File::open(path)
            .and_then(|mut f| f.read_exact(&mut head))
            .map_err(|e| format!("無法讀取下載的檔案：{e}"))?;
        let ok = if cfg!(windows) {
            &head[..2] == b"MZ"
        } else {
            &head == b"\x7fELF"
        };
        if ok {
            Ok(())
        } else {
            Err("下載的檔案不是有效的執行檔".into())
        }
    }
}

// ───────────────────────── 清理 ─────────────────────────

/// 清除先前異常結束遺留的更新暫存資料夾（超過一天才刪，避免誤刪其他執行中的實例）。
pub fn cleanup_stale_files() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let own = format!("{TEMP_PREFIX}{}", std::process::id());
    let max_age = Duration::from_secs(24 * 60 * 60);

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(TEMP_PREFIX) || name == own {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.elapsed().ok())
            .is_some_and(|age| age > max_age);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

// ───────────────────────── 測試 ─────────────────────────

#[cfg(test)]
mod tests {
    use super::platform::{matches_on, Arch, Os};
    use super::*;

    #[test]
    fn version_compare() {
        assert!(version::is_newer("v0.2.0", "0.1.9"));
        assert!(version::is_newer("1.0.0", "0.9.9"));
        assert!(version::is_newer("v0.1.10", "0.1.9"));
        assert!(!version::is_newer("v0.1.0", "0.1.0"));
        assert!(!version::is_newer("0.1.0", "0.2.0"));
        assert!(!version::is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn linux_x86_64_matching() {
        let m = |n| matches_on(n, Os::Linux, Arch::X86_64);
        assert!(m("video-downloader-linux-x86_64.tar.gz"));
        assert!(m("video-downloader-linux-x86_64"));
        assert!(m("video_downloader_omarchy_wsl_x86_64.tar.gz"));
        assert!(m("video-downloader-linux-amd64.tar.gz"));
        assert!(m("Video_Downloader_linux_x64.tgz"));
        assert!(!m("video-downloader-linux-x86_64.sha256"));
        assert!(!m("video-downloader-linux-aarch64.tar.gz"));
        assert!(!m("video_downloader.exe"));
        assert!(!m("video-downloader.exe"));
    }

    #[test]
    fn linux_aarch64_and_windows_matching() {
        assert!(matches_on("video-downloader-linux-arm64.tar.gz", Os::Linux, Arch::Aarch64));
        assert!(!matches_on("video-downloader-linux-x86_64.tar.gz", Os::Linux, Arch::Aarch64));
        assert!(matches_on("video_downloader.exe", Os::Windows, Arch::X86_64));
        assert!(!matches_on("video_downloader.exe", Os::Windows, Arch::Aarch64));
        assert!(!matches_on("video_downloader.exe", Os::Other, Arch::X86_64));
    }

    #[test]
    fn pick_prefers_archive() {
        let names = ["video-downloader-linux-x86_64", "video-downloader-linux-x86_64.tar.gz"];
        // pick 使用目前平台；只在 Linux x86_64 驗證偏好壓縮檔
        if platform::HOST_OS == Os::Linux && platform::HOST_ARCH == Arch::X86_64 {
            let got = platform::pick(&names, |n| n).unwrap();
            assert!(platform::is_tar_gz(got));
        }
    }

    #[test]
    fn latest_tag_from_redirect_url() {
        let f = github::tag_from_url;
        assert_eq!(
            f("https://github.com/cloverdefa/video-downloader/releases/tag/v1.15.0"),
            Some("v1.15.0")
        );
        assert_eq!(f("https://github.com/cloverdefa/video-downloader/releases"), None);
        assert_eq!(f("https://github.com/x/y/releases/tag/"), None);
        assert_eq!(f("https://github.com/x/y/releases/tag/v1<script>"), None);
    }

    #[test]
    fn parse_asset_list() {
        let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let html = format!(
            r#"<li><a href="/cloverdefa/video-downloader/releases/download/v0.2.0/video_downloader.exe" rel="nofollow">video_downloader.exe</a>
            <span>sha256:{hex}</span></li>
            <li><a href="/cloverdefa/video-downloader/releases/download/v0.2.0/video_downloader_omarchy_wsl_x86_64.tar.gz">x</a></li>
            <li><a href="/cloverdefa/video-downloader/archive/refs/tags/v0.2.0.zip">Source code</a></li>"#
        );
        let v = github::parse_assets(&html, "v0.2.0");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "video_downloader.exe");
        assert_eq!(v[0].sha256.as_deref(), Some(hex));
        assert_eq!(v[1].name, "video_downloader_omarchy_wsl_x86_64.tar.gz");
        assert!(v[1].sha256.is_none());
    }

    #[test]
    fn parse_asset_list_empty_on_layout_change() {
        assert!(github::parse_assets("<div>nothing</div>", "v0.2.0").is_empty());
    }

    #[test]
    fn download_url_is_trusted() {
        assert!(download_url("v1.0.0", "a.tar.gz").starts_with(&download_prefix()));
    }
}
