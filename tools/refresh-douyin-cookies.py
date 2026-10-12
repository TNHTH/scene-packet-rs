#!/usr/bin/env python
"""合并 document.cookie 与 Netscape 文件，保留 HttpOnly 和不同路径项。"""
from __future__ import annotations

import argparse
import os
import tempfile
import time
from pathlib import Path
from typing import Tuple

OUT = Path(r"E:\AIVideoReader\cookies\douyin.txt")
DOMAIN = ".douyin.com"
CookieKey = Tuple[str, str, str]
CookieRow = Tuple[str, str, str, str, str, str, str]


def parse_netscape(path: Path) -> dict[CookieKey, CookieRow]:
    rows = {}
    if not path.exists():
        return rows
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line or (line.startswith("#") and not line.startswith("#HttpOnly_")):
            continue
        parts = line.split("\t")
        if len(parts) == 7:
            domain, flag, cpath, secure, expiry, name, value = parts
            rows[((domain[10:] if domain.startswith("#HttpOnly_") else domain), cpath, name)] = tuple(parts)
    return rows


def write_netscape(path: Path, rows: dict[CookieKey, CookieRow]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = ["# Netscape HTTP Cookie File", "# Refreshed for yt-dlp"]
    lines.extend("\t".join(row) for _, row in sorted(rows.items()))
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=path.parent, delete=False) as f:
            temporary = Path(f.name)
            f.write("\n".join(lines) + "\n")
            f.flush()
            os.fsync(f.fileno())
        temporary.replace(path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def merge_document_cookie(cookie_str: str, existing: Path, token: str = "") -> int:
    if not isinstance(cookie_str, str) or not isinstance(token, str):
        raise ValueError("cookie 和 token 必须是字符串")
    rows = parse_netscape(existing)
    expiry = str(int(time.time()) + 86400 * 30)
    updates = []
    for part in cookie_str.split(";"):
        if "=" in part:
            name, value = part.strip().split("=", 1)
            if name and name != "douyin.com":
                updates.append((name, value))
    if token.strip():
        updates.append(("msToken", token.strip()))
    updated = 0
    for name, value in updates:
        if any(c in name + value for c in "\r\n\t"):
            raise ValueError("cookie 含非法控制字符")
        key = (DOMAIN, "/", name)
        old = rows.get(key)
        # document.cookie 无法提供 HttpOnly 项，不用同名脚本可见项覆盖它。
        if old and old[0].startswith("#HttpOnly_"):
            continue
        rows[key] = (DOMAIN, "TRUE", "/", old[3] if old else "FALSE", expiry, name, value)
        updated += 1
    write_netscape(existing, rows)
    return updated


def merge_ms_token(token: str, existing: Path = OUT) -> None:
    if token.strip():
        merge_document_cookie("", existing, token)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cookie", help="document.cookie 字符串；推荐用 from-json 入口避免命令行传密钥")
    parser.add_argument("token", nargs="?", default="")
    parser.add_argument("--output", type=Path, default=OUT)
    args = parser.parse_args()
    n = merge_document_cookie(args.cookie, args.output, args.token)
    print(f"updated {n} cookies -> {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
