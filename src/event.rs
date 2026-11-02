/// 從背景執行緒傳回 UI 的事件。
#[derive(Debug)]
pub enum Event {
    Title(String),
    Progress(f32),
    Log(String),
    Finished(Result<(), String>),
    UpdateChecked(Result<Option<UpdateInfo>, String>),
    UpdateReady(Result<(), String>),
}

/// 可用的新版本資訊。
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
}
