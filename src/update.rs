//! 從 GitHub Releases 檢查新版本，並下載、驗證、替換目前執行檔。
//!
//! 發佈資產命名（請對照 release workflow，若不同請修改 `PLATFORM_ASSET`）：
//!   Windows x86_64 : `video-downloader.exe`
//! 其他平台沒有對應檔案，只提示新版並引導到下載頁。

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

/// 目前版本：直接取 Cargo.toml 的 version（CI 發佈前請確保與 tag 一致）。
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RELEASES_PAGE: &str = "https://github.com/cloverdefa/video-downloader/releases/latest";
const API_LATEST: &str = "https://api.github.com/repos/cloverdefa/video-downloader/releases/latest";
/// 只信任本 repo 的 release 下載網址，避免被導向其他來源。
const DOWNLOAD_PREFIX: &str = "https://github.com/cloverdefa/video-downloader/releases/download/";
/// Windows x86_64 發佈檔名稱。
const PLATFORM_ASSET: &str = "video-downloader.exe";

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
    /// 啟動時記下執行檔路徑，替換後仍可用來重新啟動。
    exe: PathBuf,
    rx: Option<Receiver<Msg>>,
    /// 靜默檢查（啟動時）：失敗或已是最新都回到 Idle，不顯示任何訊息。
    silent: bool,
    pending_version: Option<String>,
}

impl Updater {
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("video-downloader")),
            rx: None,
            silent: false,
            pending_version: None,
        }
    }

    pub fn busy(&self) -> bool {
        matches!(self.state, State::Checking | State::Installing { .. })
    }

    /// `silent = true`：啟動時自動檢查，只有「有新版」才會改變狀態。
    pub fn check(&mut self, ctx: &egui::Context, silent: bool) {
        if self.busy() {
            return;
        }
        self.silent = silent;
        self.state = State::Checking;
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _ = tx.send(Msg::Checked(fetch_latest()));
            ctx.request_repaint();
        });
    }

    /// 下載並安裝目前 `Available` 的版本。
    pub fn install(&mut self, ctx: &egui::Context) {
        let State::Available(info) = &self.state else {
            return;
        };
        let Some(asset) = info.asset.clone() else {
            return;
        };
        self.pending_version = Some(info.version.clone());
        self.state = State::Installing {
            done: 0,
            total: asset.size,
        };
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let ctx = ctx.clone();
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

    pub fn dismiss(&mut self) {
        if !self.busy() {
            self.state = State::Idle;
        }
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
                self.state = State::Failed(format!("could not restart: {e}"));
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
        ureq::Error::Status(403 | 429, _) => "GitHub API rate limit reached, try again later".into(),
        ureq::Error::Status(404, _) => "release not found".into(),
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(t) => t.to_string(),
    }
}

fn platform_asset() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some(PLATFORM_ASSET)
    } else {
        None
    }
}

fn fetch_latest() -> Result<Option<ReleaseInfo>, String> {
    let release: GhRelease = agent()
        .get(API_LATEST)
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(describe)?
        .into_json()
        .map_err(|e| format!("invalid response: {e}"))?;

    if !is_newer(&release.tag_name, CURRENT_VERSION) {
        return Ok(None);
    }
    let asset = platform_asset()
        .and_then(|name| release.assets.iter().find(|a| a.name == name))
        .map(|a| AssetInfo {
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
        return Err("unexpected download URL".into());
    }
    let dir = std::env::temp_dir().join(format!("video-downloader-update-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create temp dir: {e}"))?;
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
    let mut file = File::create(&download).map_err(|e| format!("cannot write file: {e}"))?;

    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let (mut done, mut last_report) = (0u64, 0u64);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("cannot write file: {e}"))?;
        hasher.update(&buf[..n]);
        done += n as u64;
        if done - last_report >= 512 * 1024 {
            last_report = done;
            progress(done, total);
        }
    }
    file.flush().map_err(|e| format!("cannot write file: {e}"))?;
    drop(file);
    progress(done, total);

    if done == 0 || (total > 0 && done != total) {
        return Err("download incomplete".into());
    }
    if let Some(expected) = &asset.sha256 {
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            return Err("checksum mismatch, update aborted".into());
        }
    }

    check_magic(&download)?;
    self_replace::self_replace(&download).map_err(|e| format!("cannot replace executable: {e}"))
}

/// 簡單檢查檔頭（PE "MZ"），避免把錯誤頁面或損毀檔案換成執行檔。
fn check_magic(path: &Path) -> Result<(), String> {
    let mut head = [0u8; 2];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .map_err(|e| format!("cannot read downloaded file: {e}"))?;
    if &head == b"MZ" {
        Ok(())
    } else {
        Err("downloaded file is not a valid executable".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
