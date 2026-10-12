#!/usr/bin/env python
"""从 JSON 文件读取 cookie+xmst 并刷新 Netscape 文件，不经子进程参数传递密钥。"""
from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path

SCRIPT = Path(__file__).with_name("refresh-douyin-cookies.py")


def main() -> int:
    spec = importlib.util.spec_from_file_location("douyin_cookies", SCRIPT)
    cookies = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(cookies)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--output", type=Path, default=cookies.OUT)
    args = parser.parse_args()
    try:
        data = json.loads(args.source.read_text(encoding="utf-8"))
        if not isinstance(data, dict):
            raise ValueError()
        cookie = data.get("cookie") or data.get("document_cookie")
        token = data.get("xmst") or data.get("msToken") or ""
        if not isinstance(cookie, str) or not cookie or not isinstance(token, str):
            raise ValueError()
        count = cookies.merge_document_cookie(cookie, args.output, token)
    except (OSError, ValueError):
        parser.exit(1, "无法刷新：检查 JSON 字段类型、cookie 格式和文件权限。\n")
    print(f"updated {count} cookies -> {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
