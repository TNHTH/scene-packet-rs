#!/usr/bin/env python
"""测试多镜像下载速度，选最快线路安装到 E 盘同名目录。"""
from __future__ import annotations

import json
import os
import shutil
import sys
import tempfile
import time
import urllib.error
import urllib.request
import zipfile
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path

FFMPEG = Path(r"E:\ffmpeg")
YTDLP = Path(r"E:\yt-dlp")
WHISPER = Path(r"E:\whisper")
MODELS = Path(r"E:\models")
DATA_ROOT = Path(r"E:\AIVideoReader")

# 每个组件的候选镜像（按常见国内/国际线路）
MIRRORS: dict[str, list[tuple[str, str]]] = {
    "ffmpeg": [
        ("gyan-official", "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip"),
        ("github-ffmpeg", "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"),
    ],
    "yt-dlp": [
        ("github-latest", "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"),
        ("ghproxy", "https://ghproxy.net/https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"),
        ("mirror-ghproxy", "https://mirror.ghproxy.com/https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"),
    ],
    "whisper": [
        ("github-cublas", "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.0/whisper-cublas-11.8.0-bin-x64.zip"),
        ("ghproxy-cublas", "https://ghproxy.net/https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.0/whisper-cublas-11.8.0-bin-x64.zip"),
        ("github-cpu", "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.0/whisper-bin-x64.zip"),
    ],
    "model": [
        ("hf-official", "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin"),
        ("hf-mirror", "https://hf-mirror.com/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin"),
    ],
}

PROBE_BYTES = 512 * 1024  # 512KB 测速
TIMEOUT = 20


@dataclass
class ProbeResult:
    name: str
    url: str
    ok: bool
    speed_kbps: float = 0.0
    latency_ms: float = 0.0
    error: str = ""


def probe_mirror(name: str, url: str) -> ProbeResult:
    req = urllib.request.Request(
        url,
        headers={
            "User-Agent": "Mozilla/5.0 avr-mirror-probe",
            "Range": f"bytes=0-{PROBE_BYTES - 1}",
        },
    )
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            if resp.status not in (200, 206):
                return ProbeResult(name, url, False, error=f"HTTP {resp.status}")
            data = resp.read(PROBE_BYTES)
            elapsed = time.perf_counter() - t0
            if not data:
                return ProbeResult(name, url, False, error="empty")
            speed = len(data) / elapsed / 1024
            return ProbeResult(name, url, True, speed_kbps=speed, latency_ms=elapsed * 1000)
    except Exception as e:  # noqa: BLE001
        return ProbeResult(name, url, False, error=str(e)[:120])


def pick_fastest(component: str) -> tuple[str, str]:
    candidates = MIRRORS[component]
    print(f"\n=== 测速: {component} ({len(candidates)} 条线路) ===")
    results: list[ProbeResult] = []
    with ThreadPoolExecutor(max_workers=len(candidates)) as pool:
        futs = {pool.submit(probe_mirror, n, u): (n, u) for n, u in candidates}
        for f in as_completed(futs):
            r = f.result()
            results.append(r)
            if r.ok:
                print(f"  OK  {r.name:18} {r.speed_kbps:8.0f} KB/s  {r.latency_ms:6.0f} ms")
            else:
                print(f"  FAIL {r.name:18} {r.error}")

    ok = [r for r in results if r.ok]
    if not ok:
        raise RuntimeError(f"{component} 所有镜像均不可用")
    best = max(ok, key=lambda r: r.speed_kbps)
    print(f"  >> 选用: {best.name} ({best.speed_kbps:.0f} KB/s)")
    return best.name, best.url


def download_file(url: str, dest: Path, label: str) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".part")
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 avr-installer"})
    print(f"[下载] {label} <- {url[:80]}...")
    with urllib.request.urlopen(req, timeout=900) as resp, open(tmp, "wb") as f:
        total = int(resp.headers.get("Content-Length") or 0)
        done = 0
        t0 = time.perf_counter()
        while True:
            chunk = resp.read(1024 * 512)
            if not chunk:
                break
            f.write(chunk)
            done += len(chunk)
            if total and done % (5 * 1024 * 1024) < len(chunk):
                pct = done * 100 // total
                speed = done / (time.perf_counter() - t0 + 1e-6) / 1024
                print(f"  {pct}%  {speed:.0f} KB/s", flush=True)
    tmp.replace(dest)
    print(f"[完成] {dest}")


def extract_ffmpeg(zip_path: Path, out_dir: Path, alt_names: bool = False) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(zip_path) as zf:
        want = ["ffmpeg.exe", "ffprobe.exe"]
        for name in want:
            hit = next((i for i in zf.infolist() if Path(i.filename).name.lower() == name), None)
            if not hit and alt_names:
                # BtbN 包路径更深，按文件名匹配
                hit = next((i for i in zf.infolist() if Path(i.filename).name.lower() == name), None)
            if not hit:
                raise FileNotFoundError(f"zip 内无 {name}")
            target = out_dir / name
            with zf.open(hit) as src, open(target, "wb") as dst:
                shutil.copyfileobj(src, dst)


def extract_whisper(zip_path: Path, out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(zip_path) as zf:
        for info in zf.infolist():
            base = Path(info.filename).name
            if info.is_dir() or not base.lower().endswith((".exe", ".dll")):
                continue
            target = out_dir / base
            with zf.open(info) as src, open(target, "wb") as dst:
                shutil.copyfileobj(src, dst)


def _p(path: Path) -> str:
    """TOML 安全路径（避免 \\f \\n 等转义）。"""
    return path.as_posix()


def write_config() -> None:
    DATA_ROOT.mkdir(parents=True, exist_ok=True)
    for sub in ("cache", "exports", "logs", "bin"):
        (DATA_ROOT / sub).mkdir(exist_ok=True)
    meta = DATA_ROOT / ".avr-root"
    if not meta.exists():
        meta.write_text(json.dumps({"schema_version": 2, "created_by": "avr 2.0.0"}, indent=2), encoding="utf-8")
    model = MODELS / "ggml-large-v3-turbo.bin"
    cfg = f"""[extractor]
name = "ytdlp"
cookies_from_browser = "none"

[asr]
default = "auto"
active_preset = "whisper-cpp-cpu"
model_path = "{_p(model)}"

[output]
cache_dir = ""

[tools]
ffmpeg_path = "{_p(FFMPEG / 'ffmpeg.exe')}"
ffprobe_path = "{_p(FFMPEG / 'ffprobe.exe')}"
ytdlp_path = "{_p(YTDLP / 'yt-dlp.exe')}"
whisper_cli_path = "{_p(WHISPER / 'whisper-cli.exe')}"
whisper_model_path = "{_p(model)}"

[ui]
enable_transparency = true
scene_threshold = 0.3
visual_hint_limit = 50
"""
    config_path = DATA_ROOT / "config.toml"
    if not config_path.exists():
        config_path.write_text(cfg, encoding="utf-8")
    else:
        print("[保留] 已有 config.toml，未覆盖用户配置")
    appdata = Path(os.environ["APPDATA"]) / "AIVideoReader"
    appdata.mkdir(parents=True, exist_ok=True)
    ptr = appdata / "root.txt"
    tmp = ptr.parent / "root.txt.tmp"
    if not ptr.exists():
        tmp.write_text(str(DATA_ROOT), encoding="utf-8")
        tmp.replace(ptr)


def main() -> int:
    for p in (FFMPEG, YTDLP, WHISPER, MODELS):
        p.mkdir(parents=True, exist_ok=True)

    picks: dict[str, tuple[str, str]] = {}
    for comp in ("ffmpeg", "yt-dlp", "whisper", "model"):
        picks[comp] = pick_fastest(comp)

    # ffmpeg
    if not ((FFMPEG / "ffmpeg.exe").exists() and (FFMPEG / "ffprobe.exe").exists()):
        with tempfile.TemporaryDirectory() as td:
            z = Path(td) / "ffmpeg.zip"
            download_file(picks["ffmpeg"][1], z, f"ffmpeg ({picks['ffmpeg'][0]})")
            extract_ffmpeg(z, FFMPEG)
    else:
        print("[跳过] E:\\ffmpeg 已有")

    # yt-dlp
    yexe = YTDLP / "yt-dlp.exe"
    if not yexe.exists() or yexe.stat().st_size < 1_000_000:
        if yexe.exists():
            yexe.unlink()
        download_file(picks["yt-dlp"][1], yexe, f"yt-dlp ({picks['yt-dlp'][0]})")
    else:
        print("[跳过] E:\\yt-dlp 已有")

    # whisper
    if not (WHISPER / "whisper-cli.exe").exists():
        with tempfile.TemporaryDirectory() as td:
            z = Path(td) / "whisper.zip"
            download_file(picks["whisper"][1], z, f"whisper ({picks['whisper'][0]})")
            extract_whisper(z, WHISPER)
    else:
        print("[跳过] E:\\whisper 已有")

    # model
    model = MODELS / "ggml-large-v3-turbo.bin"
    if not model.exists() or model.stat().st_size < 100_000_000:
        if model.exists():
            model.unlink()
        download_file(picks["model"][1], model, f"model ({picks['model'][0]})")
    else:
        print("[跳过] E:\\models 已有")

    write_config()

    print("\n=== 安装完成（E 盘同名目录，删文件夹即卸载）===")
    print("  E:\\ffmpeg\\")
    print("  E:\\yt-dlp\\")
    print("  E:\\whisper\\")
    print("  E:\\models\\")
    print(f"  配置/缓存: E:\\AIVideoReader\\")
    return 0


if __name__ == "__main__":
    sys.exit(main())
