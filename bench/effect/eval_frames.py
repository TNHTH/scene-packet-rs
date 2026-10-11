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

分组：每题按边界精度归入“精确题”（exact）或“±10 秒题”（coarse）。依次看字段
`boundary_tol`/`tol`（秒，≤2 为 exact，否则 coarse）、`precision`（含 "10"/"coarse"/"approx" 为 coarse）、
`group`（直接给 exact/coarse）；都没有则算 exact。

--budget N：只取按内容分数（选帧 JSON 里 frames[].content，即产品 content_score：
64×36 灰度采样中边缘像素占比）降序的前 N 帧算覆盖率（模拟每小时名额）。

判定：某题只要有一张选中帧的 time 落在 [visible_from - tol, visible_to + tol] 就算“覆盖”。
这只是时间上的必要条件；答案是否被遮挡由后续视觉模型判定，这里不管。

输出：每题是否覆盖及命中帧时间、每段视频覆盖率、帧数与每小时帧数、合计。--json 输出机器可读结果。
"""
import argparse, json, sys
from pathlib import Path


def load_frames(p, budget=None):
    d = json.loads(Path(p).read_text())
    frames = d.get("frames", d if isinstance(d, list) else [])
    if budget is not None:
        if any("content" not in f for f in frames):
            sys.exit(f"{p}: 帧缺 content 分数，无法 --budget")
        frames = sorted(frames, key=lambda f: -float(f["content"]))[:budget]
    return sorted(float(f["time"]) for f in frames), d.get("duration")


def group_of(item):
    for k in ("g", "boundary_tol", "tol"):
        if isinstance(item.get(k), (int, float)):
            return "exact" if item[k] <= 2 else "coarse"
    p = str(item.get("precision", "")).lower()
    if p:
        return "coarse" if any(x in p for x in ("10", "coarse", "approx")) else "exact"
    g = str(item.get("group", "")).lower()
    return "coarse" if g == "coarse" else "exact"


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("questions")
    ap.add_argument("--frames", action="append", default=[], help="video_id=keyframes.json")
    ap.add_argument("--duration", action="append", default=[], help="video_id=秒（覆盖 JSON 里的 duration）")
    ap.add_argument("--tol", type=float, default=0.0, help="区间两端容差（秒），默认 0")
    ap.add_argument("--budget", type=int, default=None, help="只取内容分数最高的前 N 帧")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args(argv)
    q = json.loads(Path(a.questions).read_text())
    # 单视频格式：{"video": "x.webm", "questions": [...]}（visible: [[from, to], ...]，g: 2|10）
    if isinstance(q.get("questions"), list) and "video" in q:
        q = {Path(q["video"]).stem: {"questions": q["questions"]}}
    durs = {k: float(v) for k, v in (x.split("=", 1) for x in a.duration)}
    out = {"videos": {}, "tol": a.tol}
    tot_hit = tot_q = tot_frames = 0
    tot_secs = 0.0
    for spec in a.frames:
        vid, path = spec.split("=", 1)
        times, dur = load_frames(path, a.budget)
        dur = durs.get(vid, dur)
        qs = q.get(vid, {}).get("questions", []) if isinstance(q.get(vid), dict) else []
        rows = []
        for item in qs:
            spans = item.get("visible") or [[item["visible_from"], item["visible_to"]]]
            hits = [t for t in times if any(lo - a.tol <= t <= hi + a.tol for lo, hi in spans)]
            rows.append({"id": item["id"], "group": group_of(item), "spans": spans, "covered": bool(hits), "frames": hits})
        hit = sum(r["covered"] for r in rows)
        fph = len(times) / (dur / 3600) if dur else None
        groups = {g: [sum(r["covered"] for r in rows if r["group"] == g), sum(r["group"] == g for r in rows)] for g in ("exact", "coarse")}
        out["videos"][vid] = {"groups": groups, "questions": rows, "covered": hit, "total": len(rows), "frames": len(times), "duration": dur, "frames_per_hour": fph}
        tot_hit += hit; tot_q += len(rows); tot_frames += len(times); tot_secs += dur or 0
    out["covered"], out["total"], out["frames"], out["budget"] = tot_hit, tot_q, tot_frames, a.budget
    out["groups"] = {g: [sum(v["groups"][g][i] for v in out["videos"].values()) for i in (0, 1)] for g in ("exact", "coarse")}
    out["frames_per_hour"] = tot_frames / (tot_secs / 3600) if tot_secs else None
    if a.json:
        print(json.dumps(out, ensure_ascii=False, indent=2)); return out
    for vid, v in out["videos"].items():
        fph = f"{v['frames_per_hour']:.0f}" if v["frames_per_hour"] else "?"
        g = v["groups"]
        print(f"{vid}: 覆盖 {v['covered']}/{v['total']}（精确题 {g['exact'][0]}/{g['exact'][1]}，±10 秒题 {g['coarse'][0]}/{g['coarse'][1]}），{v['frames']} 帧，时长 {v['duration']} 秒，每小时 {fph} 帧")
        for r in v["questions"]:
            mark = "✓" if r["covered"] else "✗"
            print(f"  {mark} {r['id']} ({'精确' if r['group'] == 'exact' else '±10s'}) {r['spans']} 命中帧 {r['frames']}")
    g = out["groups"]
    print(f"合计{'（预算前 ' + str(a.budget) + ' 帧）' if a.budget else ''}：覆盖 {tot_hit}/{tot_q}（精确 {g['exact'][0]}/{g['exact'][1]}，±10 秒 {g['coarse'][0]}/{g['coarse'][1]}），{tot_frames} 帧，每小时 {out['frames_per_hour'] or 0:.0f} 帧")
    return out


if __name__ == "__main__":
    main()
