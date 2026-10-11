//! ASR 输出后处理（P4-3）：抑制 whisper 的幻觉重复，不依赖 whisper 版本或参数。
//!
//! 1. **合并连续重复**：相邻两句规范化后（去首尾空白与标点、小写）文本相同，则合并为一句，
//!    时间取并集。whisper 在音乐 / 静音段常把同一句重复几十次。
//! 2. **疑似幻觉标记**：句子时长 ≥ [`SUSPECT_MIN_SECS`] 且语速低于
//!    [`SUSPECT_MAX_UNITS_PER_SEC`]（字数 / 秒）时，标 `suspect = true`。
//!    字数：每个 CJK 字算 1，每个拉丁词 / 数字串算 1。正常语速约为中文 3–5 字/秒、英文 2–3 词/秒，
//!    阈值 0.4 只拦“十几秒只有一两个词”这类明显不匹配的句子。只标记不删除，由下游决定是否展示。

use crate::models::TranscriptSegment;

/// 疑似幻觉判定的最短时长（秒）。短句（如 1 秒的 “No!”）不判。
pub const SUSPECT_MIN_SECS: f64 = 8.0;
/// 疑似幻觉判定的语速上限（字数/秒）。
pub const SUSPECT_MAX_UNITS_PER_SEC: f64 = 0.4;

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F)
}

/// 字数：CJK 字各算 1，连续的其他字母数字算 1 个词。
pub fn speech_units(s: &str) -> usize {
    let mut n = 0;
    let mut in_word = false;
    for c in s.chars() {
        if is_cjk(c) {
            n += 1;
            in_word = false;
        } else if c.is_alphanumeric() {
            if !in_word {
                n += 1;
            }
            in_word = true;
        } else if c != '\'' {
            in_word = false;
        }
    }
    n
}

fn suspect(seg: &TranscriptSegment) -> bool {
    let dur = seg.end - seg.start;
    dur >= SUSPECT_MIN_SECS && (speech_units(&seg.text) as f64) / dur < SUSPECT_MAX_UNITS_PER_SEC
}

/// 合并连续重复并标记疑似幻觉。
pub fn postprocess(segments: Vec<TranscriptSegment>) -> Vec<TranscriptSegment> {
    let mut out: Vec<TranscriptSegment> = Vec::with_capacity(segments.len());
    for seg in segments {
        if let Some(last) = out.last_mut() {
            let k = norm(&seg.text);
            if !k.is_empty() && k == norm(&last.text) {
                last.start = last.start.min(seg.start);
                last.end = last.end.max(seg.end);
                continue;
            }
        }
        out.push(seg);
    }
    for seg in &mut out {
        seg.suspect = suspect(seg);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(start: f64, end: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start,
            end,
            text: text.into(),
            suspect: false,
        }
    }

    #[test]
    fn merges_consecutive_duplicates_only() {
        let out = postprocess(vec![
            s(0.0, 1.0, "No!"),
            s(1.0, 2.0, "no"),
            s(2.0, 3.0, "No!"),
            s(3.0, 4.0, "I know."),
            s(4.0, 5.0, "No!"),
        ]);
        let texts: Vec<_> = out.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, ["No!", "I know.", "No!"]);
        assert_eq!((out[0].start, out[0].end), (0.0, 3.0));
    }

    #[test]
    fn flags_long_low_density_segments() {
        let out = postprocess(vec![
            s(0.0, 22.8, "This is pretty freaky."), // 4 词 / 22.8 秒
            s(23.0, 26.0, "Look, Celia, we have to follow our passions."),
            s(30.0, 31.0, "No!"),                          // 短句不判
            s(40.0, 50.0, "首先我们将发射空间站的核心舱"), // 14 字 / 10 秒，正常
        ]);
        let flags: Vec<_> = out.iter().map(|x| x.suspect).collect();
        assert_eq!(flags, [true, false, false, false]);
    }

    #[test]
    fn merged_repeat_loop_becomes_suspect() {
        let segs = (0..15)
            .map(|i| s(450.0 + i as f64 * 6.0, 456.0 + i as f64 * 6.0, "No!"))
            .collect();
        let out = postprocess(segs);
        assert_eq!(out.len(), 1);
        assert!(out[0].suspect);
    }

    #[test]
    fn units_count_cjk_chars_and_words() {
        assert_eq!(speech_units("You could've gone worse."), 4);
        assert_eq!(speech_units("核心舱 2021 年"), 5);
        assert_eq!(speech_units("[Music]"), 1);
    }

    #[test]
    fn suspect_false_is_not_serialized() {
        let j = serde_json::to_string(&s(0.0, 1.0, "a")).unwrap();
        assert!(!j.contains("suspect"));
        let back: TranscriptSegment =
            serde_json::from_str(r#"{"start":0,"end":1,"text":"a"}"#).unwrap();
        assert!(!back.suspect);
    }
}
