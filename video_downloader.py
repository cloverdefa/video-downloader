#!/usr/bin/env -S uv run
# -*- coding: utf-8 -*-

"""
─────────────────────────────────────────────────────────────
簡易 GUI 影片下載器 (yt-dlp + ffmpeg)

外觀重構重點（v2 現代化風格）：
  - 全新配色：深色系 + 單一強調色，對比更柔和
  - 卡片式版面，使用 Canvas 繪製圓角矩形取代方形 Frame
  - 自訂圓角按鈕元件（RoundedButton），含 hover / disabled 狀態
  - 自訂圓角進度條（RoundedProgressBar），取代 ttk 預設樣式
  - 網址輸入框改為「圓角容器 + 無邊框 Entry」，focus 時邊框變色
  - 卡片加上細微陰影（雙層圓角矩形模擬 elevation）
  - 版本徽章、狀態列改為圓角膠囊樣式
  - 其餘下載邏輯 / 執行緒 / 事件流程與前版完全相同，僅重繪 UI 層
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


DEFAULT_WIDTH = 560
DEFAULT_HEIGHT = 460

MIN_WIDTH = 460
MIN_HEIGHT = 420


# ═════════════════════════════════════════════════════════════
# 色彩（現代深色主題）
# ═════════════════════════════════════════════════════════════


COLOR_BG = "#0b0b10"
COLOR_SURFACE = "#16161d"
COLOR_SURFACE_ALT = "#1e1e27"
COLOR_SHADOW = "#000000"

COLOR_ACCENT = "#7c6aff"
COLOR_ACCENT_HOVER = "#9384ff"
COLOR_ACCENT_DIM = "#5a4ed6"
COLOR_ACCENT_SOFT = "#241f3d"

COLOR_SUCCESS = "#34d399"
COLOR_WARNING = "#fbbf24"
COLOR_DANGER = "#f87171"

COLOR_TEXT = "#f2f2f6"
COLOR_TEXT_DIM = "#93939f"
COLOR_TEXT_MUTED = "#5c5c68"

COLOR_BORDER = "#26262f"
COLOR_BORDER_FOCUS = COLOR_ACCENT


# ═════════════════════════════════════════════════════════════
# 字型
# ═════════════════════════════════════════════════════════════


FONT_UI = (
    "Microsoft JhengHei UI",
    10,
)

FONT_TITLE = (
    "Microsoft JhengHei UI",
    14,
    "bold",
)

FONT_SMALL = (
    "Microsoft JhengHei UI",
    9,
)

FONT_BUTTON = (
    "Microsoft JhengHei UI",
    10,
    "bold",
)

FONT_LABEL = (
    "Microsoft JhengHei UI",
    9,
    "bold",
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
# 共用：圓角繪製工具
# ═════════════════════════════════════════════════════════════


def _rounded_rect_points(
    x1: float,
    y1: float,
    x2: float,
    y2: float,
    radius: float,
) -> list[float]:
    """
    產生圓角矩形的多邊形頂點，搭配 smooth=True
    可畫出視覺上平滑的圓角。
    """

    radius = max(
        0,
        min(
            radius,
            (x2 - x1) / 2,
            (y2 - y1) / 2,
        ),
    )

    return [
        x1 + radius, y1,
        x2 - radius, y1,
        x2, y1,
        x2, y1 + radius,
        x2, y2 - radius,
        x2, y2,
        x2 - radius, y2,
        x1 + radius, y2,
        x1, y2,
        x1, y2 - radius,
        x1, y1 + radius,
        x1, y1,
    ]


def draw_rounded_rect(
    canvas: tk.Canvas,
    x1: float,
    y1: float,
    x2: float,
    y2: float,
    radius: float,
    **kwargs,
) -> int:

    points = _rounded_rect_points(
        x1, y1, x2, y2, radius
    )

    return canvas.create_polygon(
        points,
        smooth=True,
        **kwargs,
    )


# ═════════════════════════════════════════════════════════════
# 元件：圓角卡片（可帶淡陰影）
# ═════════════════════════════════════════════════════════════


class RoundedCard(tk.Canvas):
    """
    以 Canvas 模擬圓角卡片，內容用 create_window 放置一個
    一般 tk.Frame，方便沿用既有的 grid / pack 佈局方式。
    """

    def __init__(
        self,
        parent: tk.Widget,
        *,
        bg_color: str = COLOR_SURFACE,
        border_color: str = COLOR_BORDER,
        radius: int = 14,
        shadow: bool = True,
        **kwargs,
    ) -> None:

        super().__init__(
            parent,
            bg=parent["bg"] if "bg" in parent.keys() else COLOR_BG,
            highlightthickness=0,
            bd=0,
            **kwargs,
        )

        self._bg_color = bg_color
        self._border_color = border_color
        self._radius = radius
        self._shadow = shadow

        self.inner = tk.Frame(
            self,
            bg=bg_color,
        )

        self.bind(
            "<Configure>",
            self._redraw,
        )

    def _redraw(
        self,
        event: tk.Event | None = None,
    ) -> None:

        self.delete("all")

        width = self.winfo_width()
        height = self.winfo_height()

        if width < 4 or height < 4:
            return

        pad = 2

        if self._shadow:
            draw_rounded_rect(
                self,
                pad,
                pad + 3,
                width - pad,
                height - pad + 1,
                self._radius,
                fill=COLOR_SHADOW,
                outline="",
                stipple="gray25",
            )

        draw_rounded_rect(
            self,
            pad,
            pad,
            width - pad,
            height - pad - (3 if self._shadow else 0),
            self._radius,
            fill=self._bg_color,
            outline=self._border_color,
            width=1,
        )

        self.create_window(
            pad + 1,
            pad + 1,
            anchor="nw",
            window=self.inner,
            width=width - 2 * pad - 2,
            height=height - 2 * pad - 2 - (3 if self._shadow else 0),
        )

    def set_border_color(self, color: str) -> None:
        self._border_color = color
        self._redraw()


# ═════════════════════════════════════════════════════════════
# 元件：圓角按鈕
# ═════════════════════════════════════════════════════════════


class RoundedButton(tk.Canvas):

    def __init__(
        self,
        parent: tk.Widget,
        text: str,
        command=None,
        *,
        primary: bool = True,
        width: int = 132,
        height: int = 40,
        radius: int = 10,
        font=FONT_BUTTON,
    ) -> None:

        super().__init__(
            parent,
            width=width,
            height=height,
            bg=parent["bg"] if "bg" in parent.keys() else COLOR_BG,
            highlightthickness=0,
            bd=0,
            cursor="hand2",
        )

        self._command = command
        self._text = text
        self._font = font
        self._radius = radius
        self._primary = primary
        self._state = "normal"
        self._hover = False

        if primary:
            self._bg_normal = COLOR_ACCENT
            self._bg_hover = COLOR_ACCENT_HOVER
            self._bg_active = COLOR_ACCENT_DIM
            self._fg = "#ffffff"
        else:
            self._bg_normal = COLOR_SURFACE_ALT
            self._bg_hover = COLOR_BORDER
            self._bg_active = COLOR_BORDER
            self._fg = COLOR_TEXT_DIM

        self._bg_disabled = COLOR_SURFACE
        self._fg_disabled = COLOR_TEXT_MUTED

        self.bind("<Enter>", self._on_enter)
        self.bind("<Leave>", self._on_leave)
        self.bind("<Button-1>", self._on_press)
        self.bind("<ButtonRelease-1>", self._on_release)
        self.bind("<Configure>", lambda _e: self._draw())

        self._draw()

    # ── 繪製 ──────────────────────────────────────────────

    def _draw(self, active: bool = False) -> None:

        self.delete("all")

        width = self.winfo_width() or int(self["width"])
        height = self.winfo_height() or int(self["height"])

        if self._state == "disabled":
            fill = self._bg_disabled
            fg = self._fg_disabled
        elif active:
            fill = self._bg_active
            fg = self._fg
        elif self._hover:
            fill = self._bg_hover
            fg = self._fg
        else:
            fill = self._bg_normal
            fg = self._fg

        draw_rounded_rect(
            self,
            1, 1,
            width - 1, height - 1,
            self._radius,
            fill=fill,
            outline="",
        )

        self.create_text(
            width / 2,
            height / 2,
            text=self._text,
            fill=fg,
            font=self._font,
        )

    # ── 事件 ──────────────────────────────────────────────

    def _on_enter(self, _e) -> None:
        if self._state == "disabled":
            return
        self._hover = True
        self._draw()

    def _on_leave(self, _e) -> None:
        self._hover = False
        self._draw()

    def _on_press(self, _e) -> None:
        if self._state == "disabled":
            return
        self._draw(active=True)

    def _on_release(self, e) -> None:
        if self._state == "disabled":
            return

        self._draw()

        width = self.winfo_width()
        height = self.winfo_height()

        if 0 <= e.x <= width and 0 <= e.y <= height:
            if self._command:
                self._command()

    # ── 對外 API（模擬 tk.Button.config）───────────────────

    def config(self, state: str | None = None, **_kwargs) -> None:
        if state is not None:
            self._state = state
            self._hover = False
            self._draw()

    configure = config


# ═════════════════════════════════════════════════════════════
# 元件：圓角進度條
# ═════════════════════════════════════════════════════════════


class RoundedProgressBar(tk.Canvas):

    def __init__(
        self,
        parent: tk.Widget,
        *,
        height: int = 10,
        radius: int = 5,
        track_color: str = COLOR_SURFACE_ALT,
        fill_color: str = COLOR_ACCENT,
    ) -> None:

        super().__init__(
            parent,
            height=height,
            bg=parent["bg"] if "bg" in parent.keys() else COLOR_BG,
            highlightthickness=0,
            bd=0,
        )

        self._radius = radius
        self._track_color = track_color
        self._fill_color = fill_color
        self._value = 0.0

        self.bind("<Configure>", lambda _e: self._draw())

    def set(self, value: float) -> None:
        self._value = max(0.0, min(100.0, value))
        self._draw()

    def _draw(self) -> None:

        self.delete("all")

        width = self.winfo_width()
        height = self.winfo_height()

        if width < 4 or height < 4:
            return

        draw_rounded_rect(
            self,
            0, 0, width, height,
            self._radius,
            fill=self._track_color,
            outline="",
        )

        fill_width = width * (self._value / 100.0)

        if fill_width >= 2:
            draw_rounded_rect(
                self,
                0, 0, fill_width, height,
                self._radius,
                fill=self._fill_color,
                outline="",
            )


# ═════════════════════════════════════════════════════════════
# 元件：圓角輸入框容器
# ═════════════════════════════════════════════════════════════


class RoundedEntry(tk.Canvas):
    """
    圓角外框容器 + 內部無邊框 Entry。
    Entry 背景與容器背景一致，因此只有圓角外框可見，
    達成「圓角輸入框」的視覺效果；focus 時外框變成強調色。
    """

    def __init__(
        self,
        parent: tk.Widget,
        *,
        height: int = 46,
        radius: int = 12,
        font=FONT_UI,
    ) -> None:

        super().__init__(
            parent,
            height=height,
            bg=parent["bg"] if "bg" in parent.keys() else COLOR_BG,
            highlightthickness=0,
            bd=0,
        )

        self._radius = radius
        self._border_color = COLOR_BORDER
        self._focused = False

        self.entry = tk.Entry(
            self,
            font=font,
            bg=COLOR_SURFACE,
            fg=COLOR_TEXT,
            insertbackground=COLOR_TEXT,
            relief="flat",
            bd=0,
        )

        self.entry.bind("<FocusIn>", self._on_focus_in)
        self.entry.bind("<FocusOut>", self._on_focus_out)

        self.bind("<Configure>", lambda _e: self._draw())

    def _on_focus_in(self, _e) -> None:
        self._focused = True
        self._border_color = COLOR_ACCENT
        self._draw()

    def _on_focus_out(self, _e) -> None:
        self._focused = False
        self._border_color = COLOR_BORDER
        self._draw()

    def _draw(self) -> None:

        self.delete("all")

        width = self.winfo_width()
        height = self.winfo_height()

        if width < 4 or height < 4:
            return

        border_width = 2 if self._focused else 1

        draw_rounded_rect(
            self,
            1, 1,
            width - 1, height - 1,
            self._radius,
            fill=COLOR_SURFACE,
            outline=self._border_color,
            width=border_width,
        )

        inset_x = 14
        inset_y = 6

        self.create_window(
            inset_x,
            inset_y,
            anchor="nw",
            window=self.entry,
            width=width - inset_x * 2,
            height=height - inset_y * 2,
        )


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
    # UI Build
    # ═════════════════════════════════════════════════════

    def _build_ui(self) -> None:
        """
        Responsive layout：

        root
        └── main
            ├── title bar（icon chip + title + version badge）
            ├── url card（label + rounded entry）
            ├── button row（圓角按鈕）
            ├── video card（影片名稱 + 進度條）
            └── status pill
        """

        main = tk.Frame(
            self.root,
            bg=COLOR_BG,
        )

        main.pack(
            fill=tk.BOTH,
            expand=True,
            padx=22,
            pady=(20, 18),
        )

        main.grid_columnconfigure(
            0,
            weight=1,
        )

        self._build_title(main)
        self._build_url_section(main)
        self._build_button_section(main)
        self._build_video_section(main)
        self._build_status_section(main)

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

        title_frame.grid_columnconfigure(1, weight=1)

        # Icon chip（圓角小徽章）
        icon_chip = tk.Canvas(
            title_frame,
            width=38,
            height=38,
            bg=COLOR_BG,
            highlightthickness=0,
            bd=0,
        )

        icon_chip.grid(
            row=0,
            column=0,
            padx=(0, 12),
        )

        draw_rounded_rect(
            icon_chip,
            1, 1, 37, 37,
            11,
            fill=COLOR_ACCENT_SOFT,
            outline="",
        )

        icon_chip.create_text(
            19, 19,
            text="⬇",
            fill=COLOR_ACCENT,
            font=("Segoe UI Emoji", 16),
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

        # Version badge（圓角膠囊）
        self._version_badge = self._make_pill_button(
            title_frame,
            text=f"v{APP_VERSION}",
            command=self._show_version_info,
        )

        self._version_badge.grid(
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
            pady=(16, 0),
        )

        parent.grid_rowconfigure(1, weight=0)

    def _make_pill_button(
        self,
        parent: tk.Widget,
        text: str,
        command,
    ) -> tk.Canvas:

        pad_x = 12
        approx_width = 16 + len(text) * 7
        height = 28

        canvas = tk.Canvas(
            parent,
            width=approx_width,
            height=height,
            bg=parent["bg"],
            highlightthickness=0,
            bd=0,
            cursor="hand2",
        )

        def render(hover: bool = False) -> None:
            canvas.delete("all")
            w = canvas.winfo_width() or approx_width
            h = canvas.winfo_height() or height
            draw_rounded_rect(
                canvas,
                1, 1, w - 1, h - 1,
                h / 2,
                fill=COLOR_BORDER if hover else COLOR_SURFACE_ALT,
                outline="",
            )
            canvas.create_text(
                w / 2,
                h / 2,
                text=text,
                fill=COLOR_TEXT_DIM,
                font=FONT_SMALL,
            )

        canvas.bind("<Enter>", lambda _e: render(True))
        canvas.bind("<Leave>", lambda _e: render(False))
        canvas.bind("<Button-1>", lambda _e: command())
        canvas.bind("<Configure>", lambda _e: render())

        render()

        return canvas

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
            pady=(20, 0),
        )

        url_frame.grid_columnconfigure(0, weight=1)
        parent.grid_rowconfigure(2, weight=0)

        # Label
        tk.Label(
            url_frame,
            text=t("prompt_url"),
            font=FONT_LABEL,
            bg=COLOR_BG,
            fg=COLOR_TEXT_DIM,
            anchor="w",
        ).grid(
            row=0,
            column=0,
            sticky="ew",
            pady=(0, 8),
        )

        # Rounded entry
        self._rounded_entry = RoundedEntry(
            url_frame,
            height=46,
            radius=12,
            font=FONT_UI,
        )

        self._rounded_entry.grid(
            row=1,
            column=0,
            sticky="ew",
        )

        self.entry = self._rounded_entry.entry

        self.entry.bind(
            "<Return>",
            lambda _: self._on_download(),
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
            pady=(16, 0),
        )

        parent.grid_rowconfigure(3, weight=0)

        self.btn_download = RoundedButton(
            btn_frame,
            text=t("btn_download"),
            command=self._on_download,
            primary=True,
            width=136,
            height=42,
            radius=11,
        )

        self.btn_download.pack(side="left")

        self.btn_cancel = RoundedButton(
            btn_frame,
            text=t("btn_cancel"),
            command=self._on_cancel,
            primary=False,
            width=104,
            height=42,
            radius=11,
        )

        self.btn_cancel.pack(
            side="left",
            padx=(10, 0),
        )

        self.btn_cancel.config(state="disabled")

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
            sticky="nsew",
            pady=(20, 0),
        )

        video_frame.grid_columnconfigure(0, weight=1)
        parent.grid_rowconfigure(4, weight=1)

        # ────────────────────────────────────────────────
        # Video card（圓角卡片）
        # ────────────────────────────────────────────────

        self._video_card = RoundedCard(
            video_frame,
            bg_color=COLOR_SURFACE,
            border_color=COLOR_BORDER,
            radius=14,
            shadow=True,
            height=90,
        )

        self._video_card.grid(
            row=0,
            column=0,
            sticky="ew",
        )

        inner = self._video_card.inner
        inner.grid_columnconfigure(0, weight=1)

        pad = tk.Frame(inner, bg=COLOR_SURFACE)
        pad.pack(fill="both", expand=True, padx=16, pady=14)
        pad.grid_columnconfigure(0, weight=1)

        tk.Label(
            pad,
            text=t("video"),
            font=FONT_LABEL,
            bg=COLOR_SURFACE,
            fg=COLOR_TEXT_MUTED,
            anchor="w",
        ).grid(row=0, column=0, sticky="ew")

        self.title_var = tk.StringVar(value="—")

        self.video_title_label = tk.Label(
            pad,
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
            pady=(4, 0),
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
            pady=(18, 0),
        )

        progress_frame.grid_columnconfigure(0, weight=1)

        self.progress_bar = RoundedProgressBar(
            progress_frame,
            height=10,
            radius=5,
            track_color=COLOR_SURFACE_ALT,
            fill_color=COLOR_ACCENT,
        )

        self.progress_bar.grid(
            row=0,
            column=0,
            sticky="ew",
        )

        self.pct_var = tk.StringVar(value="")

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
            pady=(5, 0),
        )

    # 相容於舊有呼叫方式的小 helper
    def _progress_set(self, value: float) -> None:
        self.progress_bar.set(value)

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
            pady=(14, 0),
        )

        parent.grid_rowconfigure(5, weight=0)

        status_frame.grid_columnconfigure(1, weight=1)

        self.status_dot_canvas = tk.Canvas(
            status_frame,
            width=10,
            height=10,
            bg=COLOR_BG,
            highlightthickness=0,
            bd=0,
        )

        self.status_dot_canvas.grid(
            row=0,
            column=0,
            padx=(2, 8),
        )

        self._status_dot_item = self.status_dot_canvas.create_oval(
            1, 1, 9, 9,
            fill=COLOR_TEXT_DIM,
            outline="",
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

        wrap_width = max(
            240,
            width - 110,
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

        self.status_dot_canvas.itemconfig(
            self._status_dot_item,
            fill=dot_color,
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

            self.progress_bar.set(0)

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
                    self._progress_set,
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

        self.progress_bar.set(100)

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
