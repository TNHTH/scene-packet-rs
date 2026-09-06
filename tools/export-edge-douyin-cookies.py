#!/usr/bin/env python
"""从 Edge 用户目录导出 .douyin.com Cookie 到 Netscape 文件（供 yt-dlp/avr 使用）。"""
from __future__ import annotations

import importlib.util
import os
import shutil
import sqlite3
import sys
import tempfile
import time
from pathlib import Path

OUT = Path(r"E:\AIVideoReader\cookies\douyin.txt")
EDGE_COOKIES = (
    Path(os.environ["LOCALAPPDATA"])
    / "Microsoft/Edge/User Data/Default/Network/Cookies"
)
EXPIRY = str(int(time.time()) + 86400 * 30)
REFRESH = Path(__file__).with_name("refresh-douyin-cookies.py")


def _load_refresh():
    spec = importlib.util.spec_from_file_location("refresh_douyin_cookies", REFRESH)
    mod = importlib.util.module_from_spec(spec)
    assert spec.loader
    spec.loader.exec_module(mod)
    return mod


def write_netscape(rows: list[tuple[str, str, str, str, str, str, str]]) -> None:
    lines = ["# Netscape HTTP Cookie File", "# Exported from Edge"]
    for domain, flag, cpath, secure, expiry, name, value in sorted(rows, key=lambda r: r[5]):
        lines.append(f"{domain}\t{flag}\t{cpath}\t{secure}\t{expiry}\t{name}\t{value}")
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8")


def export_from_edge(ms_token: str | None = None) -> int:
    if not EDGE_COOKIES.exists():
        print(f"Edge Cookies 不存在: {EDGE_COOKIES}", file=sys.stderr)
        return 1
    tmp = Path(tempfile.gettempdir()) / f"edge-cookies-{int(time.time())}.db"
    try:
        shutil.copy2(EDGE_COOKIES, tmp)
    except PermissionError:
        print(
            "无法复制 Edge Cookies（Edge 可能正在运行）。"
            "请关闭 Edge 后重试，或在 Edge 已登录抖音页 DevTools 复制 cookie+xmst → refresh-douyin-cookies.py。",
            file=sys.stderr,
        )
        return 2
    conn = sqlite3.connect(str(tmp))
    cur = conn.execute(
        "SELECT host_key, name, value, path, is_secure, encrypted_value FROM cookies "
        "WHERE host_key LIKE '%douyin%'"
    )
    rows: list[tuple[str, str, str, str, str, str, str]] = []
    skipped = 0
    for host, name, value, path, secure, enc in cur.fetchall():
        if not value and enc:
            skipped += 1
            continue
        domain = host if host.startswith(".") else f".{host.lstrip('.')}"
        rows.append((domain, "TRUE", path or "/", "TRUE" if secure else "FALSE", EXPIRY, name, value))
    conn.close()
    tmp.unlink(missing_ok=True)
    if not rows:
        hint = "（含加密项未解密，须关 Edge 后重试）" if skipped else ""
        print(f"未找到可用 douyin.com Cookie{hint}", file=sys.stderr)
        return 3
    write_netscape(rows)
    if ms_token:
        refresh = _load_refresh()
        refresh.merge_ms_token(ms_token.strip(), OUT)
        print("msToken merged")
    print(f"exported {len(rows)} cookies -> {OUT}")
    return 0


def main() -> int:
    ms = sys.argv[1] if len(sys.argv) > 1 else None
    return export_from_edge(ms)


if __name__ == "__main__":
    raise SystemExit(main())
