#!/usr/bin/env python3
"""whisper-cli SRT -> 仓库 Transcript JSON（复刻 avr_core::timeline::parse_srt；source=asr，同 ASR 管线）。
附 _meta：模型名、模型内容指纹（同 avr_core::fingerprint::content_fingerprint：大小 + 头尾各 1MB xxh3_64）、sha256。
用法: srt_to_transcript.py <srt> <lang> <model.bin> <whisper_cpp_commit> <out.json>
"""
import hashlib, json, re, sys, xxhash
CH = 1 << 20
TS = re.compile(r"(\d{2}:\d{2}:\d{2}[,.]\d{3})\s*-->\s*(\d{2}:\d{2}:\d{2}[,.]\d{3})")
def ts(s):
    h, m, r = s.replace(",", ".").split(":"); return int(h)*3600 + int(m)*60 + float(r)
def content_fp(p):
    import os; n = os.path.getsize(p); h = xxhash.xxh3_64()
    with open(p, "rb") as f:
        h.update(f.read(CH))
        if n > CH:
            f.seek(max(n - CH, min(CH, n))); h.update(f.read(CH))
    return f"{n}-{h.intdigest():016x}"
def sha(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for b in iter(lambda: f.read(1 << 22), b""): h.update(b)
    return h.hexdigest()
srt, lang, model, commit, out = sys.argv[1:6]
segs = []
for block in open(srt, encoding="utf-8").read().strip().split("\n\n"):
    ls = block.splitlines()
    if len(ls) < 2: continue
    tl, tx = (ls[1], ls[2:]) if ls[0].isdigit() else (ls[0], ls[1:])
    m = TS.search(tl); t = " ".join(tx).strip()
    if m and t: segs.append({"start": ts(m[1]), "end": ts(m[2]), "text": t})
json.dump({"language": lang, "source": "asr", "segments": segs,
           "_meta": {"engine": f"whisper.cpp {commit} (whisper-cli, CPU 8 threads, args = preset whisper-cpp-cpu)",
                     "model": model.rsplit("/", 1)[-1], "model_content_fingerprint": content_fp(model),
                     "model_sha256": sha(model)}}, open(out, "w"), ensure_ascii=False, indent=1)
print(out, len(segs))
