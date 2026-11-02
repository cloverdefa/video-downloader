//! 外部工具（yt-dlp / ffmpeg / deno）的尋找、解壓與執行。

use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::embedded;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 建立不會在 Windows 上彈出主控台視窗的 `Command`。
///
/// 所有由 GUI 啟動的外部程式（curl、powershell、yt-dlp…）都應該使用它。
pub fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(program);

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    command
}

pub fn tool_filename(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

pub fn app_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .or_else(|| env::current_dir().ok())
        .unwrap_or_default()
}

pub fn download_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Some(downloads) = env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Downloads"))
        .filter(|path| path.is_dir())
    {
        return downloads;
    }

    app_dir()
}

/// 依序尋找：程式目錄 → OneDrive\bin → 內嵌工具 → PATH。
pub fn tool_path(name: &str) -> Option<PathBuf> {
    let filename = tool_filename(name);

    let mut local = vec![app_dir().join(&filename)];
    if let Some(one_drive) = env::var_os("ONEDRIVE") {
        local.push(PathBuf::from(one_drive).join("bin").join(&filename));
    }

    local
        .into_iter()
        .find(|path| path.is_file())
        .or_else(|| embedded_tool(name))
        .or_else(|| find_in_path(&filename))
}

fn find_in_path(filename: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|directory| directory.join(filename))
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

pub fn tool_version(name: &str) -> String {
    let Some(path) = tool_path(name) else {
        return "missing".into();
    };

    let Ok(output) = hidden_command(path).arg("--version").output() else {
        return "missing".into();
    };

    // FFmpeg 主要將版本資訊寫入 stderr；yt-dlp 與 Deno 通常從 stdout 取得。
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| "missing".into())
}

pub fn open_directory(path: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };

    let _ = Command::new(program).arg(path).spawn();
}
