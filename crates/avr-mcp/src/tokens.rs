//! 轻量 token 估算（不依赖 tokenizer）。
//!
//! 启发式：ASCII 字母数字串每 4 个字符约 1 token（向上取整）；其余非空白字符
//!（CJK、全角、标点、符号）每个 1 token；空白不计。偏向高估，用于“是否直接返回全文”这类保守判断。

/// 估算文本 token 数。
pub fn estimate_tokens(text: &str) -> usize {
    let mut tokens = 0;
    let mut word = 0usize;
    let flush = |word: &mut usize, tokens: &mut usize| {
        if *word > 0 {
            *tokens += word.div_ceil(4);
            *word = 0;
        }
    };
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            word += 1;
            continue;
        }
        flush(&mut word, &mut tokens);
        if c.is_whitespace() {
            continue;
        }
        // CJK、全角字符、标点与其他符号都按 1 计。
        tokens += 1;
    }
    flush(&mut word, &mut tokens);
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_basics() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("你好世界"), 4);
        assert_eq!(estimate_tokens("hello world"), 4); // 5→2, 5→2
        assert_eq!(estimate_tokens("a b c"), 3);
        assert_eq!(estimate_tokens("第1句：你好"), 6); // 第 1 句 ： 你 好
        assert_eq!(estimate_tokens("  \n\t "), 0);
    }
}
