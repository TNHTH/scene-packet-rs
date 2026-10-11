//! 对 whisper 转写 JSON（Transcript 格式）跑产品的转写后处理（zh 繁→简 + P4-3 标记），用于效果集复现。
//! 用法: cargo run -p avr-core --example asr_finish -- <in.json> <out.json>
use avr_core::models::TranscriptSegment;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&a[1]).expect("read")).expect("json");
    let lang = v["language"].as_str().unwrap_or("").to_string();
    let segs: Vec<TranscriptSegment> =
        serde_json::from_value(v["segments"].clone()).expect("segments");
    let out = avr_core::asr::engine::finish(segs, &lang);
    v["segments"] = serde_json::to_value(out).expect("ser");
    std::fs::write(&a[2], serde_json::to_string_pretty(&v).expect("ser")).expect("write");
}
