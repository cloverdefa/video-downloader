#!/usr/bin/env -S uv run
# -*- coding: utf-8 -*-

"""
─────────────────────────────────────────────────────────────
簡易 GUI 影片下載器 (yt-dlp + ffmpeg)

重構重點：
  - PyInstaller EXE 支援 Windows Per-Monitor DPI Awareness V2
  - 視窗可自由拖動尺寸
  - Responsive GUI Layout
  - 視窗變寬時，輸入區 / 影片名稱 / 進度條自動延伸
  - 視窗變高時，主要內容區自動延伸
  - 不使用固定視窗尺寸
  - 左右控制區使用 Grid 配置
  - 影片名稱 wraplength 會依視窗寬度動態調整
  - 移除未使用的 deno 強制依賴
  - 修正 subprocess cwd 指向 output_dir
  - 統一 UI 狀態管理
  - 下載完成後可以再次下載
  - pathlib 處理路徑
  - 錯誤訊息顯示最後 N 行 stderr
  - 網址輸入列支援滑鼠右鍵貼上選單
  - deno 為選擇性依賴
  - 下載時顯示影片名稱
  - 版本資訊從 sys 屬性讀取
─────────────────────────────────────────────────────────────
"""

from __future__ import annotations

import ctypes
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


# ═════════════════════════════════════════════════════════════
# PyInstaller / Windows DPI
# ═════════════════════════════════════════════════════════════


if getattr(sys, "frozen", False):
    # PyInstaller runtime fix
    meipass = getattr(sys, "_MEIPASS", None)
    if meipass:
        sys.path.append(str(meipass))


def _enable_windows_dpi_awareness() -> None:
    """
    啟用 Windows Per-Monitor DPI Awareness V2。

    目的：
      - 避免 125% / 150% / 175% / 200% DPI 下 GUI 模糊
      - 避免 Windows 對 PyInstaller EXE 進行額外 Virtual Scaling

    注意：
      不額外呼叫 tk scaling，避免 DPI 二次放大。
    """

    if sys.platform != "win32":
        return

    # Windows 10 1703+
    try:
        user32 = ctypes.windll.user32

        user32.SetProcessDpiAwarenessContext.argtypes = [
            ctypes.c_void_p
        ]
        user32.SetProcessDpiAwarenessContext.restype = ctypes.c_bool

        # DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4
        context = ctypes.c_void_p(-4)

        if user32.SetProcessDpiAwarenessContext(context):
            return

    except (AttributeError, OSError):
        pass

    # Windows 8.1 fallback
    try:
        shcore = ctypes.windll.shcore

        shcore.SetProcessDpiAwareness.argtypes = [
            ctypes.c_int
        ]
        shcore.SetProcessDpiAwareness.restype = ctypes.c_int

        # PROCESS_PER_MONITOR_DPI_AWARE = 2
        shcore.SetProcessDpiAwareness(2)

    except (AttributeError, OSError):
        pass


_enable_windows_dpi_awareness()


# ═════════════════════════════════════════════════════════════
# 國際化
# ═════════════════════════════════════════════════════════════


LANGS: dict[str, dict[str, str]] = {
    "zh-TW": {
        "window_title": "影片下載器",
        "prompt_url": "影片網址",
        "btn_download": "開始下載",
        "btn_cancel": "取消",
        "empty_url": "錯誤：影片 URL 不能為空",
        "fetching_formats": "正在解析影片資訊…",
        "downloading": "下載中…",
        "download_success": "下載完成",
        "download_failed": "下載失敗",
        "yt_dlp_missing": (
            "錯誤：找不到 yt-dlp，"
            "請確認已安裝並加入 PATH"
        ),
        "ffmpeg_missing": (
            "錯誤：找不到 ffmpeg，"
            "請確認已安裝並加入 PATH"
        ),
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
        "video": "影片",
        "version_info": "版本資訊",
        "version_title": "影片下載器",
        "yt_dlp": "yt-dlp",
        "ffmpeg": "ffmpeg",
        "deno": "deno",
    }
}


_lang = "zh-TW"


def t(key: str) -> str:
    return LANGS[_lang].get(key, key)


# ═════════════════════════════════════════════════════════════
# 視窗設定
# ═════════════════════════════════════════════════════════════


DEFAULT_WIDTH = 520
DEFAULT_HEIGHT = 400

MIN_WIDTH = 420
MIN_HEIGHT = 360


# ═════════════════════════════════════════════════════════════
# 色彩
# ═════════════════════════════════════════════════════════════


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


# ═════════════════════════════════════════════════════════════
# 字型
# ═════════════════════════════════════════════════════════════


FONT_UI = (
    "Microsoft JhengHei UI",
    10,
)

FONT_TITLE = (
    "Microsoft JhengHei UI",
    13,
    "bold",
)

FONT_SMALL = (
    "Microsoft JhengHei UI",
    9,
)


# ═════════════════════════════════════════════════════════════
# 版本資訊
# ═════════════════════════════════════════════════════════════


APP_VERSION = getattr(
    sys,
    "_app_version",
    "dev",
)

YTDLP_VERSION = getattr(
    sys,
    "_ytdlp_version",
    "unknown",
)

FFMPEG_VERSION = getattr(
    sys,
    "_ffmpeg_version",
    "unknown",
)

DENO_VERSION = getattr(
    sys,
    "_deno_version",
    "unknown",
)


# ═════════════════════════════════════════════════════════════
# 工具路徑
# ═════════════════════════════════════════════════════════════


def get_tool_path(
    name: str,
) -> Path | None:
    """
    工具搜尋順序：

      1. EXE 所在目錄
      2. OneDrive\\bin
      3. PyInstaller _MEIPASS
      4. 系統 PATH
    """

    candidates: list[Path] = []

    exe_dir = (
        Path(sys.argv[0])
        .resolve()
        .parent
    )

    suffix = (
        ".exe"
        if sys.platform.startswith("win")
        else ""
    )

    candidates.append(
        exe_dir / f"{name}{suffix}"
    )

    # Windows OneDrive\\bin
    if sys.platform.startswith("win"):
        one_drive_bin = Path(
            os.path.expandvars(
                r"%OneDrive%\bin"
            )
        )

        candidates.append(
            one_drive_bin / f"{name}.exe"
        )

    # PyInstaller _MEIPASS
    meipass = getattr(
        sys,
        "_MEIPASS",
        None,
    )

    if meipass:
        candidates.append(
            Path(meipass)
            / f"{name}{suffix}"
        )

    for path in candidates:
        if path.is_file():
            return path

    found = shutil.which(name)

    if found:
        return Path(found)

    if sys.platform.startswith("win"):
        found = shutil.which(
            f"{name}.exe"
        )

        if found:
            return Path(found)

    return None


# ═════════════════════════════════════════════════════════════
# 開啟目錄
# ═════════════════════════════════════════════════════════════


def open_directory(
    path: Path,
) -> None:

    try:
        if sys.platform.startswith("win"):
            subprocess.run(
                [
                    "explorer",
                    str(path),
                ],
                check=False,
            )

        elif sys.platform == "darwin":
            subprocess.run(
                [
                    "open",
                    str(path),
                ],
                check=False,
            )

        else:
            subprocess.run(
                [
                    "xdg-open",
                    str(path),
                ],
                check=False,
            )

    except Exception:
        pass


# ═════════════════════════════════════════════════════════════
# yt-dlp 輸出解析
# ═════════════════════════════════════════════════════════════


_PROGRESS_RE = re.compile(
    r"(\d{1,3}(?:\.\d+)?)%"
)

_DEST_RE = re.compile(
    r"\[download\]\s+Destination:\s+(.+)"
)

_DEST_BARE_RE = re.compile(
    r"^Destination:\s+(.+)"
)


def parse_progress(
    line: str,
) -> float | None:

    match = _PROGRESS_RE.search(line)

    if match:
        try:
            return float(
                match.group(1)
            )
        except ValueError:
            pass

    return None


def parse_title(
    line: str,
) -> str | None:
    """
    從 yt-dlp 輸出中解析目標檔案名稱。
    """

    match = (
        _DEST_RE.search(line)
        or _DEST_BARE_RE.search(line)
    )

    if not match:
        return None

    try:
        return Path(
            match.group(1).strip()
        ).stem
    except Exception:
        return None


# ═════════════════════════════════════════════════════════════
# 主程式
# ═════════════════════════════════════════════════════════════


class VideoDownloaderApp:

    MAX_LOG_LINES = 30

    def __init__(
        self,
        root: tk.Tk,
    ) -> None:

        self.root = root

        # ────────────────────────────────────────────────
        # 視窗
        # ────────────────────────────────────────────────

        self.root.title(
            t("window_title")
        )

        self.root.geometry(
            f"{DEFAULT_WIDTH}x"
            f"{DEFAULT_HEIGHT}"
        )

        self.root.minsize(
            MIN_WIDTH,
            MIN_HEIGHT,
        )

        # 關鍵：
        # 允許使用者拖動視窗尺寸。
        self.root.resizable(
            True,
            True,
        )

        self.root.configure(
            bg=COLOR_BG
        )

        # ────────────────────────────────────────────────
        # Process state
        # ────────────────────────────────────────────────

        self._process: subprocess.Popen | None = None

        self._cancelled = False

        # 紀錄目前視窗寬度，
        # 用來動態調整影片名稱 wraplength。
        self._last_window_width = DEFAULT_WIDTH

        # ────────────────────────────────────────────────
        # UI
        # ────────────────────────────────────────────────

        self._apply_ttk_style()
        self._build_ui()

        # 視窗尺寸變更事件
        self.root.bind(
            "<Configure>",
            self._on_window_configure,
        )

        # 關閉視窗
        self.root.protocol(
            "WM_DELETE_WINDOW",
            self._on_close,
        )

    # ═════════════════════════════════════════════════════
    # ttk Style
    # ═════════════════════════════════════════════════════

    def _apply_ttk_style(self) -> None:

        style = ttk.Style(
            self.root
        )

        style.theme_use(
            "clam"
        )

        style.configure(
            "Custom.Horizontal.TProgressbar",
            troughcolor=COLOR_SURFACE,
            background=COLOR_ACCENT,
            bordercolor=COLOR_BORDER,
            lightcolor=COLOR_ACCENT,
            darkcolor=COLOR_ACCENT,
            thickness=6,
        )

    # ═════════════════════════════════════════════════════
    # UI Build
    # ═════════════════════════════════════════════════════

    def _build_ui(self) -> None:
        """
        Responsive layout：

        root
        └── main
            ├── title
            ├── separator
            ├── content
            │   ├── url section
            │   ├── button section
            │   ├── video card
            │   └── progress
            └── status

        使用 grid 讓主要內容可以隨視窗寬度伸縮。
        """

        main = tk.Frame(
            self.root,
            bg=COLOR_BG,
        )

        main.pack(
            fill=tk.BOTH,
            expand=True,
            padx=24,
            pady=(20, 18),
        )

        # 主體只有單欄
        main.grid_columnconfigure(
            0,
            weight=1,
        )

        # Result / content 不存在獨立固定高度，
        # 主要由內容自然高度決定。
        main.grid_rowconfigure(
            0,
            weight=0,
        )

        main.grid_rowconfigure(
            1,
            weight=0,
        )

        main.grid_rowconfigure(
            2,
            weight=0,
        )

        main.grid_rowconfigure(
            3,
            weight=0,
        )

        main.grid_rowconfigure(
            4,
            weight=0,
        )

        # ────────────────────────────────────────────────
        # Title
        # ────────────────────────────────────────────────

        self._build_title(
            main
        )

        # ────────────────────────────────────────────────
        # URL
        # ────────────────────────────────────────────────

        self._build_url_section(
            main
        )

        # ────────────────────────────────────────────────
        # Buttons
        # ────────────────────────────────────────────────

        self._build_button_section(
            main
        )

        # ────────────────────────────────────────────────
        # Video / progress
        # ────────────────────────────────────────────────

        self._build_video_section(
            main
        )

        # ────────────────────────────────────────────────
        # Status
        # ────────────────────────────────────────────────

        self._build_status_section(
            main
        )

    # ═════════════════════════════════════════════════════
    # Title
    # ═════════════════════════════════════════════════════

    def _build_title(
        self,
        parent: tk.Frame,
    ) -> None:

        title_frame = tk.Frame(
            parent,
            bg=COLOR_BG,
        )

        title_frame.grid(
            row=0,
            column=0,
            sticky="ew",
        )

        title_frame.grid_columnconfigure(
            0,
            weight=0,
        )

        title_frame.grid_columnconfigure(
            1,
            weight=1,
        )

        title_frame.grid_columnconfigure(
            2,
            weight=0,
        )

        # Emoji / Icon
        tk.Label(
            title_frame,
            text="⬇",
            font=("Segoe UI Emoji", 20),
            bg=COLOR_BG,
            fg=COLOR_ACCENT,
        ).grid(
            row=0,
            column=0,
            padx=(0, 10),
            sticky="w",
        )

        # Title
        tk.Label(
            title_frame,
            text=t("window_title"),
            font=FONT_TITLE,
            bg=COLOR_BG,
            fg=COLOR_TEXT,
            anchor="w",
        ).grid(
            row=0,
            column=1,
            sticky="ew",
        )

        # Version
        tk.Button(
            title_frame,
            text=f"v{APP_VERSION}",
            font=FONT_SMALL,
            bg=COLOR_SURFACE_ALT,
            fg=COLOR_TEXT_DIM,
            activebackground=COLOR_BORDER,
            activeforeground=COLOR_TEXT,
            relief="flat",
            bd=0,
            padx=8,
            pady=4,
            command=self._show_version_info,
            cursor="hand2",
        ).grid(
            row=0,
            column=2,
            sticky="e",
        )

        # Separator
        tk.Frame(
            parent,
            height=1,
            bg=COLOR_BORDER,
        ).grid(
            row=1,
            column=0,
            sticky="ew",
            pady=(14, 0),
        )

    # ═════════════════════════════════════════════════════
    # URL Section
    # ═════════════════════════════════════════════════════

    def _build_url_section(
        self,
        parent: tk.Frame,
    ) -> None:

        url_frame = tk.Frame(
            parent,
            bg=COLOR_BG,
        )

        url_frame.grid(
            row=2,
            column=0,
            sticky="ew",
            pady=(18, 0),
        )

        url_frame.grid_columnconfigure(
            0,
            weight=1,
        )

        # Label
        tk.Label(
            url_frame,
            text=t("prompt_url"),
            font=FONT_SMALL,
            bg=COLOR_BG,
            fg=COLOR_TEXT_DIM,
            anchor="w",
        ).grid(
            row=0,
            column=0,
            sticky="ew",
            pady=(0, 6),
        )

        # Border
        self._entry_outer = tk.Frame(
            url_frame,
            bg=COLOR_BORDER,
            padx=1,
            pady=1,
        )

        self._entry_outer.grid(
            row=1,
            column=0,
            sticky="ew",
        )

        self._entry_outer.grid_columnconfigure(
            0,
            weight=1,
        )

        # Inner
        entry_inner = tk.Frame(
            self._entry_outer,
            bg=COLOR_SURFACE,
        )

        entry_inner.grid(
            row=0,
            column=0,
            sticky="ew",
        )

        entry_inner.grid_columnconfigure(
            0,
            weight=1,
        )

        # Entry
        self.entry = tk.Entry(
            entry_inner,
            font=FONT_UI,
            bg=COLOR_SURFACE,
            fg=COLOR_TEXT,
            insertbackground=COLOR_TEXT,
            relief="flat",
            bd=8,
        )

        self.entry.grid(
            row=0,
            column=0,
            sticky="ew",
        )

        self.entry.bind(
            "<Return>",
            lambda _: self._on_download(),
        )

        self.entry.bind(
            "<FocusIn>",
            lambda _: self._entry_outer.config(
                bg=COLOR_ACCENT
            ),
        )

        self.entry.bind(
            "<FocusOut>",
            lambda _: self._entry_outer.config(
                bg=COLOR_BORDER
            ),
        )

        self._bind_entry_context_menu(
            self.entry
        )

    # ═════════════════════════════════════════════════════
    # Button Section
    # ═════════════════════════════════════════════════════

    def _build_button_section(
        self,
        parent: tk.Frame,
    ) -> None:

        btn_frame = tk.Frame(
            parent,
            bg=COLOR_BG,
        )

        btn_frame.grid(
            row=3,
            column=0,
            sticky="ew",
            pady=(14, 0),
        )

        # Download button
        self.btn_download = self._make_button(
            btn_frame,
            text=t("btn_download"),
            command=self._on_download,
            primary=True,
        )

        self.btn_download.pack(
            side="left"
        )

        # Cancel button
        self.btn_cancel = self._make_button(
            btn_frame,
            text=t("btn_cancel"),
            command=self._on_cancel,
            primary=False,
        )

        self.btn_cancel.pack(
            side="left",
            padx=(10, 0),
        )

        self.btn_cancel.config(
            state="disabled"
        )

    # ═════════════════════════════════════════════════════
    # Video Section
    # ═════════════════════════════════════════════════════

    def _build_video_section(
        self,
        parent: tk.Frame,
    ) -> None:

        video_frame = tk.Frame(
            parent,
            bg=COLOR_BG,
        )

        video_frame.grid(
            row=4,
            column=0,
            sticky="ew",
            pady=(18, 0),
        )

        video_frame.grid_columnconfigure(
            0,
            weight=1,
        )

        # ────────────────────────────────────────────────
        # Video card
        # ────────────────────────────────────────────────

        video_card = tk.Frame(
            video_frame,
            bg=COLOR_SURFACE,
        )

        video_card.grid(
            row=0,
            column=0,
            sticky="ew",
        )

        video_card.grid_columnconfigure(
            0,
            weight=1,
        )

        inner = tk.Frame(
            video_card,
            bg=COLOR_SURFACE,
        )

        inner.grid(
            row=0,
            column=0,
            sticky="ew",
            padx=14,
            pady=10,
        )

        inner.grid_columnconfigure(
            0,
            weight=1,
        )

        # Label
        tk.Label(
            inner,
            text=t("video"),
            font=FONT_SMALL,
            bg=COLOR_SURFACE,
            fg=COLOR_TEXT_DIM,
            anchor="w",
        ).grid(
            row=0,
            column=0,
            sticky="ew",
        )

        # Video title
        self.title_var = tk.StringVar(
            value="—"
        )

        self.video_title_label = tk.Label(
            inner,
            textvariable=self.title_var,
            font=FONT_UI,
            bg=COLOR_SURFACE,
            fg=COLOR_TEXT,
            anchor="w",
            justify="left",
        )

        self.video_title_label.grid(
            row=1,
            column=0,
            sticky="ew",
            pady=(2, 0),
        )

        # ────────────────────────────────────────────────
        # Progress
        # ────────────────────────────────────────────────

        progress_frame = tk.Frame(
            video_frame,
            bg=COLOR_BG,
        )

        progress_frame.grid(
            row=1,
            column=0,
            sticky="ew",
            pady=(16, 0),
        )

        progress_frame.grid_columnconfigure(
            0,
            weight=1,
        )

        self.progress_var = (
            tk.DoubleVar()
        )

        self.progress_bar = ttk.Progressbar(
            progress_frame,
            variable=self.progress_var,
            maximum=100,
            style="Custom.Horizontal.TProgressbar",
        )

        self.progress_bar.grid(
            row=0,
            column=0,
            sticky="ew",
        )

        self.pct_var = tk.StringVar(
            value=""
        )

        tk.Label(
            progress_frame,
            textvariable=self.pct_var,
            font=FONT_SMALL,
            bg=COLOR_BG,
            fg=COLOR_TEXT_DIM,
            anchor="e",
        ).grid(
            row=1,
            column=0,
            sticky="ew",
            pady=(3, 0),
        )

    # ═════════════════════════════════════════════════════
    # Status
    # ═════════════════════════════════════════════════════

    def _build_status_section(
        self,
        parent: tk.Frame,
    ) -> None:

        status_frame = tk.Frame(
            parent,
            bg=COLOR_BG,
        )

        status_frame.grid(
            row=5,
            column=0,
            sticky="ew",
            pady=(8, 0),
        )

        status_frame.grid_columnconfigure(
            1,
            weight=1,
        )

        self.status_dot = tk.Label(
            status_frame,
            text="●",
            font=("Segoe UI", 8),
            bg=COLOR_BG,
            fg=COLOR_TEXT_DIM,
        )

        self.status_dot.grid(
            row=0,
            column=0,
            padx=(0, 6),
            sticky="w",
        )

        self.status_label = tk.Label(
            status_frame,
            text=t("ready"),
            font=FONT_SMALL,
            bg=COLOR_BG,
            fg=COLOR_TEXT_DIM,
            anchor="w",
        )

        self.status_label.grid(
            row=0,
            column=1,
            sticky="ew",
        )

    # ═════════════════════════════════════════════════════
    # Button Factory
    # ═════════════════════════════════════════════════════

    def _make_button(
        self,
        parent: tk.Frame,
        text: str,
        command,
        primary: bool,
    ) -> tk.Button:

        if primary:
            bg = COLOR_ACCENT
            fg = "#ffffff"
            active_bg = COLOR_ACCENT_DIM

        else:
            bg = COLOR_SURFACE_ALT
            fg = COLOR_TEXT_DIM
            active_bg = COLOR_BORDER

        return tk.Button(
            parent,
            text=text,
            font=FONT_UI,
            bg=bg,
            fg=fg,
            activebackground=active_bg,
            activeforeground=fg,
            disabledforeground=COLOR_TEXT_DIM,
            relief="flat",
            bd=0,
            padx=18,
            pady=8,
            cursor="hand2",
            command=command,
        )

    # ═════════════════════════════════════════════════════
    # Responsive Resize
    # ═════════════════════════════════════════════════════

    def _on_window_configure(
        self,
        event: tk.Event,
    ) -> None:
        """
        視窗 Resize 時：

          - 動態調整影片名稱 wraplength
          - 不修改字體大小
          - 不使用 bitmap scale
        """

        if event.widget is not self.root:
            return

        width = event.width

        if width <= 0:
            return

        if width == self._last_window_width:
            return

        self._last_window_width = width

        # 實際內容寬度約為：
        # Window Width - 左右 margin
        #
        # 留一些空間給 card padding。

        wrap_width = max(
            240,
            width - 80,
        )

        try:
            self.video_title_label.config(
                wraplength=wrap_width
            )
        except tk.TclError:
            pass

    # ═════════════════════════════════════════════════════
    # Version
    # ═════════════════════════════════════════════════════

    def _show_version_info(
        self,
    ) -> None:

        messagebox.showinfo(
            t("version_info"),
            (
                f"{t('version_title')} "
                f"v{APP_VERSION}\n\n"
                f"{t('yt_dlp')} : "
                f"{YTDLP_VERSION}\n"
                f"{t('ffmpeg')} : "
                f"{FFMPEG_VERSION}\n"
                f"{t('deno')}   : "
                f"{DENO_VERSION}"
            ),
        )

    # ═════════════════════════════════════════════════════
    # Context Menu
    # ═════════════════════════════════════════════════════

    def _bind_entry_context_menu(
        self,
        entry: tk.Entry,
    ) -> None:

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

        # ────────────────────────────────────────────────
        # Cut
        # ────────────────────────────────────────────────

        def do_cut():
            if entry["state"] == "disabled":
                return

            try:
                entry.event_generate(
                    "<<Cut>>"
                )
            except tk.TclError:
                pass

        # ────────────────────────────────────────────────
        # Copy
        # ────────────────────────────────────────────────

        def do_copy():

            try:
                entry.event_generate(
                    "<<Copy>>"
                )
            except tk.TclError:
                pass

        # ────────────────────────────────────────────────
        # Paste
        # ────────────────────────────────────────────────

        def do_paste():

            if entry["state"] == "disabled":
                return

            try:
                try:
                    entry.delete(
                        tk.SEL_FIRST,
                        tk.SEL_LAST,
                    )
                except tk.TclError:
                    pass

                entry.insert(
                    tk.INSERT,
                    self.root.clipboard_get(),
                )

            except tk.TclError:
                pass

        # ────────────────────────────────────────────────
        # Select All
        # ────────────────────────────────────────────────

        def do_select_all():

            entry.select_range(
                0,
                tk.END,
            )

            entry.icursor(
                tk.END
            )

        # ────────────────────────────────────────────────
        # Clear
        # ────────────────────────────────────────────────

        def do_clear():

            if entry["state"] == "disabled":
                return

            entry.delete(
                0,
                tk.END,
            )

        # ────────────────────────────────────────────────
        # Menu items
        # ────────────────────────────────────────────────

        menu.add_command(
            label=t("ctx_cut"),
            command=do_cut,
        )

        menu.add_command(
            label=t("ctx_copy"),
            command=do_copy,
        )

        menu.add_command(
            label=t("ctx_paste"),
            command=do_paste,
        )

        menu.add_separator()

        menu.add_command(
            label=t("ctx_select_all"),
            command=do_select_all,
        )

        menu.add_command(
            label=t("ctx_clear"),
            command=do_clear,
        )

        # ────────────────────────────────────────────────
        # Popup
        # ────────────────────────────────────────────────

        def show_menu(
            event: tk.Event,
        ) -> None:

            editable = (
                entry["state"]
                != "disabled"
            )

            write_state = (
                "normal"
                if editable
                else "disabled"
            )

            menu.entryconfig(
                t("ctx_cut"),
                state=write_state,
            )

            menu.entryconfig(
                t("ctx_paste"),
                state=write_state,
            )

            menu.entryconfig(
                t("ctx_clear"),
                state=write_state,
            )

            has_selection = (
                entry.selection_present()
            )

            if not has_selection:

                menu.entryconfig(
                    t("ctx_cut"),
                    state="disabled",
                )

                menu.entryconfig(
                    t("ctx_copy"),
                    state="disabled",
                )

            else:

                menu.entryconfig(
                    t("ctx_copy"),
                    state="normal",
                )

            menu.tk_popup(
                event.x_root,
                event.y_root,
            )

        entry.bind(
            "<Button-3>",
            show_menu,
        )

        if sys.platform == "darwin":
            entry.bind(
                "<Button-2>",
                show_menu,
            )

    # ═════════════════════════════════════════════════════
    # Status
    # ═════════════════════════════════════════════════════

    def _set_status(
        self,
        text: str,
        dot_color: str = COLOR_TEXT_DIM,
    ) -> None:

        self.status_label.config(
            text=text
        )

        self.status_dot.config(
            fg=dot_color
        )

    def _set_title_display(
        self,
        title: str,
    ) -> None:

        self.title_var.set(
            title if title else "—"
        )

    def _set_downloading(
        self,
        downloading: bool,
    ) -> None:

        entry_state = (
            "disabled"
            if downloading
            else "normal"
        )

        download_state = (
            "disabled"
            if downloading
            else "normal"
        )

        cancel_state = (
            "normal"
            if downloading
            else "disabled"
        )

        self.entry.config(
            state=entry_state
        )

        self.btn_download.config(
            state=download_state
        )

        self.btn_cancel.config(
            state=cancel_state
        )

        if not downloading:

            self.progress_var.set(
                0
            )

            self.pct_var.set(
                ""
            )

    # ═════════════════════════════════════════════════════
    # Download
    # ═════════════════════════════════════════════════════

    def _on_download(self) -> None:

        url = (
            self.entry
            .get()
            .strip()
        )

        if not url:

            messagebox.showwarning(
                "錯誤",
                t("empty_url"),
            )

            return

        yt_dlp = get_tool_path(
            "yt-dlp"
        )

        ffmpeg = get_tool_path(
            "ffmpeg"
        )

        if not yt_dlp:

            messagebox.showerror(
                "錯誤",
                t("yt_dlp_missing"),
            )

            return

        if not ffmpeg:

            messagebox.showerror(
                "錯誤",
                t("ffmpeg_missing"),
            )

            return

        # deno optional
        deno = get_tool_path(
            "deno"
        )

        if not deno:

            messagebox.showwarning(
                "提示",
                t("deno_missing"),
            )

        # ────────────────────────────────────────────────
        # Reset state
        # ────────────────────────────────────────────────

        self._cancelled = False

        self._set_downloading(
            True
        )

        self._set_title_display(
            ""
        )

        self._set_status(
            t("fetching_formats"),
            COLOR_WARNING,
        )

        output_dir = (
            Path(sys.argv[0])
            .resolve()
            .parent
        )

        # ────────────────────────────────────────────────
        # yt-dlp arguments
        # ────────────────────────────────────────────────

        args = [
            str(yt_dlp),

            "-o",
            str(
                output_dir
                / "%(title)s.%(ext)s"
            ),

            "-f",
            (
                "bv*[ext=mp4]+ba[ext=m4a]"
                "/bv*+ba/b"
            ),

            url,

            "--no-playlist",

            "--user-agent",
            (
                "Mozilla/5.0 "
                "(Windows NT 10.0; Win64; x64) "
                "AppleWebKit/537.36 "
                "(KHTML, like Gecko) "
                "Chrome/122.0.0.0 "
                "Safari/537.36"
            ),

            "--referer",
            url,

            "--ffmpeg-location",
            str(ffmpeg.parent),

            "--newline",
        ]

        threading.Thread(
            target=self._run_download,
            args=(
                args,
                output_dir,
            ),
            daemon=True,
        ).start()

    # ═════════════════════════════════════════════════════
    # Cancel
    # ═════════════════════════════════════════════════════

    def _on_cancel(self) -> None:

        self._cancelled = True

        if (
            self._process
            and self._process.poll()
            is None
        ):

            try:
                self._process.terminate()
            except Exception:
                pass

        self._set_status(
            t("cancelled"),
            COLOR_WARNING,
        )

        self._set_downloading(
            False
        )

    # ═════════════════════════════════════════════════════
    # Download Worker
    # ═════════════════════════════════════════════════════

    def _run_download(
        self,
        args: list[str],
        output_dir: Path,
    ) -> None:

        log_lines: list[str] = []

        # Windows：
        # 不顯示 console 視窗。
        creationflags = (
            0x08000000
            if sys.platform.startswith("win")
            else 0
        )

        sys_encoding = (
            locale.getpreferredencoding(
                False
            )
        )

        child_env = os.environ.copy()

        # 避免 subprocess inherited UTF-8
        # 導致 Windows 編碼錯誤。
        child_env.pop(
            "PYTHONUTF8",
            None,
        )

        child_env.pop(
            "PYTHONIOENCODING",
            None,
        )

        try:

            self._process = subprocess.Popen(
                args,

                stdout=subprocess.PIPE,

                stderr=subprocess.STDOUT,

                text=True,

                encoding=sys_encoding,

                errors="replace",

                cwd=str(
                    output_dir
                ),

                env=child_env,

                creationflags=creationflags,
            )

        except Exception as exc:

            self.root.after(
                0,
                self._on_failure,
                str(exc),
            )

            return

        assert (
            self._process.stdout
            is not None
        )

        # ────────────────────────────────────────────────
        # Read stdout
        # ────────────────────────────────────────────────

        for line in self._process.stdout:

            if self._cancelled:
                break

            log_lines.append(
                line
            )

            if (
                len(log_lines)
                > self.MAX_LOG_LINES
            ):
                log_lines.pop(0)

            # ────────────────────────────────────────
            # title
            # ────────────────────────────────────────

            title = parse_title(
                line
            )

            if (
                title
                and self.title_var.get()
                == "—"
            ):

                self.root.after(
                    0,
                    self._set_title_display,
                    title,
                )

            # ────────────────────────────────────────
            # progress
            # ────────────────────────────────────────

            pct = parse_progress(
                line
            )

            if pct is not None:

                self.root.after(
                    0,
                    self.progress_var.set,
                    pct,
                )

                self.root.after(
                    0,
                    self.pct_var.set,
                    f"{pct:.1f}%",
                )

                self.root.after(
                    0,
                    self._set_status,
                    t("downloading"),
                    COLOR_ACCENT,
                )

        # 等待 process 完整結束
        if self._process.poll() is None:

            try:
                self._process.wait()
            except Exception:
                pass

        returncode = (
            self._process.returncode
        )

        if self._cancelled:
            return

        if returncode == 0:

            self.root.after(
                0,
                self._on_success,
                output_dir,
            )

        else:

            error_text = (
                "".join(log_lines)
                .strip()
            )

            if not error_text:
                error_text = (
                    t("download_failed")
                )

            self.root.after(
                0,
                self._on_failure,
                error_text,
            )

    # ═════════════════════════════════════════════════════
    # Success
    # ═════════════════════════════════════════════════════

    def _on_success(
        self,
        output_dir: Path,
    ) -> None:

        self.progress_var.set(
            100
        )

        self.pct_var.set(
            "100%"
        )

        self._set_status(
            t("download_success"),
            COLOR_SUCCESS,
        )

        self._set_downloading(
            False
        )

        open_directory(
            output_dir
        )

    # ═════════════════════════════════════════════════════
    # Failure
    # ═════════════════════════════════════════════════════

    def _on_failure(
        self,
        err: str,
    ) -> None:

        self._set_status(
            t("download_failed"),
            COLOR_DANGER,
        )

        self._set_downloading(
            False
        )

        messagebox.showerror(
            "yt-dlp 錯誤",
            err,
        )

    # ═════════════════════════════════════════════════════
    # Close
    # ═════════════════════════════════════════════════════

    def _on_close(self) -> None:

        # 視窗關閉時，如果還在下載，
        # 先終止 yt-dlp。

        if (
            self._process
            and self._process.poll()
            is None
        ):

            try:
                self._process.terminate()
            except Exception:
                pass

        self._cancelled = True

        self.root.destroy()


# ═════════════════════════════════════════════════════════════
# Entry Point
# ═════════════════════════════════════════════════════════════


def main() -> None:
    root = tk.Tk()

    VideoDownloaderApp(
        root
    )

    root.mainloop()


if __name__ == "__main__":
    main()
