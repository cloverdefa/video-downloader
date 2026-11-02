//! 透過 GitHub Release 檢查並安裝更新。

use std::{
    env,
    io::{BufRead, BufReader},
    path::Path,
    process::Stdio,
    sync::mpsc::Sender,
    thread,
};

use crate::{
    embedded,
    event::{Event, UpdateInfo},
    tools::hidden_command,
};

const USER_AGENT: &str = "video-downloader";
const RELEASE_LATEST: &str = "https://github.com/cloverdefa/video-downloader/releases/latest";
const RELEASE_TAG_PREFIX: &str = "https://github.com/cloverdefa/video-downloader/releases/tag/";
const RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/cloverdefa/video-downloader/releases/latest/download/";

const STAGE_FILE: &str = if cfg!(windows) {
    "video_downloader.update.exe"
} else {
    "video_downloader.update.tar.gz"
};

const TEMP_DIR: &str = ".video_downloader_update";

/// 刪除更新過程產生的暫存檔（下載檔與解壓資料夾）。
fn remove_update_files(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(STAGE_FILE));
    let _ = std::fs::remove_dir_all(dir.join(TEMP_DIR));
}

/// 啟動時清掉先前失敗或中斷的更新所殘留的檔案。
pub fn cleanup_stale_files() {
    if let Some(dir) = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
    {
        remove_update_files(&dir);
    }
}

pub fn check_for_update(tx: Sender<Event>) {
    thread::spawn(move || {
        let _ = tx.send(Event::UpdateChecked(fetch_update_info()));
    });
}

pub fn download_update(info: UpdateInfo, tx: Sender<Event>) {
    thread::spawn(move || {
        let result = install_update(&info.url, &tx);
        let _ = tx.send(Event::UpdateReady(result));
    });
}

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

fn install_update(url: &str, tx: &Sender<Event>) -> Result<(), String> {
    let executable = env::current_exe().map_err(|e| e.to_string())?;
    let parent = executable.parent().ok_or("找不到程式所在資料夾")?;
    let stage = parent.join(STAGE_FILE);

    let result = download_with_progress(url, &stage, tx)
        .and_then(|()| apply_update(&stage, &executable));

    // 失敗時立即清掉殘留檔；成功時由更新腳本負責清理。
    if result.is_err() {
        remove_update_files(parent);
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

#[cfg(windows)]
fn apply_update(stage: &Path, executable: &Path) -> Result<(), String> {
    let parent = executable.parent().ok_or("找不到程式所在資料夾")?;
    let script = parent.join("video_downloader_update.ps1");
    let quote = |p: &Path| p.to_string_lossy().replace('\'', "''");

    let contents = format!(
        "$ErrorActionPreference='Stop'\n\
         try {{\n\
         Wait-Process -Id {pid} -ErrorAction SilentlyContinue\n\
         Move-Item -Force '{stage}' '{exe}'\n\
         Start-Process '{exe}'\n\
         }} finally {{\n\
         Remove-Item -Force -ErrorAction SilentlyContinue '{stage}'\n\
         Remove-Item -Force -ErrorAction SilentlyContinue $MyInvocation.MyCommand.Path\n\
         }}\n",
        pid = std::process::id(),
        stage = quote(stage),
        exe = quote(executable),
    );
    std::fs::write(&script, contents).map_err(|e| e.to_string())?;

    hidden_command("powershell")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn apply_update(stage: &Path, executable: &Path) -> Result<(), String> {
    let parent = executable.parent().ok_or("找不到程式所在資料夾")?;
    let temp = parent.join(TEMP_DIR);
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

#[cfg(not(any(windows, target_os = "linux")))]
fn apply_update(_stage: &Path, _executable: &Path) -> Result<(), String> {
    Err("目前不支援此作業系統的自動更新".into())
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
