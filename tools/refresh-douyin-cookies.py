#!/usr/bin/env python
"""合并 document.cookie 与现有 Netscape 文件，保留 HttpOnly 项（如 ttwid）。"""
from __future__ import annotations

import sys
import time
from pathlib import Path

OUT = Path(r"E:\AIVideoReader\cookies\douyin.txt")
DOMAIN = ".douyin.com"
EXPIRY = str(int(time.time()) + 86400 * 30)


def parse_netscape(path: Path) -> dict[tuple[str, str], tuple[str, str, str, str]]:
    """(domain, name) -> (domain, secure, path, expiry, name, value)"""
    rows: dict[tuple[str, str], tuple[str, str, str, str, str, str]] = {}
    if not path.exists():
        return rows
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 7:
            continue
        domain, flag, cpath, secure, expiry, name, value = parts
        rows[(domain, name)] = (domain, flag, cpath, secure, expiry, name, value)
    return rows


def write_netscape(path: Path, rows: dict[tuple[str, str], tuple[str, str, str, str, str, str]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = ["# Netscape HTTP Cookie File", "# Refreshed for yt-dlp"]
    for (_domain, _name), (domain, flag, cpath, secure, expiry, name, value) in sorted(
        rows.items(), key=lambda x: x[1][5]
    ):
        lines.append(f"{domain}\t{flag}\t{cpath}\t{secure}\t{expiry}\t{name}\t{value}")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def merge_document_cookie(cookie_str: str, existing: Path) -> int:
    rows = parse_netscape(existing)
    updated = 0
    for part in cookie_str.split("; "):
        part = part.strip()
        if not part or "=" not in part:
            continue
        name, value = part.split("=", 1)
        if name == "douyin.com":
            continue
        key = (DOMAIN, name)
        old = rows.get(key)
        secure = old[3] if old else "FALSE"
        rows[key] = (DOMAIN, "TRUE", "/", secure, EXPIRY, name, value)
        updated += 1
    write_netscape(existing, rows)
    return updated


def merge_ms_token(token: str, existing: Path = OUT) -> None:
    """写入 msToken（来自 localStorage xmst 或浏览器导出）。"""
    if not token.strip():
        return
    rows = parse_netscape(existing)
    rows[(DOMAIN, "msToken")] = (DOMAIN, "TRUE", "/", "FALSE", EXPIRY, "msToken", token.strip())
    write_netscape(existing, rows)


def main() -> int:
    if len(sys.argv) < 2:
        print(
            "用法: python refresh-douyin-cookies.py \"document.cookie字符串\" [msToken]",
            file=sys.stderr,
        )
        return 1
    cookie_str = sys.argv[1]
    n = merge_document_cookie(cookie_str, OUT)
    if len(sys.argv) >= 3:
        merge_ms_token(sys.argv[2], OUT)
        print(f"msToken updated")
    print(f"updated {n} cookies -> {OUT} (total {len(parse_netscape(OUT))})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
