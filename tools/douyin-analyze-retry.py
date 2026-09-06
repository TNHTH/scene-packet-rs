#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""抖音链接：刷新 Cookie → avr analyze →（可选）yt-dlp 下载 + transcribe 兜底。"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parents[1]
AVR = ROOT / "target" / "release" / "avr.exe"
REFRESH_JSON = Path(__file__).with_name("refresh-douyin-from-json.py")
COOKIES = Path(r"E:\AIVideoReader\cookies\douyin.txt")
YTDLP = Path(r"E:\yt-dlp\yt-dlp.exe")
UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0"
)
TMP = REPO / ".tmp" / "codex"


def run(cmd: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    print("+", " ".join(str(c) for c in cmd))
    return subprocess.run(
        cmd,
        text=True,
        capture_output=True,
        check=check,
        encoding="utf-8",
        errors="replace",
    )


def refresh_creds(json_path: Path) -> None:
    r = run([sys.executable, str(REFRESH_JSON), str(json_path)])
    print(r.stdout.strip())
    if r.returncode != 0:
        print(r.stderr, file=sys.stderr)
        raise SystemExit(r.returncode)


def analyze(url: str) -> tuple[int, str]:
    if not AVR.exists():
        print(f"未找到 avr: {AVR}", file=sys.stderr)
        raise SystemExit(2)
    r = run([str(AVR), "analyze", url], check=False)
    out = (r.stdout or "") + (r.stderr or "")
    print(out)
    return r.returncode, out


def find_cache_from_output(text: str) -> Path | None:
    start = text.find("{")
    end = text.rfind("}")
    if start >= 0 and end > start:
        try:
            data = json.loads(text[start : end + 1])
            for key in ("cache_dir", "transcript_path"):
                if key in data:
                    raw = str(data[key])
                    if raw.startswith("file://"):
                        raw = raw.replace("file://?/", "").replace("/", "\\")
                    p = Path(raw)
                    return p.parent if key == "transcript_path" else p
        except json.JSONDecodeError:
            pass
    return None


def download_and_transcribe(url: str, out_dir: Path) -> Path | None:
    out_dir.mkdir(parents=True, exist_ok=True)
    out_tpl = str(out_dir / "douyin-fallback.%(ext)s")
    if not YTDLP.exists():
        print(f"未找到 yt-dlp: {YTDLP}", file=sys.stderr)
        return None
    r = run(
        [
            str(YTDLP),
            "--cookies",
            str(COKIES),
            "--user-agent",
            UA,
            "-o",
            out_tpl,
            url,
        ],
        check=False,
    )
    print(r.stdout)
    if r.returncode != 0:
        print(r.stderr, file=sys.stderr)
        return None
    media = next(out_dir.glob("douyin-fallback.*"), None)
    if not media:
        print("下载成功但未找到输出文件", file=sys.stderr)
        return None
    tr = run([str(AVR), "transcribe", str(media)], check=False)
    print(tr.stdout)
    if tr.returncode != 0:
        print(tr.stderr, file=sys.stderr)
        return None
    return find_cache_from_output((tr.stdout or "") + (tr.stderr or ""))


def main() -> int:
    ap = argparse.ArgumentParser(description="抖音 analyze 重试流水线")
    ap.add_argument("url", help="抖音短链或作品页 URL")
    ap.add_argument("--creds-json", type=Path, help="含 cookie+xmst 的 JSON，先刷新再 analyze")
    ap.add_argument("--retries", type=int, default=2, help="analyze 次数（每次可先刷新 creds）")
    ap.add_argument("--fallback-download", action="store_true", help="analyze 全失败后 yt-dlp+transcribe")
    args = ap.parse_args()

    if args.creds_json:
        if not args.creds_json.exists():
            print(f"凭证文件不存在: {args.creds_json}", file=sys.stderr)
            return 1
        refresh_creds(args.creds_json)

    last_out = ""
    for i in range(args.retries):
        if i > 0 and args.creds_json:
            refresh_creds(args.creds_json)
        code, last_out = analyze(args.url)
        if code == 0:
            cache = find_cache_from_output(last_out)
            if cache:
                print(f"OK cache_dir={cache}")
            return 0

    if args.fallback_download:
        today = __import__("datetime").date.today().isoformat()
        cache = download_and_transcribe(args.url, TMP / today)
        if cache:
            print(f"OK fallback cache_dir={cache}")
            return 0

    print("全部尝试失败。note 图文见 content-sources.md；Edge 用户见 export-edge-douyin-cookies.py", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
