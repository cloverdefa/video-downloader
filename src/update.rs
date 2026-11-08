//! 從 GitHub Releases 檢查新版本，並下載、驗證、替換目前執行檔。
//!
//! 發佈資產命名（請對照 release workflow，規則見 `matches_platform`）：
//!   Windows x86_64        : `video-downloader.exe`
//!   Linux x86_64 / aarch64 : `video-downloader-linux-<arch>.tar.gz`
//!                            （內含名為 `video-downloader` 的執行檔），或直接是執行檔
//! 其他平台（macOS 等）沒有對應檔案，只提示新版並引導到下載頁。

use eframe::egui;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

use crate::embedded;

/// 目前版本：與 UI 標題列顯示的版本同一個來源。
const CURRENT_VERSION: &str = embedded::APP_VERSION;
const API_LATEST: &str = "https://api.github.com/repos/cloverdefa/video-downloader/releases/latest";
/// 只信任本 repo 的 release 下載網址，避免被導向其他來源。
const DOWNLOAD_PREFIX: &str = "https://github.com/cloverdefa/video-downloader/releases/download/";
/// 發佈資產與壓縮檔內執行檔的名稱。
const BIN_NAME: &str = "video-downloader";
/// 不可能是執行檔的附屬檔案（檢查碼、簽章等），挑選資產時排除。
const IGNORED_SUFFIXES: [&str; 8] = [
    ".sha256", ".sha256sum", ".sig", ".asc", ".txt", ".zip", ".deb", ".rpm",
];
/// 更新用暫存資料夾的名稱前綴（後面接 process id）。
const TEMP_PREFIX: &str = "video-downloader-update-";

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    /// GitHub 會提供 "sha256:<hex>"；舊資產可能沒有。
    digest: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AssetInfo {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ReleaseInfo {
    pub version: String,
    pub notes: String,
    pub page: String,
    /// 目前平台可自動安裝的檔案；None 代表只能手動下載。
    pub asset: Option<AssetInfo>,
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

enum Msg {
    Checked(Result<Option<ReleaseInfo>, String>),
    Progress { done: u64, total: u64 },
    Installed(Result<(), String>),
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
            exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("video-downloader")),
            rx: None,
            silent: false,
            pending_version: None,
            retry: None,
        }
    }

    pub fn busy(&self) -> bool {
        matches!(self.state, State::Checking | State::Installing { .. })
    }

    // ───────────── 給 UI 用的查詢介面 ─────────────

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
                format!("發現新版本 v{}（此平台請手動下載）", info.version)
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
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let _ = tx.send(Msg::Checked(fetch_latest()));
            ctx.request_repaint();
        });
    }

    /// 下載並安裝目前 `Available` 的版本。
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
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let progress_tx = tx.clone();
            let progress_ctx = ctx.clone();
            let result = install_asset(&asset, |done, total| {
                let _ = progress_tx.send(Msg::Progress { done, total });
                progress_ctx.request_repaint();
            });
            let _ = tx.send(Msg::Installed(result));
            ctx.request_repaint();
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
        let mut msgs = Vec::new();
        while let Ok(m) = rx.try_recv() {
            msgs.push(m);
        }
        for m in msgs {
            match m {
                Msg::Checked(Ok(Some(info))) => self.state = State::Available(info),
                Msg::Checked(Ok(None)) => {
                    self.state = if self.silent {
                        State::Idle
                    } else {
                        State::UpToDate
                    }
                }
                Msg::Checked(Err(e)) => {
                    self.state = if self.silent {
                        State::Idle
                    } else {
                        State::Failed(e)
                    }
                }
                Msg::Progress { done, total } => {
                    if matches!(self.state, State::Installing { .. }) {
                        self.state = State::Installing { done, total };
                    }
                }
                Msg::Installed(Ok(())) => {
                    self.retry = None;
                    self.state = State::Installed(self.pending_version.take().unwrap_or_default())
                }
                Msg::Installed(Err(e)) => self.state = State::Failed(e),
            }
        }
        if !self.busy() {
            self.rx = None;
        }
    }
}

// ───────────────────────── 版本比較 ─────────────────────────

fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let core = s.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn is_newer(remote: &str, local: &str) -> bool {
    match (parse_version(remote), parse_version(local)) {
        (Some(r), Some(l)) => r > l,
        _ => false,
    }
}

// ───────────────────────── 網路 ─────────────────────────

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .user_agent(concat!("video-downloader/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .build()
}

fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(403 | 429, _) => "GitHub API 請求次數已達上限，請稍後再試".into(),
        ureq::Error::Status(404, _) => "找不到發佈版本".into(),
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(t) => t.to_string(),
    }
}

/// 這個資產是否是目前平台可自動安裝的執行檔。
fn matches_platform(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    if !name.starts_with(BIN_NAME) {
        return false;
    }

    if cfg!(windows) {
        cfg!(target_arch = "x86_64") && name.ends_with(".exe")
    } else if cfg!(target_os = "linux") {
        let arch: &[&str] = if cfg!(target_arch = "x86_64") {
            &["x86_64", "amd64"]
        } else if cfg!(target_arch = "aarch64") {
            &["aarch64", "arm64"]
        } else {
            return false;
        };
        name.contains("linux")
            && arch.iter().any(|a| name.contains(a))
            && (name.ends_with(".tar.gz") || !IGNORED_SUFFIXES.iter().any(|s| name.ends_with(s)))
    } else {
        false
    }
}

/// 從 release 資產中挑出目前平台的檔案；同時有壓縮檔與裸執行檔時優先用壓縮檔。
fn pick_asset(assets: &[GhAsset]) -> Option<&GhAsset> {
    assets
        .iter()
        .filter(|a| matches_platform(&a.name))
        .min_by_key(|a| u8::from(!a.name.to_ascii_lowercase().ends_with(".tar.gz")))
}

fn fetch_latest() -> Result<Option<ReleaseInfo>, String> {
    let release: GhRelease = agent()
        .get(API_LATEST)
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(describe)?
        .into_json()
        .map_err(|e| format!("回應格式錯誤：{e}"))?;

    if !is_newer(&release.tag_name, CURRENT_VERSION) {
        return Ok(None);
    }
    let asset = pick_asset(&release.assets).map(|a| AssetInfo {
            name: a.name.clone(),
            url: a.browser_download_url.clone(),
            size: a.size,
            sha256: a
                .digest
                .as_deref()
                .and_then(|d| d.strip_prefix("sha256:"))
                .map(str::to_owned),
        });
    Ok(Some(ReleaseInfo {
        version: release.tag_name.trim_start_matches(['v', 'V']).to_owned(),
        notes: release.body.unwrap_or_default(),
        page: release.html_url,
        asset,
    }))
}

// ───────────────────────── 下載與安裝 ─────────────────────────

fn install_asset(asset: &AssetInfo, mut progress: impl FnMut(u64, u64)) -> Result<(), String> {
    if !asset.url.starts_with(DOWNLOAD_PREFIX) {
        return Err("下載網址不在信任範圍內".into());
    }
    let dir = std::env::temp_dir().join(format!("{TEMP_PREFIX}{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("無法建立暫存資料夾：{e}"))?;
    let result = download_and_replace(asset, &dir, &mut progress);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn download_and_replace(
    asset: &AssetInfo,
    dir: &Path,
    progress: &mut impl FnMut(u64, u64),
) -> Result<(), String> {
    let response = agent().get(&asset.url).call().map_err(describe)?;
    let total = response
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(asset.size);
    let mut reader = response.into_reader();
    let download = dir.join(&asset.name);
    let mut file = File::create(&download).map_err(|e| format!("無法寫入檔案：{e}"))?;

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
    drop(file);
    progress(done, total);

    if done == 0 || (total > 0 && done != total) {
        return Err("下載不完整".into());
    }
    if let Some(expected) = &asset.sha256 {
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            return Err("檢查碼不符，已中止更新".into());
        }
    }

    let new_exe = prepare_binary(&download, dir)?;
    check_magic(&new_exe)?;
    self_replace::self_replace(&new_exe).map_err(|e| format!("無法替換執行檔：{e}"))
}

/// Linux：資產若是 .tar.gz 就取出 `video-downloader`，否則視為裸執行檔；最後補上執行權限。
#[cfg(unix)]
fn prepare_binary(download: &Path, dir: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;

    let is_archive = download
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.to_ascii_lowercase().ends_with(".tar.gz"));
    let binary = if is_archive {
        extract_binary(download, dir)?
    } else {
        download.to_path_buf()
    };

    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("無法設定執行權限：{e}"))?;
    Ok(binary)
}

#[cfg(unix)]
fn extract_binary(archive: &Path, dir: &Path) -> Result<PathBuf, String> {
    let file = File::open(archive).map_err(|e| format!("無法開啟壓縮檔：{e}"))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let entries = tar.entries().map_err(|e| format!("壓縮檔格式錯誤：{e}"))?;

    for entry in entries {
        let mut entry = entry.map_err(|e| format!("壓縮檔格式錯誤：{e}"))?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let is_binary = entry.header().entry_type().is_file()
            && path.file_name().and_then(|n| n.to_str()) == Some(BIN_NAME);
        if is_binary {
            let out = dir.join(format!("{BIN_NAME}.new"));
            entry
                .unpack(&out)
                .map_err(|e| format!("無法解壓縮執行檔：{e}"))?;
            return Ok(out);
        }
    }
    Err("壓縮檔內找不到執行檔".into())
}

/// Windows：發佈檔本身就是 exe。
#[cfg(not(unix))]
fn prepare_binary(download: &Path, _dir: &Path) -> Result<PathBuf, String> {
    Ok(download.to_path_buf())
}

/// 簡單檢查檔頭（Windows 為 PE "MZ"、Linux 為 ELF），避免把錯誤頁面或損毀檔案換成執行檔。
fn check_magic(path: &Path) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn linux_asset_matching() {
        assert!(matches_platform("video-downloader-linux-x86_64.tar.gz"));
        assert!(matches_platform("video-downloader-linux-x86_64"));
        assert!(!matches_platform("video-downloader-linux-x86_64.sha256"));
        assert!(!matches_platform("video-downloader-linux-aarch64.tar.gz"));
        assert!(!matches_platform("video-downloader.exe"));
    }

    #[test]
    fn version_compare() {
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }
}
