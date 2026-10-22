#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///

"""
─────────────────────────────────────────────────────────────
簡易 GUI 影片下載器 (yt-dlp + ffmpeg)

v3 重構重點：
  - 語法現代化：walrus、match、deque、contextlib.suppress、
    Final / Callable 型別標註、PEP 723 inline metadata
  - 執行緒安全：背景執行緒只透過 queue.Queue 傳遞事件，
    所有 Tk 操作都在主執行緒處理（root.after 輪詢）
  - 每次下載使用獨立的 job id 與取消旗標，避免舊事件污染新任務
  - 安全性：URL 以 "--" 與選項分隔，避免被當成 yt-dlp 參數
  - 路徑：frozen 時以 sys.executable 為準；%OneDrive% 未設定時略過
  - 字型依平台選擇；右鍵選單改用 index 而非顯示文字
  - UI 外觀與下載流程與 v2 相同
─────────────────────────────────────────────────────────────
"""

from __future__ import annotations

import ctypes
import locale
import os
import queue
import re
import shutil
import subprocess
import sys
import threading
import tkinter as tk
from collections import deque
from collections.abc import Callable
from contextlib import suppress
from pathlib import Path
from tkinter import messagebox
from typing import Any, Final, NamedTuple

IS_WIN: Final = sys.platform == "win32"
IS_MAC: Final = sys.platform == "darwin"


# ═════════════════════════════════════════════════════════════
# Windows DPI
# ═════════════════════════════════════════════════════════════


def _enable_windows_dpi_awareness() -> None:
    """
    啟用 Windows Per-Monitor DPI Awareness V2。

    目的：
      - 避免 125% / 150% / 175% / 200% DPI 下 GUI 模糊
      - 避免 Windows 對 PyInstaller EXE 進行額外 Virtual Scaling

    注意：
      不額外呼叫 tk scaling，避免 DPI 二次放大。
    """

    if not IS_WIN:
        return

    # Windows 10 1703+
    try:
        user32 = ctypes.windll.user32

        user32.SetProcessDpiAwarenessContext.argtypes = [ctypes.c_void_p]
        user32.SetProcessDpiAwarenessContext.restype = ctypes.c_bool

        # DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4
        if user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4)):
            return
    except (AttributeError, OSError):
        pass

    # Windows 8.1 fallback
    try:
        shcore = ctypes.windll.shcore

        shcore.SetProcessDpiAwareness.argtypes = [ctypes.c_int]
        shcore.SetProcessDpiAwareness.restype = ctypes.c_int

        # PROCESS_PER_MONITOR_DPI_AWARE = 2
        shcore.SetProcessDpiAwareness(2)
    except (AttributeError, OSError):
        pass


_enable_windows_dpi_awareness()


# ═════════════════════════════════════════════════════════════
# 國際化
# ═════════════════════════════════════════════════════════════


LANGS: Final[dict[str, dict[str, str]]] = {
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


DEFAULT_WIDTH: Final = 560
DEFAULT_HEIGHT: Final = 460

MIN_WIDTH: Final = 460
MIN_HEIGHT: Final = 420

POLL_INTERVAL_MS: Final = 50


# ═════════════════════════════════════════════════════════════
# 色彩（現代深色主題）
# ═════════════════════════════════════════════════════════════


COLOR_BG: Final = "#0b0b10"
COLOR_SURFACE: Final = "#16161d"
COLOR_SURFACE_ALT: Final = "#1e1e27"
COLOR_SHADOW: Final = "#000000"

COLOR_ACCENT: Final = "#7c6aff"
COLOR_ACCENT_HOVER: Final = "#9384ff"
COLOR_ACCENT_DIM: Final = "#5a4ed6"
COLOR_ACCENT_SOFT: Final = "#241f3d"

COLOR_SUCCESS: Final = "#34d399"
COLOR_WARNING: Final = "#fbbf24"
COLOR_DANGER: Final = "#f87171"

COLOR_TEXT: Final = "#f2f2f6"
COLOR_TEXT_DIM: Final = "#93939f"
COLOR_TEXT_MUTED: Final = "#5c5c68"

COLOR_BORDER: Final = "#26262f"


# ═════════════════════════════════════════════════════════════
# 字型（依平台選擇）
# ═════════════════════════════════════════════════════════════


match sys.platform:
    case "win32":
        _FONT_FAMILY = "Microsoft JhengHei UI"
        _EMOJI_FONT = "Segoe UI Emoji"
    case "darwin":
        _FONT_FAMILY = "PingFang TC"
        _EMOJI_FONT = "Apple Color Emoji"
    case _:
        _FONT_FAMILY = "Noto Sans CJK TC"
        _EMOJI_FONT = "Noto Color Emoji"

FONT_UI: Final = (_FONT_FAMILY, 10)
FONT_TITLE: Final = (_FONT_FAMILY, 14, "bold")
FONT_SMALL: Final = (_FONT_FAMILY, 9)
FONT_BUTTON: Final = (_FONT_FAMILY, 10, "bold")
FONT_LABEL: Final = (_FONT_FAMILY, 9, "bold")
FONT_ICON: Final = (_EMOJI_FONT, 16)


# ═════════════════════════════════════════════════════════════
# 版本資訊
# ═════════════════════════════════════════════════════════════


APP_VERSION: Final = getattr(sys, "_app_version", "dev")
YTDLP_VERSION: Final = getattr(sys, "_ytdlp_version", "unknown")
FFMPEG_VERSION: Final = getattr(sys, "_ffmpeg_version", "unknown")
DENO_VERSION: Final = getattr(sys, "_deno_version", "unknown")


# ═════════════════════════════════════════════════════════════
# 路徑與外部工具
# ═════════════════════════════════════════════════════════════


def get_app_dir() -> Path:
    """程式所在目錄：PyInstaller 打包後用 sys.executable，否則用腳本路徑。"""

    if getattr(sys, "frozen", False):
        return Path(sys.executable).resolve().parent

    return Path(sys.argv[0]).resolve().parent


def get_tool_path(name: str) -> Path | None:
    """
    工具搜尋順序：

      1. 程式所在目錄
      2. %OneDrive%\\bin（僅 Windows，且環境變數存在時）
      3. PyInstaller _MEIPASS
      4. 系統 PATH（shutil.which 在 Windows 會自動套用 PATHEXT）
    """

    exe = f"{name}.exe" if IS_WIN else name

    candidates: list[Path] = [get_app_dir() / exe]

    if IS_WIN and (one_drive := os.environ.get("ONEDRIVE")):
        candidates.append(Path(one_drive) / "bin" / exe)

    if meipass := getattr(sys, "_MEIPASS", None):
        candidates.append(Path(meipass) / exe)

    for path in candidates:
        if path.is_file():
            return path

    if found := shutil.which(name):
        return Path(found)

    return None


def open_directory(path: Path) -> None:
    with suppress(Exception):
        match sys.platform:
            case "win32":
                os.startfile(path)  # type: ignore[attr-defined]
            case "darwin":
                subprocess.Popen(["open", str(path)])
            case _:
                subprocess.Popen(["xdg-open", str(path)])


# ═════════════════════════════════════════════════════════════
# yt-dlp 輸出解析
# ═════════════════════════════════════════════════════════════


_PROGRESS_RE: Final = re.compile(r"(\d{1,3}(?:\.\d+)?)%")
_DEST_RE: Final = re.compile(r"\[download\]\s+Destination:\s+(.+)")
_DEST_BARE_RE: Final = re.compile(r"^Destination:\s+(.+)")


def parse_progress(line: str) -> float | None:
    if m := _PROGRESS_RE.search(line):
        return float(m[1])

    return None


def parse_title(line: str) -> str | None:
    """從 yt-dlp 輸出中解析目標檔案名稱。"""

    if m := _DEST_RE.search(line) or _DEST_BARE_RE.search(line):
        return Path(m[1].strip()).stem

    return None


# ═════════════════════════════════════════════════════════════
# 共用：圓角繪製工具
# ═════════════════════════════════════════════════════════════


def _parent_bg(widget: tk.Misc) -> str:
    """取得父元件背景色，取不到時退回主背景色。"""

    try:
        return widget.cget("bg")
    except tk.TclError:
        return COLOR_BG


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

    r = max(0.0, min(radius, (x2 - x1) / 2, (y2 - y1) / 2))

    points = [
        (x1 + r, y1),
        (x2 - r, y1),
        (x2, y1),
        (x2, y1 + r),
        (x2, y2 - r),
        (x2, y2),
        (x2 - r, y2),
        (x1 + r, y2),
        (x1, y2),
        (x1, y2 - r),
        (x1, y1 + r),
        (x1, y1),
    ]

    return [coord for point in points for coord in point]


def draw_rounded_rect(
    canvas: tk.Canvas,
    x1: float,
    y1: float,
    x2: float,
    y2: float,
    radius: float,
    **kwargs: Any,
) -> int:
    return canvas.create_polygon(
        _rounded_rect_points(x1, y1, x2, y2, radius),
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

    _SHADOW_OFFSET: Final = 3

    def __init__(
        self,
        parent: tk.Misc,
        *,
        bg_color: str = COLOR_SURFACE,
        border_color: str = COLOR_BORDER,
        radius: int = 14,
        shadow: bool = True,
        **kwargs: Any,
    ) -> None:
        super().__init__(
            parent,
            bg=_parent_bg(parent),
            highlightthickness=0,
            bd=0,
            **kwargs,
        )

        self._bg_color = bg_color
        self._border_color = border_color
        self._radius = radius
        self._shadow = shadow

        self.inner = tk.Frame(self, bg=bg_color)

        self.bind("<Configure>", self._redraw)

    def _redraw(self, _event: tk.Event | None = None) -> None:
        self.delete("all")

        width = self.winfo_width()
        height = self.winfo_height()

        if width < 4 or height < 4:
            return

        pad = 2
        drop = self._SHADOW_OFFSET if self._shadow else 0

        if self._shadow:
            draw_rounded_rect(
                self,
                pad,
                pad + drop,
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
            height - pad - drop,
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
            height=height - 2 * pad - 2 - drop,
        )


# ═════════════════════════════════════════════════════════════
# 元件：圓角按鈕
# ═════════════════════════════════════════════════════════════


class RoundedButton(tk.Canvas):
    def __init__(
        self,
        parent: tk.Misc,
        text: str,
        command: Callable[[], None] | None = None,
        *,
        primary: bool = True,
        width: int = 132,
        height: int = 40,
        radius: int = 10,
        font: tuple[Any, ...] = FONT_BUTTON,
    ) -> None:
        super().__init__(
            parent,
            width=width,
            height=height,
            bg=_parent_bg(parent),
            highlightthickness=0,
            bd=0,
            cursor="hand2",
        )

        self._command = command
        self._text = text
        self._font = font
        self._radius = radius
        self._enabled = True
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

        if not self._enabled:
            fill, fg = self._bg_disabled, self._fg_disabled
        elif active:
            fill, fg = self._bg_active, self._fg
        elif self._hover:
            fill, fg = self._bg_hover, self._fg
        else:
            fill, fg = self._bg_normal, self._fg

        draw_rounded_rect(
            self,
            1,
            1,
            width - 1,
            height - 1,
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

    def _on_enter(self, _e: tk.Event) -> None:
        if self._enabled:
            self._hover = True
            self._draw()

    def _on_leave(self, _e: tk.Event) -> None:
        self._hover = False
        self._draw()

    def _on_press(self, _e: tk.Event) -> None:
        if self._enabled:
            self._draw(active=True)

    def _on_release(self, e: tk.Event) -> None:
        if not self._enabled:
            return

        self._draw()

        inside = 0 <= e.x <= self.winfo_width() and 0 <= e.y <= self.winfo_height()

        if inside and self._command:
            self._command()

    # ── 對外 API ─────────────────────────────────────────

    def set_enabled(self, enabled: bool) -> None:
        self._enabled = enabled
        self._hover = False
        self._draw()


# ═════════════════════════════════════════════════════════════
# 元件：圓角進度條
# ═════════════════════════════════════════════════════════════


class RoundedProgressBar(tk.Canvas):
    def __init__(
        self,
        parent: tk.Misc,
        *,
        height: int = 10,
        radius: int = 5,
        track_color: str = COLOR_SURFACE_ALT,
        fill_color: str = COLOR_ACCENT,
    ) -> None:
        super().__init__(
            parent,
            height=height,
            bg=_parent_bg(parent),
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
            0,
            0,
            width,
            height,
            self._radius,
            fill=self._track_color,
            outline="",
        )

        fill_width = width * (self._value / 100.0)

        if fill_width >= 2:
            draw_rounded_rect(
                self,
                0,
                0,
                fill_width,
                height,
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
        parent: tk.Misc,
        *,
        height: int = 46,
        radius: int = 12,
        font: tuple[Any, ...] = FONT_UI,
    ) -> None:
        super().__init__(
            parent,
            height=height,
            bg=_parent_bg(parent),
            highlightthickness=0,
            bd=0,
        )

        self._radius = radius
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

        self.entry.bind("<FocusIn>", lambda _e: self._set_focus(True))
        self.entry.bind("<FocusOut>", lambda _e: self._set_focus(False))

        self.bind("<Configure>", lambda _e: self._draw())

    def _set_focus(self, focused: bool) -> None:
        self._focused = focused
        self._draw()

    def _draw(self) -> None:
        self.delete("all")

        width = self.winfo_width()
        height = self.winfo_height()

        if width < 4 or height < 4:
            return

        draw_rounded_rect(
            self,
            1,
            1,
            width - 1,
            height - 1,
            self._radius,
            fill=COLOR_SURFACE,
            outline=COLOR_ACCENT if self._focused else COLOR_BORDER,
            width=2 if self._focused else 1,
        )

        inset_x, inset_y = 14, 6

        self.create_window(
            inset_x,
            inset_y,
            anchor="nw",
            window=self.entry,
            width=width - inset_x * 2,
            height=height - inset_y * 2,
        )


# ═════════════════════════════════════════════════════════════
# 背景執行緒 → 主執行緒 事件
# ═════════════════════════════════════════════════════════════


class _Event(NamedTuple):
    job: int
    kind: str  # "title" | "progress" | "success" | "failure"
    data: Any = None


# ═════════════════════════════════════════════════════════════
# 主程式
# ═════════════════════════════════════════════════════════════


class VideoDownloaderApp:
    MAX_LOG_LINES: Final = 30
    EMPTY_TITLE: Final = "—"

    def __init__(self, root: tk.Tk) -> None:
        self.root = root

        # ── 視窗 ────────────────────────────────────────
        self.root.title(t("window_title"))
        self.root.geometry(f"{DEFAULT_WIDTH}x{DEFAULT_HEIGHT}")
        self.root.minsize(MIN_WIDTH, MIN_HEIGHT)
        self.root.resizable(True, True)
        self.root.configure(bg=COLOR_BG)

        # ── 下載狀態 ────────────────────────────────────
        self._process: subprocess.Popen[str] | None = None
        self._events: queue.Queue[_Event] = queue.Queue()
        self._job_id = 0
        self._cancel_event = threading.Event()
        self._poll_after_id: str | None = None

        # 紀錄目前視窗寬度，用來動態調整影片名稱 wraplength。
        self._last_window_width = DEFAULT_WIDTH

        # ── UI ──────────────────────────────────────────
        self._build_ui()

        self.root.bind("<Configure>", self._on_window_configure)
        self.root.protocol("WM_DELETE_WINDOW", self._on_close)

        self._poll_events()

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

        main = tk.Frame(self.root, bg=COLOR_BG)
        main.pack(fill=tk.BOTH, expand=True, padx=22, pady=(20, 18))
        main.grid_columnconfigure(0, weight=1)

        self._build_title(main)
        self._build_url_section(main)
        self._build_button_section(main)
        self._build_video_section(main)
        self._build_status_section(main)

    # ── Title ─────────────────────────────────────────────

    def _build_title(self, parent: tk.Frame) -> None:
        title_frame = tk.Frame(parent, bg=COLOR_BG)
        title_frame.grid(row=0, column=0, sticky="ew")
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
        icon_chip.grid(row=0, column=0, padx=(0, 12))

        draw_rounded_rect(
            icon_chip,
            1,
            1,
            37,
            37,
            11,
            fill=COLOR_ACCENT_SOFT,
            outline="",
        )

        icon_chip.create_text(
            19,
            19,
            text="⬇",
            fill=COLOR_ACCENT,
            font=FONT_ICON,
        )

        tk.Label(
            title_frame,
            text=t("window_title"),
            font=FONT_TITLE,
            bg=COLOR_BG,
            fg=COLOR_TEXT,
            anchor="w",
        ).grid(row=0, column=1, sticky="ew")

        # Version badge（圓角膠囊）
        self._version_badge = self._make_pill_button(
            title_frame,
            text=f"v{APP_VERSION}",
            command=self._show_version_info,
        )
        self._version_badge.grid(row=0, column=2, sticky="e")

        # Separator
        tk.Frame(parent, height=1, bg=COLOR_BORDER).grid(
            row=1,
            column=0,
            sticky="ew",
            pady=(16, 0),
        )

    def _make_pill_button(
        self,
        parent: tk.Widget,
        text: str,
        command: Callable[[], None],
    ) -> tk.Canvas:
        approx_width = 16 + len(text) * 7
        height = 28

        canvas = tk.Canvas(
            parent,
            width=approx_width,
            height=height,
            bg=_parent_bg(parent),
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
                1,
                1,
                w - 1,
                h - 1,
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

    # ── URL ───────────────────────────────────────────────

    def _build_url_section(self, parent: tk.Frame) -> None:
        url_frame = tk.Frame(parent, bg=COLOR_BG)
        url_frame.grid(row=2, column=0, sticky="ew", pady=(20, 0))
        url_frame.grid_columnconfigure(0, weight=1)

        tk.Label(
            url_frame,
            text=t("prompt_url"),
            font=FONT_LABEL,
            bg=COLOR_BG,
            fg=COLOR_TEXT_DIM,
            anchor="w",
        ).grid(row=0, column=0, sticky="ew", pady=(0, 8))

        self._rounded_entry = RoundedEntry(url_frame, height=46, radius=12)
        self._rounded_entry.grid(row=1, column=0, sticky="ew")

        self.entry = self._rounded_entry.entry
        self.entry.bind("<Return>", lambda _e: self._on_download())

        self._bind_entry_context_menu(self.entry)

    # ── Buttons ───────────────────────────────────────────

    def _build_button_section(self, parent: tk.Frame) -> None:
        btn_frame = tk.Frame(parent, bg=COLOR_BG)
        btn_frame.grid(row=3, column=0, sticky="ew", pady=(16, 0))

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
        self.btn_cancel.pack(side="left", padx=(10, 0))
        self.btn_cancel.set_enabled(False)

    # ── Video ─────────────────────────────────────────────

    def _build_video_section(self, parent: tk.Frame) -> None:
        video_frame = tk.Frame(parent, bg=COLOR_BG)
        video_frame.grid(row=4, column=0, sticky="nsew", pady=(20, 0))
        video_frame.grid_columnconfigure(0, weight=1)
        parent.grid_rowconfigure(4, weight=1)

        # Video card（圓角卡片）
        self._video_card = RoundedCard(
            video_frame,
            bg_color=COLOR_SURFACE,
            border_color=COLOR_BORDER,
            radius=14,
            shadow=True,
            height=90,
        )
        self._video_card.grid(row=0, column=0, sticky="ew")

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

        self.title_var = tk.StringVar(value=self.EMPTY_TITLE)

        self.video_title_label = tk.Label(
            pad,
            textvariable=self.title_var,
            font=FONT_UI,
            bg=COLOR_SURFACE,
            fg=COLOR_TEXT,
            anchor="w",
            justify="left",
        )
        self.video_title_label.grid(row=1, column=0, sticky="ew", pady=(4, 0))

        # Progress
        progress_frame = tk.Frame(video_frame, bg=COLOR_BG)
        progress_frame.grid(row=1, column=0, sticky="ew", pady=(18, 0))
        progress_frame.grid_columnconfigure(0, weight=1)

        self.progress_bar = RoundedProgressBar(
            progress_frame,
            height=10,
            radius=5,
            track_color=COLOR_SURFACE_ALT,
            fill_color=COLOR_ACCENT,
        )
        self.progress_bar.grid(row=0, column=0, sticky="ew")

        self.pct_var = tk.StringVar(value="")

        tk.Label(
            progress_frame,
            textvariable=self.pct_var,
            font=FONT_SMALL,
            bg=COLOR_BG,
            fg=COLOR_TEXT_DIM,
            anchor="e",
        ).grid(row=1, column=0, sticky="ew", pady=(5, 0))

    # ── Status ────────────────────────────────────────────

    def _build_status_section(self, parent: tk.Frame) -> None:
        status_frame = tk.Frame(parent, bg=COLOR_BG)
        status_frame.grid(row=5, column=0, sticky="ew", pady=(14, 0))
        status_frame.grid_columnconfigure(1, weight=1)

        self.status_dot_canvas = tk.Canvas(
            status_frame,
            width=10,
            height=10,
            bg=COLOR_BG,
            highlightthickness=0,
            bd=0,
        )
        self.status_dot_canvas.grid(row=0, column=0, padx=(2, 8))

        self._status_dot_item = self.status_dot_canvas.create_oval(
            1,
            1,
            9,
            9,
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
        self.status_label.grid(row=0, column=1, sticky="ew")

    # ═════════════════════════════════════════════════════
    # Responsive Resize
    # ═════════════════════════════════════════════════════

    def _on_window_configure(self, event: tk.Event) -> None:
        """
        視窗 Resize 時動態調整影片名稱 wraplength，
        不修改字體大小、不使用 bitmap scale。
        """

        if event.widget is not self.root:
            return

        width = event.width

        if width <= 0 or width == self._last_window_width:
            return

        self._last_window_width = width

        with suppress(tk.TclError):
            self.video_title_label.config(wraplength=max(240, width - 110))

    # ═════════════════════════════════════════════════════
    # Version
    # ═════════════════════════════════════════════════════

    def _show_version_info(self) -> None:
        messagebox.showinfo(
            t("version_info"),
            (
                f"{t('version_title')} v{APP_VERSION}\n\n"
                f"{t('yt_dlp')} : {YTDLP_VERSION}\n"
                f"{t('ffmpeg')} : {FFMPEG_VERSION}\n"
                f"{t('deno')}   : {DENO_VERSION}"
            ),
        )

    # ═════════════════════════════════════════════════════
    # Context Menu
    # ═════════════════════════════════════════════════════

    def _bind_entry_context_menu(self, entry: tk.Entry) -> None:
        # 選單項目 index（含 separator 佔用 index 3）
        cut_i, copy_i, paste_i, select_all_i, clear_i = 0, 1, 2, 4, 5

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

        def is_editable() -> bool:
            return str(entry["state"]) != "disabled"

        def do_cut() -> None:
            if is_editable():
                with suppress(tk.TclError):
                    entry.event_generate("<<Cut>>")

        def do_copy() -> None:
            with suppress(tk.TclError):
                entry.event_generate("<<Copy>>")

        def do_paste() -> None:
            if not is_editable():
                return

            with suppress(tk.TclError):
                with suppress(tk.TclError):
                    entry.delete(tk.SEL_FIRST, tk.SEL_LAST)

                entry.insert(tk.INSERT, self.root.clipboard_get())

        def do_select_all() -> None:
            entry.select_range(0, tk.END)
            entry.icursor(tk.END)

        def do_clear() -> None:
            if is_editable():
                entry.delete(0, tk.END)

        menu.add_command(label=t("ctx_cut"), command=do_cut)
        menu.add_command(label=t("ctx_copy"), command=do_copy)
        menu.add_command(label=t("ctx_paste"), command=do_paste)
        menu.add_separator()
        menu.add_command(label=t("ctx_select_all"), command=do_select_all)
        menu.add_command(label=t("ctx_clear"), command=do_clear)

        def show_menu(event: tk.Event) -> None:
            editable = is_editable()
            has_selection = entry.selection_present()

            def state(enabled: bool) -> str:
                return "normal" if enabled else "disabled"

            menu.entryconfig(cut_i, state=state(editable and has_selection))
            menu.entryconfig(copy_i, state=state(has_selection))
            menu.entryconfig(paste_i, state=state(editable))
            menu.entryconfig(select_all_i, state="normal")
            menu.entryconfig(clear_i, state=state(editable))

            menu.tk_popup(event.x_root, event.y_root)

        entry.bind("<Button-3>", show_menu)

        if IS_MAC:
            entry.bind("<Button-2>", show_menu)

    # ═════════════════════════════════════════════════════
    # UI State helpers
    # ═════════════════════════════════════════════════════

    def _set_status(
        self,
        text: str,
        dot_color: str = COLOR_TEXT_DIM,
    ) -> None:
        self.status_label.config(text=text)
        self.status_dot_canvas.itemconfig(self._status_dot_item, fill=dot_color)

    def _set_title_display(self, title: str) -> None:
        self.title_var.set(title or self.EMPTY_TITLE)

    def _set_downloading(self, downloading: bool) -> None:
        self.entry.config(state="disabled" if downloading else "normal")
        self.btn_download.set_enabled(not downloading)
        self.btn_cancel.set_enabled(downloading)

        if not downloading:
            self.progress_bar.set(0)
            self.pct_var.set("")

    # ═════════════════════════════════════════════════════
    # Download
    # ═════════════════════════════════════════════════════

    def _build_args(
        self,
        url: str,
        yt_dlp: Path,
        ffmpeg: Path,
        output_dir: Path,
    ) -> list[str]:
        """
        組合 yt-dlp 參數。URL 一律放在 "--" 之後，
        避免以 "-" 開頭的輸入被當成選項。
        """

        return [
            str(yt_dlp),
            "-o",
            str(output_dir / "%(title).80B [%(id)s].%(ext)s"),
            "-f",
            "bv*[ext=mp4]+ba[ext=m4a]/bv*+ba/b",
            "--windows-filenames",
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
            "--",
            url,
        ]

    def _on_download(self) -> None:
        # 下載進行中（Return 鍵也會觸發）時忽略
        if self._process and self._process.poll() is None:
            return

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

        # deno optional
        if not get_tool_path("deno"):
            messagebox.showwarning("提示", t("deno_missing"))

        # ── Reset state ─────────────────────────────────
        self._job_id += 1
        self._cancel_event = threading.Event()

        self._set_downloading(True)
        self._set_title_display("")
        self._set_status(t("fetching_formats"), COLOR_WARNING)

        output_dir = get_app_dir()
        args = self._build_args(url, yt_dlp, ffmpeg, output_dir)

        threading.Thread(
            target=self._run_download,
            args=(self._job_id, self._cancel_event, args, output_dir),
            daemon=True,
        ).start()

    def _terminate_process(self) -> None:
        proc = self._process

        if proc and proc.poll() is None:
            with suppress(Exception):
                proc.terminate()

    def _on_cancel(self) -> None:
        self._cancel_event.set()

        # 讓舊 job 之後送出的事件全部失效
        self._job_id += 1

        self._terminate_process()

        self._set_status(t("cancelled"), COLOR_WARNING)
        self._set_downloading(False)

    # ═════════════════════════════════════════════════════
    # Download Worker（背景執行緒：不可碰 Tk）
    # ═════════════════════════════════════════════════════

    def _run_download(
        self,
        job: int,
        cancel: threading.Event,
        args: list[str],
        output_dir: Path,
    ) -> None:
        def post(kind: str, data: Any = None) -> None:
            self._events.put(_Event(job, kind, data))

        log_lines: deque[str] = deque(maxlen=self.MAX_LOG_LINES)

        # Windows：不顯示 console 視窗。
        creationflags = getattr(subprocess, "CREATE_NO_WINDOW", 0)

        child_env = os.environ.copy()

        # 避免 subprocess inherited UTF-8 導致 Windows 編碼錯誤。
        child_env.pop("PYTHONUTF8", None)
        child_env.pop("PYTHONIOENCODING", None)

        try:
            proc = subprocess.Popen(
                args,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                encoding=locale.getencoding(),
                errors="replace",
                cwd=output_dir,
                env=child_env,
                creationflags=creationflags,
            )
        except Exception as exc:
            post("failure", str(exc))
            return

        self._process = proc

        with proc:
            # 在 Popen 完成前就按下取消的情況
            if cancel.is_set():
                proc.terminate()

            title_seen = False

            assert proc.stdout is not None

            for line in proc.stdout:
                if cancel.is_set():
                    break

                log_lines.append(line)

                if not title_seen and (title := parse_title(line)):
                    title_seen = True
                    post("title", title)

                if (pct := parse_progress(line)) is not None:
                    post("progress", pct)

            if cancel.is_set():
                return  # __exit__ 會等待已被 terminate 的行程結束

            returncode = proc.wait()

        if returncode == 0:
            post("success", output_dir)
        else:
            post("failure", "".join(log_lines).strip() or t("download_failed"))

    # ═════════════════════════════════════════════════════
    # 事件輪詢（主執行緒）
    # ═════════════════════════════════════════════════════

    def _poll_events(self) -> None:
        try:
            while True:
                event = self._events.get_nowait()

                if event.job == self._job_id:
                    self._handle_event(event)
        except queue.Empty:
            pass

        self._poll_after_id = self.root.after(POLL_INTERVAL_MS, self._poll_events)

    def _handle_event(self, event: _Event) -> None:
        match event.kind:
            case "title":
                self._set_title_display(event.data)

            case "progress":
                pct: float = event.data
                self.progress_bar.set(pct)
                self.pct_var.set(f"{pct:.1f}%")
                self._set_status(t("downloading"), COLOR_ACCENT)

            case "success":
                self._on_success(event.data)

            case "failure":
                self._on_failure(event.data)

    def _on_success(self, output_dir: Path) -> None:
        self.progress_bar.set(100)
        self.pct_var.set("100%")

        self._set_status(t("download_success"), COLOR_SUCCESS)
        self._set_downloading(False)

        open_directory(output_dir)

    def _on_failure(self, err: str) -> None:
        self._set_status(t("download_failed"), COLOR_DANGER)
        self._set_downloading(False)

        messagebox.showerror("yt-dlp 錯誤", err)

    # ═════════════════════════════════════════════════════
    # Close
    # ═════════════════════════════════════════════════════

    def _on_close(self) -> None:
        # 視窗關閉時，如果還在下載，先終止 yt-dlp。
        self._cancel_event.set()
        self._terminate_process()

        if self._poll_after_id is not None:
            self.root.after_cancel(self._poll_after_id)

        self.root.destroy()


# ═════════════════════════════════════════════════════════════
# Entry Point
# ═════════════════════════════════════════════════════════════


def main() -> None:
    root = tk.Tk()

    VideoDownloaderApp(root)

    root.mainloop()


if __name__ == "__main__":
    main()
