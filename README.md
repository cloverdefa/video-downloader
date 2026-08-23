# Video Downloader

一個基於 **yt-dlp + FFmpeg + Deno** 的 Windows GUI 影片下載工具。

提供簡潔的圖形化介面，使用者只需要貼上影片網址即可開始下載，不需要手動操作 yt-dlp 命令列。

## 功能特色

* Windows 圖形化介面
* 基於 [yt-dlp](https://github.com/yt-dlp/yt-dlp) 進行影片下載
* 使用 [FFmpeg](https://ffmpeg.org/) 進行影片與音訊合併及媒體處理
* 使用 [Deno](https://deno.com/) 作為 JavaScript Runtime
* 支援現代 YouTube 影片解析需求
* Portable 免安裝設計
* Release 版本內建所需執行檔
* 顯示影片名稱
* 顯示下載進度
* 支援取消下載
* 支援 `Enter` 快速開始下載
* 網址輸入框提供滑鼠右鍵選單
* 下載完成後可以直接再次下載
* 顯示目前應用程式版本及第三方元件版本

## 使用方式

### 下載

請前往 GitHub Releases 下載最新版本：

**[下載最新版本](https://github.com/cloverdefa/video-downloader/releases/latest)**

Release 採用 Portable ZIP 格式，不需要安裝程式。

### Windows

1. 下載最新的 `video-downloader-*.zip`。
2. 解壓縮 ZIP。
3. 執行：

```text
video_downloader.exe
```

不需要安裝 Python、yt-dlp、FFmpeg 或 Deno。

### 下載影片

1. 開啟 `video_downloader.exe`。
2. 將影片網址貼到「影片網址」輸入框。
3. 點擊「開始下載」。
4. 程式會使用 yt-dlp 解析影片資訊。
5. 解析完成後開始下載。
6. 下載進度會顯示在進度列。

也可以在網址輸入框中直接按下 `Enter` 開始下載。

## 第三方元件

本專案主要使用以下第三方工具：

| 元件                                         | 用途                                           |
| ------------------------------------------ | -------------------------------------------- |
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) | 影片及音訊下載與網站解析                                 |
| [FFmpeg](https://ffmpeg.org/)              | 影片、音訊處理與格式合併                                 |
| [Deno](https://deno.com/)                  | JavaScript Runtime，提供部分網站所需的 JavaScript 執行環境 |

Release 版本會將上述執行檔直接打包至應用程式中，因此一般使用者不需要另外安裝這些工具。

### yt-dlp

yt-dlp 是本專案主要的下載核心。

實際支援哪些網站與格式，取決於 yt-dlp 本身的支援範圍。

### FFmpeg

FFmpeg 用於影片及音訊的後製處理。

例如某些網站提供：

* 影片串流
* 音訊串流

兩者需要分別下載，再透過 FFmpeg 合併成最終影片檔案。

### Deno

Deno 是 JavaScript Runtime。

現代網站，尤其是 YouTube，其影片解析流程可能需要 JavaScript Runtime 才能取得完整的格式資訊。

本專案將 Deno 一起打包，以提供較完整的 YouTube 下載支援。

如果沒有 Deno，程式仍然可以啟動與執行，但部分 YouTube 影片可能只能取得較低畫質或受限的格式。

## Portable 設計

本專案不提供傳統 Windows Installer。

Release 採用 Portable 方式：

```text
video-downloader-v2026.08.23-windows-x64/
├── video_downloader.exe
├── LICENSE
└── README.md
```

解壓縮後即可直接執行。

應用程式會優先搜尋自身所在目錄中的第三方執行檔，因此不需要修改 Windows `PATH`。

## 系統需求

目前 Release 主要針對：

| 平台            | 支援狀態 |
| ------------- | ---- |
| Windows x64   | 支援   |
| Windows ARM64 | 未提供  |
| macOS         | 不支援  |
| Linux         | 不支援  |

目前版本以 Windows x64 為主要目標平台。

## 開發環境

如果要直接從原始碼執行，需要：

* Windows
* Python 3.11+
* yt-dlp
* FFmpeg
* Deno
* PyInstaller

### 從原始碼執行

```powershell
python video_downloader.py
```

程式會依序尋找第三方工具：

1. 程式所在目錄
2. `%OneDrive%\bin`
3. PyInstaller `_MEIPASS`
4. 系統 `PATH`

因此開發環境可以直接使用系統安裝的 yt-dlp、FFmpeg 與 Deno。

## 建置

### 安裝 PyInstaller

```powershell
python -m pip install --upgrade pip
python -m pip install pyinstaller
```

確認第三方工具可以正常使用：

```powershell
yt-dlp --version
ffmpeg -version
deno --version
```

### 建置執行檔

執行：

```powershell
pyinstaller --clean video_downloader.spec
```

完成後會產生：

```text
dist/
└── video_downloader.exe
```

### `.spec` 建置流程

`video_downloader.spec` 會自動：

1. 搜尋 yt-dlp
2. 搜尋 FFmpeg
3. 搜尋 Deno
4. 取得各元件版本
5. 將必要執行檔加入 PyInstaller package
6. 將版本資訊注入 Runtime Hook
7. 建立 `video_downloader.exe`

其中 yt-dlp 與 FFmpeg 是必要元件。

Deno 則為選用元件；如果建置環境沒有 Deno，仍然可以完成建置，但 YouTube 的部分功能可能受到限制。

## Release 流程

正式版本透過 Git Tag 觸發 GitHub Actions。

例如：

```bash
git checkout main
git pull

git tag v2026.08.23
git push origin v2026.08.23
```

GitHub Actions 會自動：

1. 建立 Windows build environment
2. 安裝 Python
3. 安裝 PyInstaller
4. 下載最新 yt-dlp
5. 下載 FFmpeg
6. 下載 Deno
7. 執行 PyInstaller build
8. 驗證產生的 executable
9. 建立 Portable ZIP
10. 建立 GitHub Release
11. 上傳 ZIP 至 Release Assets

最終會產生：

```text
video-downloader-v2026.08.23-windows-x64.zip
```

## 版本號

Release 使用 Git Tag 作為版本號：

```text
vYYYY.MM.DD
```

例如：

```text
v2026.08.23
```

如果同一天需要發布修正版，可以使用：

```text
v2026.08.23-Fix.1
```

或：

```text
v2026.08.23.1
```

版本號會在 Release build 時注入應用程式，並顯示在 GUI 右上角。

## 注意事項

本專案本身只是 yt-dlp 的圖形化操作介面。

實際下載能力與網站支援情況取決於 yt-dlp。

部分網站可能：

* 要求登入
* 需要 Cookie
* 限制下載頻率
* 使用特殊驗證機制
* 因網站更新而暫時無法下載

因此無法保證所有網站或所有影片都能正常下載。

## 著作權與使用責任

本專案僅提供影片下載工具的圖形化介面。

使用者應自行確認：

* 下載內容是否具有合法下載權利
* 是否符合來源網站的服務條款
* 是否符合所在地相關法律

本專案不授予使用者任何下載受著作權或其他法律限制內容的權利。

第三方元件仍受其各自授權條款約束：

* yt-dlp
* FFmpeg
* Deno

## 授權

本專案採用 MIT License。

詳細內容請參閱 [`LICENSE`](LICENSE)。

Copyright © 2026 DAST

