# Video Downloader

一個以 Rust、egui 和 [yt-dlp](https://github.com/yt-dlp/yt-dlp) 製作的桌面影片下載工具。貼上影片網址即可解析並下載，程式會顯示影片名稱與下載進度，也可以取消下載。

目前 GitHub Release 提供 Windows x64 執行檔。原始碼及打包設定位於本儲存庫。

## 功能

- 深色圖形介面，以繁體中文顯示
- 使用 yt-dlp 解析網站並下載影片或音訊
- 優先選擇 MP4 影片與 M4A 音訊，並透過 FFmpeg 合併；若不可用則依 yt-dlp 規則選擇其他格式
- 顯示解析狀態、影片名稱及下載進度
- 可取消下載；下載成功後自動開啟輸出資料夾
- 網址欄支援 Enter 快速開始及剪貼簿右鍵選單
- 顯示應用程式、yt-dlp、FFmpeg 與 Deno 版本資訊

## 下載與使用

前往 **[GitHub Releases](https://github.com/cloverdefa/video-downloader/releases/latest)** 下載最新的 `video_downloader.exe`，並在 Windows 上執行。Release 執行檔已內嵌 yt-dlp、FFmpeg 和 Deno，不必另外安裝 Python 或這些工具。

1. 開啟 `video_downloader.exe`。
2. 在「影片網址」欄貼上網址，按「開始下載」或 Enter。
3. 等候解析與下載；需要時按「取消」停止。
4. 下載完成後，程式會開啟存放檔案的資料夾。

檔案會存到執行檔所在的資料夾。檔名格式為影片標題（最多 80 個字元）及影片 ID。請將執行檔放在可寫入的資料夾。

## 執行環境與工具

程式啟動時依序搜尋執行檔所在資料夾、Windows `%OneDrive%\bin`、程式內嵌工具的暫存目錄及系統 `PATH`，尋找下列工具：

| 工具 | 必要性 | 用途 |
| --- | --- | --- |
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) | 必要 | 網站解析與下載 |
| [FFmpeg](https://ffmpeg.org/) | 必要 | 合併影音串流及媒體處理 |
| [Deno](https://deno.com/) | 選用 | 為需要 JavaScript Runtime 的網站（如 YouTube）提供解析支援 |

若從原始碼執行且缺少 yt-dlp 或 FFmpeg，程式會提示錯誤。缺少 Deno 時仍可執行，但 YouTube 可用格式或畫質可能受限。實際網站支援情況取決於 yt-dlp、網站限制及其變更。

## 從原始碼執行

建議使用 Windows 與 Rust stable。另須提供 yt-dlp 與 FFmpeg；Deno 選用。將工具放在 `bin/`（檔名分別為 `yt-dlp.exe`、`ffmpeg.exe`、`deno.exe`）可在編譯時嵌入工具；沒有放入 `bin/` 時，程式會搜尋執行檔所在資料夾及系統 PATH。

```powershell
cargo run
```

在 Windows 上，`yt-dlp.exe`、`ffmpeg.exe` 及選用的 `deno.exe` 可放在程式同一資料夾或加入系統 `PATH`。若要將工具嵌入單一執行檔，請先將工具放入 `bin/`，再建置。

## 建置 Windows 執行檔

安裝 Rust stable，並將 yt-dlp、FFmpeg 及選用的 Deno 放入 `bin/`。這些工具會嵌入執行檔，首次啟動時解壓至暫存目錄供程式呼叫。

```powershell
cargo build --release
```

成功後，執行檔位於 `target/release/video-downloader.exe`。Release workflow 會自動下載並嵌入 yt-dlp、FFmpeg 和 Deno；yt-dlp 與 FFmpeg 是必要工具。

## 發布流程

GitHub Actions 的 Release workflow 支援兩種觸發方式：

- `test` 分支的 Pull Request 合併至 `main` 後自動執行。
- 在 GitHub Actions 手動執行 Release workflow，選擇 `patch`、`minor` 或 `major` 版本類型。

Workflow 會依既有版本計算下一個 `vMAJOR.MINOR.PATCH` 版本，在 Windows runner 取得 Rust 工具鏈、yt-dlp、FFmpeg 與 Deno，將工具嵌入 Rust 建置的 `video_downloader.exe`，最後建立 GitHub Release 並上傳該執行檔。Release 不會建立 Portable ZIP。

## 支援範圍與限制

目前發布的二進位檔目標為 Windows x64。其他平台沒有提供 Release 執行檔；打包 workflow 以 Windows 為目標。

有些網站或影片可能要求登入、Cookie 或其他驗證，也可能限制請求頻率或變更解析方式，因此無法保證所有網址均可下載。此程式不提供 Cookie 管理介面。請確認自己有權下載內容，並遵守來源網站條款及所在地法律。

## 授權

本專案採用 MIT License，詳見 [`LICENSE`](LICENSE)。打包的第三方工具仍受各自授權條款約束。
