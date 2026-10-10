#!/usr/bin/env python3
"""P3 效果集：索引模式“只看概要能否定位到正确的段”评测。

用法:
  python3 bench/effect/eval_index.py --set /workspace/avr-effect-set \
      [--questions questions.json] [--boot 2000] [--json out.json]

输入:
  <set>/manifest.json, <set>/transcripts/<id>.json（仓库 Transcript JSON）,
  questions.json: [{video_id, question, answer_start, answer_end, kind: speech|visual}]
规则（写死，出结果前不得修改）:
  - 分段：精确复刻 crates/avr-mcp/src/index.rs::sections
    （SECTION_MAX_SEGMENTS=20, SECTION_MAX_SECS=60；新句满足 count<20 且 seg.end-section.start<=60 则并入，否则开新段）。
  - 概要方法只换生成方式：A 首句前 40 字(+…，复刻 summarize)；B 每段 TF-IDF top-8 词元
    （以视频内各段为文档集，词元同匹配器）；C 段内头、中、尾各一句拼接。
    UPPER = 段全文（上限）。
  - 匹配器：BM25(k1=1.2,b=0.75)，中文（CJK）按字二元组，英文/数字按词小写。分数并列按段序。
  - 正确：段时间区间与 [answer_start, answer_end] 有正重叠。
  - 段数 < 6 的视频：主指标改报 hit@1（标注 LOW_N）。visual 题单独统计，不计入方法比较。
"""
import argparse, json, math, random, re, sys
from collections import Counter

SECTION_MAX_SECS, SECTION_MAX_SEGMENTS, SUMMARY_CHARS = 60.0, 20, 40
CJK = re.compile(r"[\u3400-\u9fff\uf900-\ufaff]+")
WORD = re.compile(r"[a-z0-9]+(?:'[a-z]+)?")
TOPK = 8

def tokens(text):
    out = []
    low = text.lower()
    for m in re.finditer(r"[\u3400-\u9fff\uf900-\ufaff]+|[a-z0-9]+(?:'[a-z]+)?", low):
        t = m.group(0)
        if CJK.fullmatch(t):
            out += [t[i:i+2] for i in range(len(t)-1)] if len(t) > 1 else [t]
        else:
            out.append(t)
    return out

def sections(segs):
    out = []
    for i, s in enumerate(segs):
        if out and out[-1]["n"] < SECTION_MAX_SEGMENTS and s["end"] - out[-1]["start"] <= SECTION_MAX_SECS:
            out[-1]["n"] += 1; out[-1]["end"] = max(out[-1]["end"], s["end"])
        else:
            out.append({"start": s["start"], "end": s["end"], "first": i, "n": 1})
    for sec in out:
        sec["texts"] = [segs[j]["text"].strip() for j in range(sec["first"], sec["first"] + sec["n"])]
    return out

def summ_a(sec):
    t = sec["texts"][0]
    return t[:SUMMARY_CHARS] + ("…" if len(t) > SUMMARY_CHARS else "")

def summ_c(sec):
    t = sec["texts"]; idx = sorted({0, len(t)//2, len(t)-1})
    return " ".join(t[i] for i in idx)

def summ_b_all(secs):
    docs = [Counter(tokens(" ".join(s["texts"]))) for s in secs]
    N = len(docs); df = Counter(w for d in docs for w in d)
    res = []
    for d in docs:
        tot = sum(d.values()) or 1
        sc = {w: (c/tot) * math.log((1+N)/(1+df[w]) + 1) for w, c in d.items()}
        top = sorted(sc, key=lambda w: (-sc[w], w))[:TOPK]
        res.append(" ".join(top))
    return res

def bm25_rank(query, docs, k1=1.2, b=0.75):
    dt = [tokens(d) for d in docs]; N = len(dt)
    avg = sum(map(len, dt)) / N if N else 0
    df = Counter(w for d in dt for w in set(d))
    q = tokens(query); scores = []
    for i, d in enumerate(dt):
        tf = Counter(d); s = 0.0
        for w in q:
            if w in tf:
                idf = math.log(1 + (N - df[w] + .5) / (df[w] + .5))
                s += idf * tf[w] * (k1+1) / (tf[w] + k1 * (1 - b + b * len(d) / (avg or 1)))
        scores.append(s)
    return sorted(range(N), key=lambda i: (-scores[i], i))

def overlaps(sec, q):
    return min(sec["end"], q["answer_end"]) - max(sec["start"], q["answer_start"]) > 0

METHODS = ["A_first40", "B_tfidf", "C_headmidtail", "UPPER_fulltext"]

def boot_ci(xs, B, rng):
    if not xs: return (float("nan"),)*2
    m = sorted(sum(rng.choice(xs) for _ in xs)/len(xs) for _ in range(B))
    return m[int(.025*B)], m[int(.975*B)-1]

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--set", default="/workspace/avr-effect-set")
    ap.add_argument("--questions"); ap.add_argument("--boot", type=int, default=2000)
    ap.add_argument("--json")
    a = ap.parse_args()
    man = json.load(open(f"{a.set}/manifest.json"))
    qs = json.load(open(a.questions or f"{a.set}/questions.json"))
    vids = {}
    print("== 视频 ==")
    for v in man["videos"]:
        tr = json.load(open(f"{a.set}/transcripts/{v['id']}.json"))
        secs = sections(tr["segments"])
        summ = {"A_first40": [summ_a(s) for s in secs], "B_tfidf": summ_b_all(secs),
                "C_headmidtail": [summ_c(s) for s in secs],
                "UPPER_fulltext": [" ".join(s["texts"]) for s in secs]}
        n = len(secs); low = n < 6
        vids[v["id"]] = dict(secs=secs, summ=summ, low=low)
        print(f"{v['id']:10s} 句数={len(tr['segments']):3d} 段数={n:2d} 随机hit@1={1/n:.2f} hit@2={min(2,n)/n:.2f}"
              + ("  [LOW_N<6 → 主指标 hit@1]" if low else ""))
    rows = []
    for q in qs:
        v = vids.get(q["video_id"])
        if v is None: print(f"跳过未知视频 {q['video_id']}", file=sys.stderr); continue
        secs = v["secs"]; n = len(secs)
        gold = [i for i, s in enumerate(secs) if overlaps(s, q)]
        # 随机基线：随机排序时前 k 含任一正确段的概率
        def rand_hit(k):
            g = len(gold); k = min(k, n)
            return 1 - math.comb(n - g, k) / math.comb(n, k) if g else 0.0
        r = dict(video=q["video_id"], kind=q.get("kind", "speech"), q=q["question"], gold=gold,
                 low=v["low"], rand1=rand_hit(1), rand2=rand_hit(2))
        for m in METHODS:
            rank = bm25_rank(q["question"], v["summ"][m])
            r[m] = dict(rank=rank[:3], h1=float(rank[0] in gold), h2=float(bool(set(rank[:2]) & set(gold))))
        rows.append(r)
    rng = random.Random(0)
    report = {}
    for kind in ("speech", "visual"):
        rs = [r for r in rows if r["kind"] == kind]
        print(f"\n== {kind} 题 n={len(rs)}" + ("（不计入概要方法比较；索引无画面描述，已知缺陷）" if kind == "visual" else "") + " ==")
        if not rs: continue
        # 主指标：LOW_N 视频用 hit@1，其余 hit@2
        def prim(r, m): return r[m]["h1"] if r["low"] else r[m]["h2"]
        rand_p = sum(r["rand1"] if r["low"] else r["rand2"] for r in rs) / len(rs)
        rand1 = sum(r["rand1"] for r in rs)/len(rs); rand2 = sum(r["rand2"] for r in rs)/len(rs)
        print(f"随机: hit@1={rand1:.2f} hit@2={rand2:.2f} 主指标={rand_p:.2f}")
        for m in METHODS:
            h1 = [r[m]["h1"] for r in rs]; h2 = [r[m]["h2"] for r in rs]; p = [prim(r, m) for r in rs]
            mp = sum(p)/len(p); lo, hi = boot_ci(p, a.boot, rng)
            print(f"{m:15s} hit@1={sum(h1)/len(h1):.2f} hit@2={sum(h2)/len(h2):.2f} "
                  f"主指标={mp:.2f} [{lo:.2f},{hi:.2f}] 比随机 {mp-rand_p:+.2f}")
            report.setdefault(kind, {})[m] = dict(hit1=sum(h1)/len(h1), hit2=sum(h2)/len(h2), primary=mp, ci=[lo, hi], lift=mp-rand_p)
        for vid in sorted({r["video"] for r in rs}):
            vr = [r for r in rs if r["video"] == vid]
            print(f"  {vid}: n={len(vr)} " + " ".join(f"{m[:1] if m[0]!='U' else 'U'}={sum(prim(r,m) for r in vr)/len(vr):.2f}" for m in METHODS))
    if a.json:
        json.dump(dict(rows=rows, report=report), open(a.json, "w"), ensure_ascii=False, indent=1)

if __name__ == "__main__":
    main()
