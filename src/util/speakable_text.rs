//! 判断文本里是否含有「可朗读」的字符。
//!
//! 为什么需要它：Edge / 微软朗读服务对**纯标点或纯引号**文本（`“”`、`，`、`……`）
//! 不返回音频 —— 这是服务端**正常行为**（没有可朗读内容），不是通道故障。
//! 但合成链路只能看到「收没收到音频」，于是会把这种情况误报成
//! 「朗读服务暂时不可用……请改用其它朗读通道」，把用户往「换通道」上误导。
//! 在发请求前用本判定给出准确错误：既省一次必然失败的请求，也不冒充故障。

use std::sync::OnceLock;

use regex::Regex;

/// `\p{L}`（字母，汉字属于 Lo）+ `\p{N}`（数字）。
///
/// 用 Unicode 属性而不是 `[a-zA-Z0-9]`：中文、日文、韩文、希腊字母等都要算可朗读，
/// 而纯标点/引号/省略号/破折号/emoji 都不属于这两个类别，服务端对它们不发音。
static SPEAKABLE_CHAR: OnceLock<Regex> = OnceLock::new();

/// 文本里是否至少有一个字母或数字。
///
/// 注意这里**不做 trim** 判断：纯空白由调用方按「朗读文本为空」处理（语义不同），
/// 本函数只回答「有没有能读出来的字符」。
pub fn has_speakable_content(text: &str) -> bool {
    let pattern = SPEAKABLE_CHAR
        .get_or_init(|| Regex::new(r"[\p{L}\p{N}]").expect("可朗读字符正则是常量，必须合法"));
    pattern.is_match(text)
}

#[cfg(test)]
mod tests {
    use super::has_speakable_content;

    /// 纯标点/引号/空白/emoji：服务端本来就不返回音频，不能当通道故障。
    #[test]
    fn punctuation_only_text_is_not_speakable() {
        for text in [
            "……",
            "“”",
            "“",
            "，。",
            "——",
            "\u{3000}\u{3000}",
            "",
            "   ",
            "?!.,;:",
            "😀😀",
            "（）【】",
        ] {
            assert!(!has_speakable_content(text), "text = {text:?}");
        }
    }

    /// 字母/数字（含汉字、全角digit、其它语言）都算可朗读。
    #[test]
    fn text_with_letters_or_digits_is_speakable() {
        for text in [
            "嗯",
            "123",
            "abc",
            "他说：“你好。”",
            "Ａ１",
            "3.14",
            "한글",
            "άλφα",
            "ＡＢＣ",
        ] {
            assert!(has_speakable_content(text), "text = {text:?}");
        }
    }
}
