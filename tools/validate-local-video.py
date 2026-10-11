#!/usr/bin/env python3
"""校验 packet 图片、离线 HTML、Markdown 引用、时间覆盖与来源 hash。"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import re
import subprocess


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def validate(directory, source=None, ffprobe=None):
    manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
    frames = manifest["frames"]
    assert frames, "没有截图"
    times = [f["time"] for f in frames]
    assert times == sorted(set(times)), "时间重复或未排序"
    assert all(0 <= t < manifest["duration"] for t in times), "时间超界"
    markdown = (directory / "README.md").read_text(encoding="utf-8")
    html = (directory / "index.html").read_text(encoding="utf-8")
    embedded = re.findall(r"src='data:image/jpeg;base64,([^']+)'", html)
    assert len(embedded) == len(frames), "离线 HTML 图片数量不符"
    root = directory.resolve()
    for frame, encoded in zip(frames, embedded):
        path = (directory / frame["path"]).resolve()
        assert root in path.parents, "图片路径越界"
        assert digest(path) == frame["sha256"], "图片 hash 不符"
        assert base64.b64decode(encoded, validate=True) == path.read_bytes(), "内嵌图片不符"
        assert "](" + frame["path"] + ")" in markdown, "Markdown 引用缺失"
        assert frame["width"] > 0 and frame["height"] > 0, "尺寸无效"
        if ffprobe:
            data = json.loads(
                subprocess.check_output(
                    [
                        ffprobe,
                        "-v",
                        "error",
                        "-show_entries",
                        "stream=width,height",
                        "-of",
                        "json",
                        str(path),
                    ]
                )
            )
            assert data["streams"][0]["width"] == frame["width"]
            assert data["streams"][0]["height"] == frame["height"]
    assert manifest["platform"] == "local"
    assert manifest["coverage"]["frames"] == len(frames)
    assert manifest["coverage"]["first"] == times[0] and manifest["coverage"]["last"] == times[-1]
    if source:
        assert digest(source) == manifest["source_sha256"], "原视频 hash 不符"
    return {
        "valid": True,
        "frames": len(frames),
        "first": times[0],
        "last": times[-1],
        "duration": manifest["duration"],
        "max_gap_seconds": manifest["coverage"]["max_gap_seconds"],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--source", type=Path)
    parser.add_argument("--ffprobe", help="使用真实解码器额外核对图片尺寸")
    args = parser.parse_args()
    print(json.dumps(validate(args.directory, args.source, args.ffprobe), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
