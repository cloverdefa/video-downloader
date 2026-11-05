//! 透過 GitHub Release 檢查並安裝更新。
//!
//! 流程：
//! 1. 以重新導向取得最新 Release 標籤並比較版本。
//! 2. 把更新檔下載到系統快取資料夾（不污染程式所在資料夾）。
//! 3. 下載完成後才置換目前的程式並重新啟動。

use std::{
    env,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Stdio,
    sync::mpsc::Sender,
    thread,
};

use crate::{
    embedded,
    event::{Event, UpdateInfo},
    tools::hidden_command,
};

// ---------------------------------------------------------------------------
// 常數
// ---------------------------------------------------------------------------

const USER_AGENT: &str = "video-downloader";
const RELEASE_LATEST: &str = "https://github.com/cloverdefa/video-downloader/releases/latest";
const RELEASE_TAG_PREFIX: &str = "https://github.com/cloverdefa/video-downloader/releases/tag/";
const RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/cloverdefa/video-downloader/releases/latest/download/";

/// 快取資料夾名稱：`<cache>/video-downloader/update/`。
const APP_DIR: &str = "video-downloader";
const UPDATE_DIR: &str = "update";

/// 下載檔在快取資料夾中的檔名。
const STAGE_FILE: &str = if cfg!(windows) {
    "video_downloader.update.exe"
} else {
    "video_downloader.update.tar.gz"
};

/// 解壓縮用的子資料夾（僅 Linux 使用）。
#[cfg(target_os = "linux")]
const EXTRACT_DIR: &str = "extract";

// ---------------------------------------------------------------------------
// 公開 API
// ---------------------------------------------------------------------------

/// 在背景執行緒檢查是否有新版本。
pub fn check_for_update(tx: Sender<Event>) {
    thread::spawn(move || {
        let _ = tx.send(Event::UpdateChecked(fetch_update_info()));
    });
}

/// 在背景執行緒下載並安裝更新。
pub fn download_update(info: UpdateInfo, tx: Sender<Event>) {
    thread::spawn(move || {
        let result = install_update(&info.url, &tx);
        let _ = tx.send(Event::UpdateReady(result));
    });
}

/// 啟動時清掉先前失敗或中斷的更新所殘留的檔案。
pub fn cleanup_stale_files() {
    if let Ok(cache) = cache_dir() {
        remove_update_files(&cache);
    }

    let Some(exe) = env::current_exe().ok() else {
        return;
    };

    if let Some(dir) = exe.parent() {
        remove_legacy_files(dir);
    }

    #[cfg(windows)]
    cleanup_windows_leftovers(&exe);
}

// ---------------------------------------------------------------------------
// 版本檢查
// ---------------------------------------------------------------------------

fn fetch_update_info() -> Result<Option<UpdateInfo>, String> {
    let tag = latest_release_tag()?;
    let version = tag.strip_prefix('v').unwrap_or(&tag);

    if !is_newer_version(version, embedded::APP_VERSION) {
        return Ok(None);
    }

    // GitHub 會將此固定資產網址重新導向至最新 Release 中同名的檔案。
    Ok(Some(UpdateInfo {
        version: version.to_owned(),
        url: format!("{RELEASE_DOWNLOAD_PREFIX}{}", asset_name()?),
    }))
}

fn asset_name() -> Result<&'static str, String> {
    if !cfg!(target_arch = "x86_64") {
        return Err("目前僅提供 Windows x64 與 Linux x86_64 更新".into());
    }

    if cfg!(target_os = "windows") {
        Ok("video_downloader.exe")
    } else if cfg!(target_os = "linux") {
        Ok("video_downloader_omarchy_wsl_x86_64.tar.gz")
    } else {
        Err("目前不支援此作業系統的自動更新".into())
    }
}

/// 透過 GitHub 網頁的重新導向取得最新 Release 標籤，不呼叫 REST API。
fn latest_release_tag() -> Result<String, String> {
    // Windows 使用 NUL，Unix-like 系統使用 /dev/null，避免把 HTML 寫入 stdout。
    let null_device = if cfg!(windows) { "NUL" } else { "/dev/null" };

    let output = hidden_command("curl")
        .args([
            "-fLsS",
            "--connect-timeout",
            "10",
            "--max-time",
            "30",
            "-A",
            USER_AGENT,
            "-o",
            null_device,
            "-w",
            "%{url_effective}",
            RELEASE_LATEST,
        ])
        .output()
        .map_err(|e| format!("無法啟動 curl：{e}"))?;

    if !output.status.success() {
        let details = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if details.is_empty() {
            format!(
                "無法取得 GitHub 最新 Release（curl 結束碼：{}）",
                output.status
            )
        } else {
            format!("無法取得 GitHub 最新 Release：{details}")
        });
    }

    let effective_url = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let tag = effective_url
        .strip_prefix(RELEASE_TAG_PREFIX)
        .ok_or_else(|| format!("無法從 GitHub Release 網址解析版本標籤：{effective_url}"))?
        .trim_end_matches('/');

    if tag.is_empty() || tag.contains(['/', '?', '#']) {
        return Err(format!("GitHub 回傳了無效的 Release 標籤：{tag}"));
    }

    Ok(tag.to_owned())
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    fn parse(version: &str) -> Vec<u64> {
        version
            .split('.')
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    }

    let (latest, current) = (parse(latest), parse(current));

    for i in 0..latest.len().max(current.len()) {
        let (a, b) = (
            latest.get(i).copied().unwrap_or(0),
            current.get(i).copied().unwrap_or(0),
        );
        if a != b {
            return a > b;
        }
    }

    false
}

// ---------------------------------------------------------------------------
// 快取資料夾與暫存檔管理
// ---------------------------------------------------------------------------

/// 更新用的快取資料夾（不存在時會建立）。
///
/// - Windows：`%LOCALAPPDATA%\video-downloader\update`
/// - Linux：`$XDG_CACHE_HOME` 或 `~/.cache` 下的 `video-downloader/update`
/// - 取不到環境變數時退回系統暫存資料夾。
fn cache_dir() -> Result<PathBuf, String> {
    let base = if cfg!(windows) {
        env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        env::var_os("XDG_CACHE_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    }
    .unwrap_or_else(env::temp_dir);

    let dir = base.join(APP_DIR).join(UPDATE_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("無法建立快取資料夾：{e}"))?;
    Ok(dir)
}

/// 刪除快取資料夾中的更新暫存檔（下載檔與解壓資料夾）。
fn remove_update_files(cache: &Path) {
    let _ = std::fs::remove_file(cache.join(STAGE_FILE));
    #[cfg(target_os = "linux")]
    let _ = std::fs::remove_dir_all(cache.join(EXTRACT_DIR));
}

/// 清除舊版程式曾放在 exe 旁邊的殘留檔案。
fn remove_legacy_files(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(STAGE_FILE));
    let _ = std::fs::remove_file(dir.join("video_downloader_update.ps1"));
    let _ = std::fs::remove_dir_all(dir.join(".video_downloader_update"));
}

// ---------------------------------------------------------------------------
// 下載
// ---------------------------------------------------------------------------

fn install_update(url: &str, tx: &Sender<Event>) -> Result<(), String> {
    let executable = env::current_exe().map_err(|e| e.to_string())?;
    let cache = cache_dir()?;
    let stage = cache.join(STAGE_FILE);

    // 先完整下載到快取，成功後才置換程式。
    let result = download_with_progress(url, &stage, tx)
        .and_then(|()| apply_update(&stage, &cache, &executable));

    // 失敗時立即清掉殘留檔；Linux 成功時由更新腳本負責清理。
    if result.is_err() {
        remove_update_files(&cache);
    }

    result
}

/// 用 curl 下載，並解析 `--progress-bar` 在 stderr 輸出的百分比。
fn download_with_progress(url: &str, dest: &Path, tx: &Sender<Event>) -> Result<(), String> {
    let mut child = hidden_command("curl")
        .args(["-fL", "--progress-bar", "-A", USER_AGENT, url, "-o"])
        .arg(dest)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("無法下載更新：{e}"))?;

    let mut last_error = String::new();

    if let Some(stderr) = child.stderr.take() {
        let mut reader = BufReader::new(stderr);
        let mut chunk = Vec::new();

        // 進度列以 '\r' 覆寫同一行，錯誤訊息則以 '\n' 結尾。
        loop {
            chunk.clear();
            match reader.read_until(b'\r', &mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }

            let text = String::from_utf8_lossy(&chunk);
            for segment in text
                .split(['\r', '\n'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if let Some(progress) = parse_curl_progress(segment) {
                    let _ = tx.send(Event::UpdateProgress(progress));
                } else if segment.starts_with("curl:") {
                    last_error = segment.to_owned();
                }
            }
        }
    }

    let status = child.wait().map_err(|e| e.to_string())?;

    if status.success() {
        Ok(())
    } else if last_error.is_empty() {
        Err("下載更新檔失敗".into())
    } else {
        Err(format!("下載更新檔失敗：{last_error}"))
    }
}

fn parse_curl_progress(segment: &str) -> Option<f32> {
    let (before, _) = segment.rsplit_once('%')?;
    let value = before.split_whitespace().last()?.trim_start_matches('#');
    value.parse::<f32>().ok().map(|p| p.clamp(0.0, 100.0))
}

// ---------------------------------------------------------------------------
// 置換程式：Windows
// ---------------------------------------------------------------------------

/// 在 exe 旁邊產生同名加後綴的路徑，例如 `video_downloader.exe.old`。
#[cfg(windows)]
fn sibling_with_suffix(executable: &Path, suffix: &str) -> Option<PathBuf> {
    let mut name = executable.file_name()?.to_os_string();
    name.push(suffix);
    Some(executable.with_file_name(name))
}

/// 清除 Windows 更新留下的 `.new`（中斷的更新）與 `.old`（上一版備份）。
#[cfg(windows)]
fn cleanup_windows_leftovers(executable: &Path) {
    if let Some(new) = sibling_with_suffix(executable, ".new") {
        let _ = std::fs::remove_file(new);
    }

    if let Some(old) = sibling_with_suffix(executable, ".old") {
        // 舊版程式可能還在結束中，最多重試約 10 秒。
        for _ in 0..20 {
            if !old.exists() || std::fs::remove_file(&old).is_ok() {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(500));
        }
    }
}

/// Windows 不允許覆寫或刪除執行中的 exe，但允許重新命名：
///
/// 1. 把快取中的下載檔搬到 exe 旁的 `.new`（跨磁碟機時改用複製）
/// 2. 目前的 exe 改名為 `.old`
/// 3. `.new` 改名為正式名稱並啟動
///
/// 任一步失敗都會還原，`.old` 會在新版啟動時刪除。
#[cfg(windows)]
fn apply_update(stage: &Path, _cache: &Path, executable: &Path) -> Result<(), String> {
    let old = sibling_with_suffix(executable, ".old").ok_or("無法決定備份檔名")?;
    let new = sibling_with_suffix(executable, ".new").ok_or("無法決定暫存檔名")?;

    let _ = std::fs::remove_file(&old);
    let _ = std::fs::remove_file(&new);

    // 步驟 1：搬到 exe 同一個資料夾，確保後續 rename 不會跨磁碟機。
    move_file(stage, &new).map_err(|e| format!("無法搬移更新檔：{e}"))?;

    // 步驟 2：備份目前的程式。
    if let Err(e) = std::fs::rename(executable, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(format!("無法備份目前的程式：{e}"));
    }

    // 步驟 3：換上新版，失敗則還原，避免留下沒有 exe 的狀況。
    if let Err(e) = std::fs::rename(&new, executable) {
        let _ = std::fs::rename(&old, executable);
        let _ = std::fs::remove_file(&new);
        return Err(format!("無法替換程式：{e}"));
    }

    std::process::Command::new(executable)
        .spawn()
        .map(drop)
        .map_err(|e| format!("無法重新啟動：{e}"))
}

/// 搬移檔案；跨磁碟機導致 rename 失敗時改用複製後刪除來源。
#[cfg(windows)]
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }

    std::fs::copy(from, to).map(drop).inspect_err(|_| {
        let _ = std::fs::remove_file(to);
    })?;
    let _ = std::fs::remove_file(from);
    Ok(())
}

// ---------------------------------------------------------------------------
// 置換程式：Linux
// ---------------------------------------------------------------------------

/// 在快取資料夾解壓，再由背景腳本等目前程序結束後覆寫並重新啟動。
#[cfg(target_os = "linux")]
fn apply_update(stage: &Path, cache: &Path, executable: &Path) -> Result<(), String> {
    let temp = cache.join(EXTRACT_DIR);
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).map_err(|e| e.to_string())?;

    let extracted = hidden_command("tar")
        .arg("-xzf")
        .arg(stage)
        .arg("-C")
        .arg(&temp)
        .status()
        .map_err(|e| e.to_string())?
        .success();

    if !extracted {
        return Err("無法解壓更新檔".into());
    }

    let replacement = temp.join("video_downloader/video_downloader");
    if !replacement.is_file() {
        return Err("更新檔缺少程式".into());
    }

    // cp 不受跨檔案系統限制；啟動新版後再清掉快取中的下載檔與解壓資料夾。
    let quote = |p: &Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"));
    let script = format!(
        "while kill -0 {} 2>/dev/null; do sleep 1; done; \
         cp {rep} {exe}; chmod +x {exe}; {exe} & rm -f {stage}; rm -rf {temp}",
        std::process::id(),
        rep = quote(&replacement),
        exe = quote(executable),
        stage = quote(stage),
        temp = quote(&temp),
    );

    hidden_command("sh")
        .args(["-c", &script])
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// 置換程式：其他平台
// ---------------------------------------------------------------------------

#[cfg(not(any(windows, target_os = "linux")))]
fn apply_update(_stage: &Path, _cache: &Path, _executable: &Path) -> Result<(), String> {
    Err("目前不支援此作業系統的自動更新".into())
}

// ---------------------------------------------------------------------------
// 測試
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{is_newer_version, parse_curl_progress};

    #[test]
    fn parses_curl_progress() {
        assert_eq!(parse_curl_progress("######              45.2%"), Some(45.2));
        assert_eq!(parse_curl_progress("#### 100.0%"), Some(100.0));
        assert_eq!(parse_curl_progress("curl: (22) error"), None);
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer_version("1.2.0", "1.1.9"));
        assert!(is_newer_version("1.10", "1.9.5"));
        assert!(!is_newer_version("1.2", "1.2.0"));
        assert!(!is_newer_version("1.0.0", "1.0.1"));
    }
}
