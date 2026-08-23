# -*- mode: python ; coding: utf-8 -*-
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

from PyInstaller.utils.hooks import collect_submodules

_SHIM_THRESHOLD_MB = 1.0


def find_tool(name: str, required_mb: float = 0.0) -> str | None:
    candidates: list[str] = []

    found = shutil.which(name)
    if found:
        candidates.append(found)

    if os.name == "nt":
        found_exe = shutil.which(f"{name}.exe")
        if found_exe and found_exe not in candidates:
            candidates.append(found_exe)

    for path in candidates:
        if required_mb > 0:
            size_mb = os.path.getsize(path) / 1024 / 1024

            if size_mb < required_mb:
                print(
                    f"[spec] skip {path} "
                    f"({size_mb:.2f} MB, possible shim)"
                )
                continue

        return path

    return None


def to_short_path(path: str, name: str) -> str:
    dest_dir = Path(tempfile.gettempdir()) / "pyinstaller_tools"
    dest_dir.mkdir(exist_ok=True)

    suffix = Path(path).suffix
    dest = dest_dir / f"{name}{suffix}"

    src_size = os.path.getsize(path)
    dst_size = os.path.getsize(dest) if dest.exists() else -1

    if src_size != dst_size:
        print(
            f"[spec] copy {name} "
            f"({src_size / 1024 / 1024:.1f} MB) -> {dest}"
        )
        shutil.copy2(path, dest)
    else:
        print(f"[spec] cached copy is up to date: {dest}")

    return str(dest)


def get_tool_version(path: str, name: str) -> str:
    try:
        result = subprocess.run(
            [path, "--version"],
            capture_output=True,
            text=True,
            timeout=10,
            encoding="utf-8",
            errors="ignore",
        )

        output = result.stdout.strip() or result.stderr.strip()

        if not output:
            return "unknown"

        first_line = output.splitlines()[0]

        if name == "yt-dlp":
            return first_line

        if name == "ffmpeg":
            m = re.search(
                r"ffmpeg version\s+([^\s]+)",
                first_line,
                re.IGNORECASE,
            )
            return m.group(1) if m else first_line

        if name == "deno":
            m = re.search(
                r"deno\s+([0-9.]+)",
                first_line,
                re.IGNORECASE,
            )
            return m.group(1) if m else first_line

        return first_line

    except Exception as e:
        print(f"[spec] failed to get {name} version: {e}")
        return "unknown"


# ── 搜尋工具 ──────────────────────────────────────────────────────────────────
yt_dlp_path = find_tool("yt-dlp")
ffmpeg_path = find_tool("ffmpeg", required_mb=_SHIM_THRESHOLD_MB)
deno_path = find_tool("deno", required_mb=_SHIM_THRESHOLD_MB)

APP_VERSION = "2026.08.20-Fix.1"
YTDLP_VERSION = (
    get_tool_version(yt_dlp_path, "yt-dlp")
    if yt_dlp_path
    else "missing"
)
FFMPEG_VERSION = (
    get_tool_version(ffmpeg_path, "ffmpeg")
    if ffmpeg_path
    else "missing"
)
DENO_VERSION = (
    get_tool_version(deno_path, "deno")
    if deno_path
    else "missing"
)


for label, path in [
    ("yt-dlp", yt_dlp_path),
    ("ffmpeg", ffmpeg_path),
    ("deno", deno_path),
]:
    if path:
        print(
            f"[spec] {label:8s} -> {path} "
            f"({os.path.getsize(path) / 1024 / 1024:.1f} MB)"
        )
    else:
        print(f"[spec] {label:8s} -> not found")


print(
    f"[spec] version "
    f"app={APP_VERSION} "
    f"yt-dlp={YTDLP_VERSION} "
    f"ffmpeg={FFMPEG_VERSION} "
    f"deno={DENO_VERSION}"
)


missing = [
    n
    for n, p in {
        "yt-dlp": yt_dlp_path,
        "ffmpeg": ffmpeg_path,
    }.items()
    if not p
]

if missing:
    raise FileNotFoundError(
        f"Required tools not found: {', '.join(missing)}"
    )


# ── 複製到短路徑 ──────────────────────────────────────────────────────────────
yt_dlp_short = to_short_path(yt_dlp_path, "yt-dlp")
ffmpeg_short = to_short_path(ffmpeg_path, "ffmpeg")


# ── binaries ──────────────────────────────────────────────────────────────────
binaries = [(yt_dlp_short, ".")]

if deno_path:
    deno_short = to_short_path(deno_path, "deno")
    binaries.append((deno_short, "."))
else:
    print(
        "[spec] WARNING: deno not found; "
        "YouTube quality may be limited."
    )


# ── datas（ffmpeg 純複製，繞過 binary 分析）───────────────────────────────────
datas = [(ffmpeg_short, ".")]

print(f"[spec] binaries = {[b[0] for b in binaries]}")
print(f"[spec] datas    = {[d[0] for d in datas]}")


# ── runtime hook：將版本字串注入 sys，取代 build_info.py ──────────────────────
hook_content = f"""\
import sys
sys._app_version    = "{APP_VERSION}"
sys._ytdlp_version  = "{YTDLP_VERSION}"
sys._ffmpeg_version = "{FFMPEG_VERSION}"
sys._deno_version   = "{DENO_VERSION}"
"""


hook_path = Path("runtime_hook_versions.py")
hook_path.write_text(
    hook_content,
    encoding="utf-8",
)

print(f"[spec] runtime hook written: {hook_path}")


# ── Analysis ──────────────────────────────────────────────────────────────────
a = Analysis(
    ["video_downloader.py"],
    pathex=[],
    binaries=binaries,
    datas=datas,
    hiddenimports=collect_submodules("tkinter"),
    hookspath=[],
    hooksconfig={},
    runtime_hooks=["runtime_hook_versions.py"],
    excludes=["pytest", "unittest"],
    noarchive=False,
)


pyz = PYZ(a.pure)


exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name="video_downloader",
    debug=False,
    strip=False,
    upx=False,
    console=False,
)


# ── build 完畢後刪除 runtime hook 暫存檔 ─────────────────────────────────────
try:
    hook_path.unlink()
    print(f"[spec] runtime hook removed: {hook_path}")
except Exception as e:
    print(
        f"[spec] failed to remove runtime hook "
        f"(ignored): {e}"
    )


# build/ 資料夾為 PyInstaller 的中間產物
build_dir = Path("build")

try:
    shutil.rmtree(build_dir)
    print(f"[spec] build directory removed: {build_dir}")
except Exception as e:
    print(
        f"[spec] failed to remove build directory "
        f"(ignored): {e}"
    )
