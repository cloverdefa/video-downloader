//! 使用者設定的讀取與儲存。

use std::{fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// 使用者自訂的下載資料夾；`None` 代表使用預設位置。
    #[serde(default)]
    pub download_dir: Option<PathBuf>,
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("video-downloader").join("config.json"))
}

impl Settings {
    /// 讀取失敗（不存在、格式錯誤）一律回傳預設值。
    pub fn load() -> Self {
        config_path()
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        let path = config_path()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "找不到設定資料夾"))?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // 先寫暫存檔再改名，避免寫到一半造成設定檔損毀。
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&tmp, &path)
    }
}
