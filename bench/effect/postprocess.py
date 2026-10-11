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
    out = []
    for s in segs:
        if out and norm(s["text"]) and norm(s["text"]) == norm(out[-1]["text"]):
            out[-1]["start"] = min(out[-1]["start"], s["start"]); out[-1]["end"] = max(out[-1]["end"], s["end"]); continue
        out.append(dict(s))
    for s in out:
        d = s["end"] - s["start"]
        if d >= MIN_SECS and units(s["text"]) / d < MAX_UPS: s["suspect"] = True
    return out
if __name__ == "__main__":
    d = json.load(open(sys.argv[1])); before = stats(d["segments"])
    d["segments"] = pp(d["segments"]); d.setdefault("_meta", {})["postprocess"] = "P4-3"
    json.dump(d, open(sys.argv[2], "w"), ensure_ascii=False, indent=1)
    n, dup = before; sus = sum(1 for s in d["segments"] if s.get("suspect"))
    print(f"before: {n} 句, 与上一句重复 {dup} ({dup/max(n,1):.0%}); after: {len(d['segments'])} 句, suspect {sus}")
