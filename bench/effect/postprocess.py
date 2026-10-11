#!/usr/bin/env python3
"""对 Transcript JSON 应用与 avr_core::asr::postprocess 相同的规则（离线对比用）。
用法: postprocess.py <in.json> <out.json>；打印 段数、重复段比例、疑似幻觉数。"""
import json, re, sys
MIN_SECS, MAX_UPS = 8.0, 0.4
CJK = re.compile(r"[\u3400-\u9fff\uf900-\ufaff]")
def norm(t): return "".join(c.lower() for c in t if c.isalnum())
def units(t):
    n = len(CJK.findall(t)); t = CJK.sub(" ", t)
    return n + len(re.findall(r"[^\W_]+(?:'[^\W_]+)*", t))
def stats(segs):
    n = len(segs); dup = sum(1 for a, b in zip(segs, segs[1:]) if norm(a["text"]) and norm(a["text"]) == norm(b["text"]))
    return n, dup
def pp(segs):
    """不删内容：只打 suspect = repeat / duration_mismatch。"""
    out, prev = [], ""
    for s in segs:
        s = dict(s); k = norm(s["text"]); d = s["end"] - s["start"]
        s.pop("suspect", None)
        if k and k == prev: s["suspect"] = "repeat"
        elif d >= MIN_SECS and units(s["text"]) / d < MAX_UPS: s["suspect"] = "duration_mismatch"
        prev = k; out.append(s)
    return out
if __name__ == "__main__":
    d = json.load(open(sys.argv[1])); before = stats(d["segments"])
    d["segments"] = pp(d["segments"]); d.setdefault("_meta", {})["postprocess"] = "P4-3"
    json.dump(d, open(sys.argv[2], "w"), ensure_ascii=False, indent=1)
    n, dup = before; sus = sum(1 for s in d["segments"] if s.get("suspect"))
    import collections
    c = collections.Counter(s.get("suspect") for s in d["segments"])
    print(f"{n} 句；repeat {c['repeat']} ({c['repeat']/max(n,1):.0%})；duration_mismatch {c['duration_mismatch']}；索引可见 {c[None]} 句")
    for s in d["segments"]:
        if s.get("suspect"): print(f"  {s['start']:7.1f}-{s['end']:7.1f} {s['suspect']:17s} {s['text']}")
