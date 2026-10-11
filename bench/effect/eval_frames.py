"""P5-4 选帧覆盖率评测。

用法:
  python eval_frames.py questions/p5_visual.json \
      --frames board_leibniz=kf_board.json --frames slides_chaos=kf_slides.json [--tol 0]

输入
- 标注文件（知识管理员维护，格式如下）：
  {
    "<video_id>": {
      "questions": [
        {"id": "b2", "question": "...", "answer": "...",
         "visible_from": 204, "visible_to": 212}     # 答案在画面上完整可见的区间（秒，闭区间）
      ],
      ...                                            # 其他字段（boards / slides / note）忽略
    },
    "note": "..."                                   # 非 dict 的顶层字段忽略
  }
- 选帧结果：`cargo run -p avr-core --example keyframes -- <video>` 的输出 JSON
  （需含 "frames": [{"time": 秒, ...}] 与 "duration"；没有 duration 时用 --duration id=秒）。

判定：某题只要有一张选中帧的 time 落在 [visible_from - tol, visible_to + tol] 就算“覆盖”。
这只是时间上的必要条件；答案是否被遮挡由后续视觉模型判定，这里不管。

输出：每题是否覆盖及命中帧时间、每段视频覆盖率、帧数与每小时帧数、合计。--json 输出机器可读结果。
"""
import argparse, json, sys
from pathlib import Path


def load_frames(p):
    d = json.loads(Path(p).read_text())
    frames = d.get("frames", d if isinstance(d, list) else [])
    return [float(f["time"]) for f in frames], d.get("duration")


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("questions")
    ap.add_argument("--frames", action="append", default=[], help="video_id=keyframes.json")
    ap.add_argument("--duration", action="append", default=[], help="video_id=秒（覆盖 JSON 里的 duration）")
    ap.add_argument("--tol", type=float, default=0.0, help="区间两端容差（秒），默认 0")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args(argv)
    q = json.loads(Path(a.questions).read_text())
    durs = {k: float(v) for k, v in (x.split("=", 1) for x in a.duration)}
    out = {"videos": {}, "tol": a.tol}
    tot_hit = tot_q = tot_frames = 0
    tot_secs = 0.0
    for spec in a.frames:
        vid, path = spec.split("=", 1)
        times, dur = load_frames(path)
        dur = durs.get(vid, dur)
        qs = q.get(vid, {}).get("questions", []) if isinstance(q.get(vid), dict) else []
        rows = []
        for item in qs:
            lo, hi = item["visible_from"] - a.tol, item["visible_to"] + a.tol
            hits = [t for t in times if lo <= t <= hi]
            rows.append({"id": item["id"], "from": item["visible_from"], "to": item["visible_to"], "covered": bool(hits), "frames": hits})
        hit = sum(r["covered"] for r in rows)
        fph = len(times) / (dur / 3600) if dur else None
        out["videos"][vid] = {"questions": rows, "covered": hit, "total": len(rows), "frames": len(times), "duration": dur, "frames_per_hour": fph}
        tot_hit += hit; tot_q += len(rows); tot_frames += len(times); tot_secs += dur or 0
    out["covered"], out["total"], out["frames"] = tot_hit, tot_q, tot_frames
    out["frames_per_hour"] = tot_frames / (tot_secs / 3600) if tot_secs else None
    if a.json:
        print(json.dumps(out, ensure_ascii=False, indent=2)); return out
    for vid, v in out["videos"].items():
        fph = f"{v['frames_per_hour']:.0f}" if v["frames_per_hour"] else "?"
        print(f"{vid}: 覆盖 {v['covered']}/{v['total']}，{v['frames']} 帧，时长 {v['duration']} 秒，每小时 {fph} 帧")
        for r in v["questions"]:
            mark = "✓" if r["covered"] else "✗"
            print(f"  {mark} {r['id']} [{r['from']}, {r['to']}] 命中帧 {r['frames']}")
    print(f"合计：覆盖 {tot_hit}/{tot_q}，{tot_frames} 帧，每小时 {out['frames_per_hour'] or 0:.0f} 帧")
    return out


if __name__ == "__main__":
    main()
