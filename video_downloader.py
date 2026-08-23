#!/usr/bin/env -S uv run
# -*- coding: utf-8 -*-

"""
─────────────────────────────────────────────────────────────
簡易 GUI 影片下載器 (yt-dlp + ffmpeg)
重構重點：
  - 移除未使用的 deno 依賴
  - 修正 subprocess cwd 指向 output_dir
  - 統一 UI 狀態管理（下載中禁用輸入與按鈕）
  - 新增下載完成後可再次下載（不強制關閉視窗）
  - 改用 pathlib 處理路徑
  - 錯誤訊息顯示最後 N 行 stderr，更易 debug
  - 網址輸入列支援滑鼠右鍵貼上選單
  - deno 為選擇性依賴：缺少時警告而不阻擋下載
  - 下載時顯示影片名稱
  - 美化 GUI 外觀
  - 版本資訊從 sys 屬性讀取（由 runtime hook 注入），不再依賴 build_info.py
─────────────────────────────────────────────────────────────
"""

import locale
import os
import re
import shutil
import subprocess
import sys
import threading
import tkinter as tk
import tkinter.ttk as ttk
from pathlib import Path
from tkinter import messagebox

# ✔ PyInstaller runtime fix
if getattr(sys, "frozen", False):
    sys.path.append(sys._MEIPASS)

# ── 國際化 ────────────────────────────────────────────────────────────────────

LANGS: dict[str, dict[str, str]] = {
    "zh-TW": {
        "window_title": "影片下載器",
        "prompt_url": "影片網址",
        "btn_download": "開始下載",
        "btn_cancel": "取消",
        "empty_url": "錯誤：影片 URL 不能為空",
        "fetching_formats": "正在解析影片資訊…",
        "download_success": "下載完成",
        "download_failed": "下載失敗",
        "yt_dlp_missing": "錯誤：找不到 yt-dlp，請確認已安裝並加入 PATH",
        "ffmpeg_missing": "錯誤：找不到 ffmpeg，請確認已安裝並加入 PATH",
        "deno_missing": (
            "未找到 deno，YouTube 下載可能只能取得較低畫質格式。\n"
            "建議將 deno.exe 放在與本程式相同目錄以獲得完整支援。"
        ),
        "cancelled": "已取消下載",
        "ready": "就緒，請輸入網址後開始下載",
        "ctx_paste": "貼上",
        "ctx_copy": "複製",
        "ctx_cut": "剪下",
        "ctx_select_all": "全選",
        "ctx_clear": "清除",
    }
}

_lang = "zh-TW"


def t(key: str) -> str:
    return LANGS[_lang].get(key, key)


# ── 設計常數 ──────────────────────────────────────────────────────────────────

COLOR_BG = "#1e1e2e"
COLOR_SURFACE = "#2a2a3e"
COLOR_SURFACE_ALT = "#323248"
COLOR_ACCENT = "#7c6af7"
COLOR_ACCENT_DIM = "#5a4ec4"
COLOR_SUCCESS = "#3ddba0"
COLOR_WARNING = "#f0a04a"
COLOR_DANGER = "#f06c6c"
COLOR_TEXT = "#e8e8f0"
COLOR_TEXT_DIM = "#888899"
COLOR_BORDER = "#3a3a52"

FONT_UI = ("Microsoft JhengHei UI", 10)
FONT_TITLE = ("Microsoft JhengHei UI", 13, "bold")
FONT_SMALL = ("Microsoft JhengHei UI", 9)

# ── 版本資訊（由 runtime hook 注入至 sys，dev 模式 fallback 為 "dev"）─────────
# spec 的 runtime_hook_versions.py 在 exe 啟動時最早執行，
# 會將 APP_VERSION 等字串寫入 sys 屬性，因此這裡直接讀取即可。
# 開發模式（直接執行 .py）時屬性不存在，fallback 為 "dev"。

APP_VERSION = getattr(sys, "_app_version", "dev")
YTDLP_VERSION = getattr(sys, "_ytdlp_version", "unknown")
FFMPEG_VERSION = getattr(sys, "_ffmpeg_version", "unknown")
DENO_VERSION = getattr(sys, "_deno_version", "unknown")

# ── 工具路徑解析 ──────────────────────────────────────────────────────────────


def get_tool_path(name: str) -> Path | None:
    """
    依序搜尋：
      1. 本程式所在目錄（exe 旁邊，含 deno 的慣例擺放位置）
      2. OneDrive\\bin（Windows 慣例擺放位置）
      3. PyInstaller _MEIPASS（onefile 解壓目錄）
      4. 系統 PATH
    """
    candidates: list[Path] = []

    exe_dir = Path(sys.argv[0]).resolve().parent
    suffix = ".exe" if sys.platform.startswith("win") else ""
    candidates.append(exe_dir / f"{name}{suffix}")

    if sys.platform.startswith("win"):
        one_drive_bin = Path(os.path.expandvars(r"%OneDrive%\bin"))
        candidates.append(one_drive_bin / f"{name}.exe")

    meipass = getattr(sys, "_MEIPASS", None)
    if meipass:
        candidates.append(Path(meipass) / f"{name}{suffix}")

    for p in candidates:
        if p.is_file():
            return p

    found = shutil.which(name)
    if found:
        return Path(found)

    if sys.platform.startswith("win"):
        found = shutil.which(f"{name}.exe")
        if found:
            return Path(found)

    return None


def open_directory(path: Path) -> None:
    try:
        if sys.platform.startswith("win"):
            subprocess.run(["explorer", str(path)], check=False)
        elif sys.platform == "darwin":
            subprocess.run(["open", str(path)], check=False)
        else:
            subprocess.run(["xdg-open", str(path)], check=False)
    except Exception:
        pass


# ── 進度 / 影片名稱解析 ───────────────────────────────────────────────────────

_PROGRESS_RE = re.compile(r"(\d{1,3}(?:\.\d+)?)%")
_DEST_RE = re.compile(r"\[download\]\s+Destination:\s+(.+)")
_DEST_BARE_RE = re.compile(r"^Destination:\s+(.+)")


def parse_progress(line: str) -> float | None:
    m = _PROGRESS_RE.search(line)
    return float(m.group(1)) if m else None


def parse_title(line: str) -> str | None:
    """從 yt-dlp 輸出行解析目標檔案名稱，去除副檔名後作為影片標題。"""
    m = _DEST_RE.search(line) or _DEST_BARE_RE.search(line)
    if m:
        return Path(m.group(1).strip()).stem
    return None


# ── 主視窗 ────────────────────────────────────────────────────────────────────


class VideoDownloaderApp:
    MAX_LOG_LINES = 30

    def __init__(self, root: tk.Tk) -> None:
        self.root = root
        self.root.title(t("window_title"))
        self.root.geometry("520x400")
        self.root.resizable(False, False)
        self.root.configure(bg=COLOR_BG)

        self._process: subprocess.Popen | None = None
        self._cancelled = False

        self._apply_ttk_style()
        self._build_ui()

    # ── ttk 樣式 ──────────────────────────────────────────────────────────────

    def _apply_ttk_style(self) -> None:
        style = ttk.Style(self.root)
        style.theme_use("clam")
        style.configure(
            "Custom.Horizontal.TProgressbar",
            troughcolor=COLOR_SURFACE,
            background=COLOR_ACCENT,
            bordercolor=COLOR_BORDER,
            lightcolor=COLOR_ACCENT,
            darkcolor=COLOR_ACCENT,
            thickness=6,
        )

    # ── UI 建構 ───────────────────────────────────────────────────────────────

    def _build_ui(self) -> None:
        # ── 標題列 ────────────────────────────────────────
        title_frame = tk.Frame(self.root, bg=COLOR_BG)
        title_frame.pack(fill="x", padx=24, pady=(24, 0))

        tk.Label(
            title_frame,
            text="⬇",
            font=("Segoe UI Emoji", 20),
            bg=COLOR_BG,
            fg=COLOR_ACCENT,
        ).pack(side="left", padx=(0, 10))

        tk.Label(
            title_frame,
            text=t("window_title"),
            font=FONT_TITLE,
            bg=COLOR_BG,
            fg=COLOR_TEXT,
        ).pack(side="left")

        # 版本按鈕：顯示 "v0.10.6" 而非單純 "v"
        tk.Button(
            title_frame,
            text=f"v{APP_VERSION}",
            font=FONT_SMALL,
            bg=COLOR_SURFACE_ALT,
            fg=COLOR_TEXT_DIM,
            relief="flat",
            bd=0,
            command=self._show_version_info,
            cursor="hand2",
        ).pack(side="right")

        # 分隔線
        tk.Frame(self.root, height=1, bg=COLOR_BORDER).pack(
            fill="x", padx=24, pady=(14, 0)
        )

        # ── 網址輸入區 ────────────────────────────────────
        url_frame = tk.Frame(self.root, bg=COLOR_BG)
        url_frame.pack(fill="x", padx=24, pady=(18, 0))

        tk.Label(
            url_frame,
            text=t("prompt_url"),
            font=FONT_SMALL,
            bg=COLOR_BG,
            fg=COLOR_TEXT_DIM,
        ).pack(anchor="w", pady=(0, 6))

        # 輸入框外框（模擬 border + focus ring）
        self._entry_outer = tk.Frame(url_frame, bg=COLOR_BORDER, padx=1, pady=1)
        self._entry_outer.pack(fill="x")

        entry_inner = tk.Frame(self._entry_outer, bg=COLOR_SURFACE)
        entry_inner.pack(fill="x")

        self.entry = tk.Entry(
            entry_inner,
            font=FONT_UI,
            bg=COLOR_SURFACE,
            fg=COLOR_TEXT,
            insertbackground=COLOR_TEXT,
            relief="flat",
            bd=8,
        )
        self.entry.pack(fill="x")
        self.entry.bind("<Return>", lambda _: self._on_download())
        self.entry.bind(
            "<FocusIn>", lambda _: self._entry_outer.config(bg=COLOR_ACCENT)
        )
        self.entry.bind(
            "<FocusOut>", lambda _: self._entry_outer.config(bg=COLOR_BORDER)
        )
        self._bind_entry_context_menu(self.entry)

        # ── 按鈕列 ────────────────────────────────────────
        btn_frame = tk.Frame(self.root, bg=COLOR_BG)
        btn_frame.pack(fill="x", padx=24, pady=(14, 0))

        self.btn_download = self._make_button(
            btn_frame,
            text=t("btn_download"),
            command=self._on_download,
            primary=True,
        )
        self.btn_download.pack(side="left")

        self.btn_cancel = self._make_button(
            btn_frame,
            text=t("btn_cancel"),
            command=self._on_cancel,
            primary=False,
        )
        self.btn_cancel.pack(side="left", padx=(10, 0))
        self.btn_cancel.config(state="disabled")

        # ── 影片名稱卡片 ──────────────────────────────────
        title_card = tk.Frame(self.root, bg=COLOR_SURFACE)
        title_card.pack(fill="x", padx=24, pady=(18, 0))

        inner = tk.Frame(title_card, bg=COLOR_SURFACE)
        inner.pack(fill="x", padx=14, pady=10)

        tk.Label(
            inner,
            text="影片",
            font=FONT_SMALL,
            bg=COLOR_SURFACE,
            fg=COLOR_TEXT_DIM,
        ).pack(anchor="w")

        self.title_var = tk.StringVar(value="—")
        tk.Label(
            inner,
            textvariable=self.title_var,
            font=FONT_UI,
            bg=COLOR_SURFACE,
            fg=COLOR_TEXT,
            anchor="w",
            wraplength=440,
            justify="left",
        ).pack(anchor="w", pady=(2, 0))

        # ── 進度列 ────────────────────────────────────────
        progress_frame = tk.Frame(self.root, bg=COLOR_BG)
        progress_frame.pack(fill="x", padx=24, pady=(16, 0))

        self.progress_var = tk.DoubleVar()
        self.progress_bar = ttk.Progressbar(
            progress_frame,
            variable=self.progress_var,
            maximum=100,
            style="Custom.Horizontal.TProgressbar",
        )
        self.progress_bar.pack(fill="x")

        self.pct_var = tk.StringVar(value="")
        tk.Label(
            progress_frame,
            textvariable=self.pct_var,
            font=FONT_SMALL,
            bg=COLOR_BG,
            fg=COLOR_TEXT_DIM,
            anchor="e",
        ).pack(fill="x", pady=(3, 0))

        # ── 狀態列 ────────────────────────────────────────
        status_frame = tk.Frame(self.root, bg=COLOR_BG)
        status_frame.pack(fill="x", padx=24, pady=(8, 20))

        self.status_dot = tk.Label(
            status_frame,
            text="●",
            font=("Segoe UI", 8),
            bg=COLOR_BG,
            fg=COLOR_TEXT_DIM,
        )
        self.status_dot.pack(side="left", padx=(0, 6))

        self.status_label = tk.Label(
            status_frame,
            text=t("ready"),
            font=FONT_SMALL,
            bg=COLOR_BG,
            fg=COLOR_TEXT_DIM,
            anchor="w",
        )
        self.status_label.pack(side="left", fill="x", expand=True)

    def _make_button(
        self, parent: tk.Frame, text: str, command, primary: bool
    ) -> tk.Button:
        if primary:
            bg, fg, abg = COLOR_ACCENT, "#ffffff", COLOR_ACCENT_DIM
        else:
            bg, fg, abg = COLOR_SURFACE_ALT, COLOR_TEXT_DIM, COLOR_BORDER

        return tk.Button(
            parent,
            text=text,
            font=FONT_UI,
            bg=bg,
            fg=fg,
            activebackground=abg,
            activeforeground=fg,
            relief="flat",
            bd=0,
            padx=18,
            pady=8,
            cursor="hand2",
            command=command,
        )

    def _show_version_info(self) -> None:
        messagebox.showinfo(
            "版本資訊",
            (
                f"影片下載器 v{APP_VERSION}\n\n"
                f"yt-dlp : {YTDLP_VERSION}\n"
                f"ffmpeg : {FFMPEG_VERSION}\n"
                f"deno   : {DENO_VERSION}"
            ),
        )

    # ── 右鍵選單 ──────────────────────────────────────────────────────────────

    def _bind_entry_context_menu(self, entry: tk.Entry) -> None:
        menu = tk.Menu(
            entry,
            tearoff=0,
            bg=COLOR_SURFACE,
            fg=COLOR_TEXT,
            activebackground=COLOR_ACCENT,
            activeforeground="#ffffff",
            relief="flat",
            bd=1,
        )

        def do_cut():
            if entry["state"] == "disabled":
                return
            try:
                entry.event_generate("<<Cut>>")
            except tk.TclError:
                pass

        def do_copy():
            try:
                entry.event_generate("<<Copy>>")
            except tk.TclError:
                pass

        def do_paste():
            if entry["state"] == "disabled":
                return
            try:
                try:
                    entry.delete(tk.SEL_FIRST, tk.SEL_LAST)
                except tk.TclError:
                    pass
                entry.insert(tk.INSERT, self.root.clipboard_get())
            except tk.TclError:
                pass

        def do_select_all():
            entry.select_range(0, tk.END)
            entry.icursor(tk.END)

        def do_clear():
            if entry["state"] == "disabled":
                return
            entry.delete(0, tk.END)

        menu.add_command(label=t("ctx_cut"), command=do_cut)
        menu.add_command(label=t("ctx_copy"), command=do_copy)
        menu.add_command(label=t("ctx_paste"), command=do_paste)
        menu.add_separator()
        menu.add_command(label=t("ctx_select_all"), command=do_select_all)
        menu.add_command(label=t("ctx_clear"), command=do_clear)

        def show_menu(event: tk.Event) -> None:
            is_editable = entry["state"] != "disabled"
            write_state = "normal" if is_editable else "disabled"
            menu.entryconfig(t("ctx_cut"), state=write_state)
            menu.entryconfig(t("ctx_paste"), state=write_state)
            menu.entryconfig(t("ctx_clear"), state=write_state)

            has_selection = bool(entry.selection_present())
            if not has_selection:
                menu.entryconfig(t("ctx_cut"), state="disabled")
                menu.entryconfig(t("ctx_copy"), state="disabled")

            menu.tk_popup(event.x_root, event.y_root)

        entry.bind("<Button-3>", show_menu)
        if sys.platform == "darwin":
            entry.bind("<Button-2>", show_menu)

    # ── 狀態輔助 ──────────────────────────────────────────────────────────────

    def _set_status(self, text: str, dot_color: str = COLOR_TEXT_DIM) -> None:
        self.status_label.config(text=text)
        self.status_dot.config(fg=dot_color)

    def _set_title_display(self, title: str) -> None:
        self.title_var.set(title if title else "—")

    def _set_downloading(self, downloading: bool) -> None:
        state_entry = "disabled" if downloading else "normal"
        state_download = "disabled" if downloading else "normal"
        state_cancel = "normal" if downloading else "disabled"

        self.entry.config(state=state_entry)
        self.btn_download.config(state=state_download)
        self.btn_cancel.config(state=state_cancel)

        if not downloading:
            self.progress_var.set(0)
            self.pct_var.set("")

    # ── 事件處理 ──────────────────────────────────────────────────────────────

    def _on_download(self) -> None:
        url = self.entry.get().strip()
        if not url:
            messagebox.showwarning("錯誤", t("empty_url"))
            return

        yt_dlp = get_tool_path("yt-dlp")
        ffmpeg = get_tool_path("ffmpeg")

        if not yt_dlp:
            messagebox.showerror("錯誤", t("yt_dlp_missing"))
            return
        if not ffmpeg:
            messagebox.showerror("錯誤", t("ffmpeg_missing"))
            return

        deno = get_tool_path("deno")
        if not deno:
            messagebox.showwarning("提示", t("deno_missing"))

        self._cancelled = False
        self._set_downloading(True)
        self._set_title_display("")
        self._set_status(t("fetching_formats"), COLOR_WARNING)

        output_dir = Path(sys.argv[0]).resolve().parent

        args = [
            str(yt_dlp),
            "-o",
            str(output_dir / "%(title)s.%(ext)s"),
            "-f",
            "bv*[ext=mp4]+ba[ext=m4a]/bv*+ba/b",
            url,
            "--no-playlist",
            "--user-agent",
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
                "AppleWebKit/537.36 (KHTML, like Gecko) "
                "Chrome/122.0.0.0 Safari/537.36"
            ),
            "--referer",
            url,
            "--ffmpeg-location",
            str(ffmpeg.parent),
            "--newline",
        ]

        threading.Thread(
            target=self._run_download,
            args=(args, output_dir),
            daemon=True,
        ).start()

    def _on_cancel(self) -> None:
        self._cancelled = True
        if self._process and self._process.poll() is None:
            self._process.terminate()
        self._set_status(t("cancelled"), COLOR_WARNING)
        self._set_downloading(False)

    # ── 下載執行緒 ────────────────────────────────────────────────────────────

    def _run_download(self, args: list[str], output_dir: Path) -> None:
        log_lines: list[str] = []

        creationflags = 0x08000000 if sys.platform.startswith("win") else 0

        sys_encoding = locale.getpreferredencoding(False)

        child_env = os.environ.copy()
        child_env.pop("PYTHONUTF8", None)
        child_env.pop("PYTHONIOENCODING", None)

        self._process = subprocess.Popen(
            args,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            encoding=sys_encoding,
            errors="replace",
            cwd=str(output_dir),
            env=child_env,
            creationflags=creationflags,
        )

        assert self._process.stdout is not None

        for line in self._process.stdout:
            if self._cancelled:
                break

            log_lines.append(line)
            if len(log_lines) > self.MAX_LOG_LINES:
                log_lines.pop(0)

            title = parse_title(line)
            if title and self.title_var.get() == "—":
                self.root.after(0, self._set_title_display, title)

            pct = parse_progress(line)
            if pct is not None:
                self.root.after(0, self.progress_var.set, pct)
                self.root.after(0, self.pct_var.set, f"{pct:.1f}%")
                self.root.after(0, self._set_status, "下載中…", COLOR_ACCENT)

        if self._process.poll() is None:
            self._process.wait()

        returncode = self._process.returncode

        if self._cancelled:
            return

        if returncode == 0:
            self.root.after(0, self._on_success, output_dir)
        else:
            err = "".join(log_lines).strip() or t("download_failed")
            self.root.after(0, self._on_failure, err)

    # ── 完成回呼 ──────────────────────────────────────────────────────────────

    def _on_success(self, output_dir: Path) -> None:
        self.progress_var.set(100)
        self.pct_var.set("100%")
        self._set_status(t("download_success"), COLOR_SUCCESS)
        self._set_downloading(False)
        open_directory(output_dir)

    def _on_failure(self, err: str) -> None:
        self._set_status(t("download_failed"), COLOR_DANGER)
        self._set_downloading(False)
        messagebox.showerror("yt-dlp 錯誤", err)


# ── 進入點 ────────────────────────────────────────────────────────────────────


def main() -> None:
    root = tk.Tk()
    VideoDownloaderApp(root)
    root.mainloop()


if __name__ == "__main__":
    main()
