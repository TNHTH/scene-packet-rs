#!/usr/bin/env python
"""从 JSON 文件读取 cookie+xmst 并刷新 Netscape cookies.txt。"""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

SCRIPT = Path(__file__).with_name("refresh-douyin-cookies.py")


def main() -> int:
    if len(sys.argv) < 2:
        print("用法: python refresh-douyin-from-json.py <cookie.json>", file=sys.stderr)
        print("JSON 字段: cookie (必填), xmst 或 msToken (推荐)", file=sys.stderr)
        return 1
    data = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    cookie = data.get("cookie") or data.get("document_cookie")
    if not cookie:
        print("JSON 缺少 cookie 字段", file=sys.stderr)
        return 1
    args = [sys.executable, str(SCRIPT), cookie]
    xmst = data.get("xmst") or data.get("msToken")
    if xmst:
        args.append(xmst)
    r = subprocess.run(args)
    return r.returncode

if __name__ == "__main__":
    sys.exit(main())
