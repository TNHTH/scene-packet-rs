//! ASR 输出后处理（P4-3）：标记 whisper 的疑似幻觉，不依赖 whisper 版本或参数。
//!
//! **不删内容**：每一句原文和原时间范围都保留，只在 `suspect` 上打标记和原因；
//! compact 与索引默认跳过带标记的句，full / `read_transcript` / 缓存照样有原文，误判可找回。
//!
//! 1. `repeat`：与上一句规范化后（只留字母数字、小写）文本相同。一串连续重复只保留第一句不打标，
//!    其余各句保持自己的时间范围、标 `repeat`（相当于“合并”但不丢原时间）。
//! 2. `duration_mismatch`：时长 ≥ [`SUSPECT_MIN_SECS`] 且语速 < [`SUSPECT_MAX_UNITS_PER_SEC`] 字/秒。
//!    字数：每个 CJK 字算 1，每个拉丁词 / 数字串算 1。正常语速约中文 3–5 字/秒、英文 2–3 词/秒，
//!    阈值只拦“十几秒只有一两个词”这类明显不匹配的句子。注意：whisper 把真实台词的时间戳拉长到
//!    后面的静音 / 音乐里时也会命中，所以这一类只是“疑似”，需要人工对照。
//!
//! 两者同时成立时记 `repeat`。

use crate::models::{SuspectReason, TranscriptSegment};

/// 时长不匹配判定的最短时长（秒）。短句（如 1 秒的 “No!”）不判。
pub const SUSPECT_MIN_SECS: f64 = 8.0;
/// 时长不匹配判定的语速上限（字数/秒）。
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

/// 字数：CJK 字各算 1，连续的其他字母数字（可含撇号）算 1 个词。
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

fn duration_mismatch(seg: &TranscriptSegment) -> bool {
    let dur = seg.end - seg.start;
    dur >= SUSPECT_MIN_SECS && (speech_units(&seg.text) as f64) / dur < SUSPECT_MAX_UNITS_PER_SEC
}

/// 给疑似幻觉句打标记，不改动文本、时间与句数。
pub fn postprocess(mut segments: Vec<TranscriptSegment>) -> Vec<TranscriptSegment> {
    let mut prev = String::new();
    for seg in &mut segments {
        let k = norm(&seg.text);
        seg.suspect = if !k.is_empty() && k == prev {
            Some(SuspectReason::Repeat)
        } else if duration_mismatch(seg) {
            Some(SuspectReason::DurationMismatch)
        } else {
            None
        };
        prev = k;
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use SuspectReason::*;

    fn s(start: f64, end: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start,
            end,
            text: text.into(),
            suspect: None,
        }
    }

    fn reasons(v: &[TranscriptSegment]) -> Vec<Option<SuspectReason>> {
        v.iter().map(|x| x.suspect).collect()
    }

    #[test]
    fn marks_consecutive_repeats_and_keeps_everything() {
        let input = vec![
            s(0.0, 1.0, "No!"),
            s(1.0, 2.0, "no"),
            s(2.0, 3.0, "No!"),
            s(3.0, 4.0, "I know."),
            s(4.0, 5.0, "No!"),
        ];
        let out = postprocess(input.clone());
        assert_eq!(
            reasons(&out),
            [None, Some(Repeat), Some(Repeat), None, None]
        );
        // 不删内容：文本、时间、句数全部保留。
        assert_eq!(out.len(), input.len());
        for (a, b) in out.iter().zip(&input) {
            assert_eq!((a.start, a.end, &a.text), (b.start, b.end, &b.text));
        }
    }

    #[test]
    fn flags_long_low_density_segments() {
        let out = postprocess(vec![
            s(0.0, 22.8, "This is pretty freaky."), // 4 词 / 22.8 秒
            s(23.0, 26.0, "Look, Celia, we have to follow our passions."),
            s(30.0, 31.0, "No!"),                          // 短句不判
            s(40.0, 50.0, "首先我们将发射空间站的核心舱"), // 14 字 / 10 秒，正常
        ]);
        assert_eq!(reasons(&out), [Some(DurationMismatch), None, None, None]);
    }

    #[test]
    fn repeat_takes_precedence() {
        let out = postprocess(vec![s(0.0, 1.0, "No!"), s(1.0, 30.0, "No!")]);
        assert_eq!(reasons(&out), [None, Some(Repeat)]);
    }

    #[test]
    fn units_count_cjk_chars_and_words() {
        assert_eq!(speech_units("You could've gone worse."), 4);
        assert_eq!(speech_units("核心舱 2021 年"), 5);
    }

    #[test]
    fn suspect_serialization() {
        let j = serde_json::to_string(&s(0.0, 1.0, "a")).unwrap();
        assert!(!j.contains("suspect"));
        let mut x = s(0.0, 1.0, "a");
        x.suspect = Some(DurationMismatch);
        assert!(serde_json::to_string(&x)
            .unwrap()
            .contains(r#""suspect":"duration_mismatch""#));
        let back: TranscriptSegment =
            serde_json::from_str(r#"{"start":0,"end":1,"text":"a"}"#).unwrap();
        assert_eq!(back.suspect, None);
    }

    #[test]
    fn clean_text_skips_suspects_full_text_keeps_them() {
        let t = crate::models::Transcript {
            language: "en".into(),
            source: crate::models::TranscriptSource::Asr,
            segments: postprocess(vec![s(0.0, 1.0, "Hi"), s(1.0, 2.0, "Hi")]),
        };
        assert_eq!(t.clean_text(), "Hi");
        assert_eq!(t.full_text(), "Hi\nHi");
    }
}
