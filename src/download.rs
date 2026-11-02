//! yt-dlp 參數組合、行程啟動與輸出解析。

use std::{
    io::{self, BufRead, BufReader, Read},
    path::Path,
    process::{Child, Stdio},
    sync::mpsc::Sender,
    thread,
};

use crate::{event::Event, tools::hidden_command};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
    AppleWebKit/537.36 (KHTML, like Gecko) \
    Chrome/122.0.0.0 Safari/537.36";

const VIDEO_FORMAT: &str = "bv*[ext=mp4]+ba[ext=m4a]/bv*+ba/b";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DownloadMode {
    Video,
    Audio,
}

pub fn build_args(
    url: &str,
    mode: DownloadMode,
    output_dir: &Path,
    ffmpeg_dir: &Path,
    deno: Option<&Path>,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-o".into(),
        output_dir
            .join("%(title).80B [%(id)s].%(ext)s")
            .to_string_lossy()
            .into_owned(),
    ];

    match mode {
        DownloadMode::Video => args.extend(["-f", VIDEO_FORMAT].map(String::from)),
        DownloadMode::Audio => args.extend(
            ["-x", "--audio-format", "mp3", "--audio-quality", "0"].map(String::from),
        ),
    }

    // Windows 專用：檔名處理，以及 UTF-8 輸出（GUI 程式的管線預設會用
    // 系統地區編碼，例如 cp950，無法輸出日文等字元）。
    if cfg!(windows) {
        args.extend(["--windows-filenames", "--encoding", "utf-8"].map(String::from));
    }

    args.extend(
        [
            "--no-playlist",
            "--user-agent",
            USER_AGENT,
            "--referer",
            url,
            "--ffmpeg-location",
        ]
        .map(String::from),
    );
    args.push(ffmpeg_dir.to_string_lossy().into_owned());

    if let Some(deno) = deno {
        args.push("--js-runtimes".into());
        args.push(format!("deno:{}", deno.display()));
    }

    args.extend(["--newline", "--", url].map(String::from));
    args
}

/// 啟動 yt-dlp，並建立讀取 stdout / stderr 的背景執行緒。
pub fn spawn(
    ytdlp: &Path,
    args: &[String],
    working_dir: &Path,
    tx: &Sender<Event>,
) -> io::Result<Child> {
    let mut child = hidden_command(ytdlp)
        .args(args)
        .current_dir(working_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(stdout) = child.stdout.take() {
        spawn_reader(stdout, tx.clone(), false);
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_reader(stderr, tx.clone(), true);
    }

    Ok(child)
}

fn spawn_reader<R: Read + Send + 'static>(stream: R, tx: Sender<Event>, log: bool) {
    thread::spawn(move || read_output_lines(BufReader::new(stream), &tx, log));
}

fn read_output_lines<R: BufRead>(mut reader: R, tx: &Sender<Event>, log: bool) {
    let mut bytes = Vec::new();

    loop {
        bytes.clear();

        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&bytes);
                process_output_line(tx, line.trim_end_matches(['\r', '\n']), log);
            }
        }
    }
}

fn process_output_line(tx: &Sender<Event>, line: &str, log: bool) {
    if let Some(title) = parse_title(line) {
        let _ = tx.send(Event::Title(title));
    }

    if let Some(progress) = parse_progress(line) {
        let _ = tx.send(Event::Progress(progress.clamp(0.0, 100.0)));
    }

    if log {
        let _ = tx.send(Event::Log(line.chars().take(500).collect()));
    }
}

fn parse_title(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("[download] Destination: ")
        .or_else(|| line.strip_prefix("Destination: "))?;

    Some(
        Path::new(rest.trim())
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    )
}

fn parse_progress(line: &str) -> Option<f32> {
    let index = line.find('%')?;
    line[..index].split_whitespace().last()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress() {
        assert_eq!(parse_progress("[download]  42.5% of 10MiB"), Some(42.5));
        assert_eq!(parse_progress("[download] Destination: a.mp4"), None);
    }

    #[test]
    fn parses_title() {
        assert_eq!(
            parse_title("[download] Destination: C:/x/My Video [abc].mp4").as_deref(),
            Some("My Video [abc]")
        );
        assert_eq!(parse_title("nothing here"), None);
    }
}
