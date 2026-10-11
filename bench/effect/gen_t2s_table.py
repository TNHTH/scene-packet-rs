#!/usr/bin/env python3
"""从 OpenCC TSCharacters.txt 生成 crates/avr-core/src/asr/t2s_table.rs（一对一单字；多候选取第一个，
一对多的上下文例外在 zh.rs 里处理）。用法: gen_t2s_table.py <TSCharacters.txt>"""
import sys
pairs = {}
for l in open(sys.argv[1], encoding="utf-8"):
    l = l.rstrip("\n")
    if "\t" not in l: continue
    k, v = l.split("\t", 1); v = v.split()[0]
    if len(k) == 1 and len(v) == 1 and k != v: pairs[k] = v
items = sorted(pairs.items(), key=lambda kv: ord(kv[0]))
body = "".join(f"    ('{k}', '{v}'),\n" for k, v in items)
open("crates/avr-core/src/asr/t2s_table.rs", "w").write(f"""//! 繁→简单字映射表（自动生成，勿手改；`bench/effect/gen_t2s_table.py`）。
//!
//! 来源：OpenCC `TSCharacters.txt`（Apache License 2.0，© OpenCC 贡献者），
//! 一对一单字条目，多候选取第一个，共 {len(items)} 条。

pub(super) static T2S: &[(char, char)] = &[
{body}];
""")
