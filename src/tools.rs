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

/// 內嵌工具解壓到暫存資料夾時使用的名稱前綴（後面接版本號）。
const EMBEDDED_DIR_PREFIX: &str = "video-downloader-";

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

/// 使用者家目錄：優先 `HOME`（Linux / macOS / WSL），其次 `USERPROFILE`（Windows）。
fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Windows 的系統預設下載資料夾（Known Folder，即使被搬到別的磁碟也能正確取得）。
#[cfg(windows)]
fn system_download_dir() -> Option<PathBuf> {
    dirs::download_dir()
}

#[cfg(not(windows))]
fn system_download_dir() -> Option<PathBuf> {
    None
}

/// 下載資料夾：依序找 `~/Downloads`、`~/downloads`、`~/下載`，
/// 接著是 Windows 系統預設下載資料夾，都不存在時退回程式所在目錄。
pub fn default_download_dir() -> PathBuf {
    const CANDIDATES: [&str; 3] = ["Downloads", "downloads", "下載"];

    home_dir()
        .and_then(|home| {
            CANDIDATES
                .iter()
                .map(|name| home.join(name))
                .find(|path| path.is_dir())
        })
        .or_else(|| system_download_dir().filter(|path| path.is_dir()))
        .unwrap_or_else(app_dir)
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

fn embedded_dir_name() -> String {
    format!("{EMBEDDED_DIR_PREFIX}{}", embedded::APP_VERSION)
}

/// 刪除暫存資料夾中舊版本留下的內嵌工具（目前版本的不動）。
pub fn cleanup_old_embedded_dirs() {
    let current = embedded_dir_name();

    let Ok(entries) = fs::read_dir(env::temp_dir()) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if name.starts_with(EMBEDDED_DIR_PREFIX)
            && name.as_ref() != current.as_str()
            && entry.path().is_dir()
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// 暫存資料夾中所有由本程式解壓出的內嵌工具資料夾。
pub fn embedded_dirs() -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(env::temp_dir()) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(EMBEDDED_DIR_PREFIX)
                && entry.path().is_dir()
        })
        .map(|entry| entry.path())
        .collect()
}

fn embedded_tool(name: &str) -> Option<PathBuf> {
    let data: &[u8] = match name {
        "yt-dlp" => embedded::YTDLP?,
        "ffmpeg" => embedded::FFMPEG?,
        "deno" => embedded::DENO?,
        _ => return None,
    };

    let directory = env::temp_dir().join(embedded_dir_name());
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
