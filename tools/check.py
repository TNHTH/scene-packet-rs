#!/usr/bin/env python3
"""同一提交的本地/CI 门禁；不下载模型，不使用私有视频。"""
import os
from pathlib import Path
import subprocess
import sys

os.chdir(Path(__file__).resolve().parents[1])
env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1", PYTHONUTF8="1")
commands = [
    ["cargo", "test", "--locked", "-p", "avr-core", "-p", "avr-mcp", "-p", "avr-cli"],
    [
        "cargo",
        "clippy",
        "--locked",
        "-p",
        "avr-core",
        "-p",
        "avr-mcp",
        "-p",
        "avr-cli",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ],
    ["cargo", "build", "--locked", "-p", "avr-cli"],
    [sys.executable, "-B", "tools/validate-local-video.py", "--help"],
]
for command in commands:
    print("+ " + " ".join(command), flush=True)
    subprocess.run(command, env=env, check=True)
