/// 從背景執行緒傳回 UI 的事件。
#[derive(Debug)]
pub enum Event {
    Title(String),
    Progress(f32),
    Log(String),
    Finished(Result<(), String>),
}
