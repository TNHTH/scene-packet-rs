//! 中文转写后处理：繁→简（P4-1 修订）。
//!
//! whisper medium/large 对 `zh` 常输出繁体。初始提示词会改变解码、实测漏整句（CER 0.047→0.244），
//! 所以改为转写后换字形：不改变识别内容、不改用词（等价 zh-Hans，不做 zh-CN 地区词替换）。
//! 实现：内置 OpenCC 单字表（Apache-2.0）+ 少量一对多上下文规则。没有用 `zhconv` crate：
//! 它是 GPL-2.0-or-later，与本仓库 MIT 许可的分发二进制不兼容。
//!
//! 一对多规则：
//! - `著`（简体也合法）：作助词时转 `着`；`著作/著名/显著/名著/土著…` 等词保留。
//! - `乾`：一般转 `干`；`乾隆/乾坤/乾元/乾卦/乾嘉/乾陵` 保留。
//! - `瞭`：一般转 `了`；`瞭望` 保留。`藉`：一般转 `借`；`狼藉/慰藉/枕藉` 保留。

use super::t2s_table::T2S;

const ZHU_KEEP_NEXT: &[char] = &['作', '名', '称', '述', '录', '书', '者'];
const ZHU_KEEP_PREV: &[char] = &[
    '显', '顯', '卓', '土', '原', '名', '巨', '论', '論', '专', '專', '编', '編', '遗', '遺', '合',
    '新', '译', '譯', '昭',
];
const QIAN_KEEP_NEXT: &[char] = &['隆', '坤', '元', '卦', '嘉', '陵'];

fn table(c: char) -> char {
    T2S.binary_search_by_key(&c, |&(k, _)| k)
        .map(|i| T2S[i].1)
        .unwrap_or(c)
}

/// 整段文本繁→简。
pub fn t2s(s: &str) -> String {
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in cs.iter().enumerate() {
        let prev = i.checked_sub(1).map(|j| cs[j]);
        let next = cs.get(i + 1).copied();
        let r = match c {
            '著' => {
                if next.is_some_and(|n| ZHU_KEEP_NEXT.contains(&n))
                    || prev.is_some_and(|p| ZHU_KEEP_PREV.contains(&p))
                {
                    '著'
                } else {
                    '着'
                }
            }
            '乾' if next.is_some_and(|n| QIAN_KEEP_NEXT.contains(&n)) => '乾',
            // OpenCC 字表不含 瞭→了（简体也收 瞭），这里显式处理。
            '瞭' if next == Some('望') => '瞭',
            '瞭' => '了',
            '藉' if prev.is_some_and(|p| ['狼', '慰', '枕'].contains(&p)) => '藉',
            '藉' => '借',
            _ => table(c),
        };
        out.push(r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_and_unique() {
        assert!(T2S.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(T2S.len() > 3000);
    }

    #[test]
    fn converts_traditional_keeps_others() {
        assert_eq!(
            t2s("長征五號運載火箭首次飛行任務取得圓滿成功"),
            "长征五号运载火箭首次飞行任务取得圆满成功"
        );
        assert_eq!(t2s("空间站 2021 ABC，。"), "空间站 2021 ABC，。");
    }

    #[test]
    fn one_to_many_context() {
        assert_eq!(t2s("這意味著"), "这意味着");
        assert_eq!(t2s("接著看著"), "接着看着");
        assert_eq!(t2s("著作 著名 顯著 名著 土著"), "著作 著名 显著 名著 土著");
        assert_eq!(t2s("乾淨 乾燥 乾隆 乾坤"), "干净 干燥 乾隆 乾坤");
        assert_eq!(t2s("瞭解 瞭望"), "了解 瞭望");
        assert_eq!(t2s("藉口 狼藉"), "借口 狼藉");
        assert_eq!(t2s("幹部 頭髮 發展 後來"), "干部 头发 发展 后来");
    }
}
