#!/usr/bin/env python
"""部署 cuBLAS 版 whisper + CUDA 11.8 cuBLAS DLL 到 E:\\whisper。"""
from __future__ import annotations

import json
import shutil
import subprocess
import sys
import urllib.request
import zipfile
from pathlib import Path

WHISPER = Path(r"E:\whisper")
DATA_ROOT = Path(r"E:\AIVideoReader")
LOGS = DATA_ROOT / "logs"
LOGS.mkdir(parents=True, exist_ok=True)

WHISPER_ZIP_URL = (
    "https://ghproxy.net/https://github.com/ggml-org/whisper.cpp/"
    "releases/download/v1.9.0/whisper-cublas-11.8.0-bin-x64.zip"
)
CUBLAS_REDIST = (
    "https://developer.download.nvidia.com/compute/cuda/redist/libcublas/"
    "windows-x86_64/libcublas-windows-x86_64-11.8.1.74-archive.zip"
)
NEED_DLLS = ("cublas64_11.dll", "cublasLt64_11.dll")


def download(url: str, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".part")
    print(f"[下载] {dest.name}")
    req = urllib.request.Request(url, headers={"User-Agent": "avr-gpu-setup/1.0"})
    with urllib.request.urlopen(req, timeout=900) as resp, open(tmp, "wb") as f:
        total = int(resp.headers.get("Content-Length") or 0)
        done = 0
        while True:
            chunk = resp.read(1024 * 512)
            if not chunk:
                break
            f.write(chunk)
            done += len(chunk)
            if total and done % (5 * 1024 * 1024) < len(chunk):
                print(f"  {done * 100 // total}%", flush=True)
    tmp.replace(dest)
    print(f"[完成] {dest}")


def deploy_whisper_cublas() -> None:
    if (WHISPER / "whisper-cli.exe").exists() and (WHISPER / "ggml-cuda.dll").exists():
        print("[跳过] cuBLAS whisper 已存在")
        return
    zpath = LOGS / "whisper-cublas.zip"
    download(WHISPER_ZIP_URL, zpath)
    WHISPER.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(zpath) as zf:
        for info in zf.infolist():
            base = Path(info.filename).name
            if info.is_dir() or not base.lower().endswith((".exe", ".dll")):
                continue
            target = WHISPER / base
            with zf.open(info) as src, open(target, "wb") as dst:
                shutil.copyfileobj(src, dst)
    zpath.unlink(missing_ok=True)
    print(f"[解包] cuBLAS whisper -> {WHISPER}")


def deploy_cublas_dlls() -> None:
    if all((WHISPER / n).exists() for n in NEED_DLLS):
        print("[跳过] cuBLAS DLL 已存在")
        return
    zpath = LOGS / "libcublas-redist.zip"
    download(CUBLAS_REDIST, zpath)
    with zipfile.ZipFile(zpath) as zf:
        found: dict[str, bytes] = {}
        for info in zf.infolist():
            base = Path(info.filename).name
            if base in NEED_DLLS and base not in found:
                with zf.open(info) as src:
                    found[base] = src.read()
        for name in NEED_DLLS:
            if name not in found:
                raise FileNotFoundError(f"redist 包中未找到 {name}")
            out = WHISPER / name
            out.write_bytes(found[name])
            print(f"[拷贝] {out} ({out.stat().st_size // 1024 // 1024} MB)")
    zpath.unlink(missing_ok=True)


def verify() -> None:
    exe = WHISPER / "whisper-cli.exe"
    r = subprocess.run(
        [str(exe), "-h"],
        cwd=str(WHISPER),
        capture_output=True,
        text=True,
        timeout=30,
    )
    if r.returncode != 0:
        print("STDOUT:", r.stdout[:500])
        print("STDERR:", r.stderr[:500])
        raise RuntimeError(f"whisper-cli 启动失败 exit={r.returncode}")
    print("[验证] whisper-cli -h OK")


def update_config() -> None:
    cfg_path = DATA_ROOT / "config.toml"
    text = cfg_path.read_text(encoding="utf-8")
    text = text.replace("whisper-cpp-cpu", "whisper-cpp-cublas")
    cfg_path.write_text(text, encoding="utf-8")
    print("[配置] active_preset -> whisper-cpp-cublas")


def main() -> int:
    deploy_whisper_cublas()
    deploy_cublas_dlls()
    verify()
    update_config()
    print("\n=== GPU 版 whisper 已就绪 ===")
    print(f"  目录: {WHISPER}")
    print(f"  新增: {', '.join(NEED_DLLS)}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:
        print(f"失败: {e}", file=sys.stderr)
        sys.exit(1)
