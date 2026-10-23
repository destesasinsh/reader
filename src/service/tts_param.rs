//! SSML `prosody` 属性（`rate` / `pitch` / `volume`）的归一化。
//!
//! 这几个值最终会被拼成 `rate="+10%"` 这样的属性。前端做的是「档位→百分比」的
//! 换算，正常路径送来的都是合法值；但这条命令是 IPC 入口，手写调用或旧配置可能
//! 送来 `10`（缺符号）、`abc`、`1e9`。原样拼进 SSML 的后果是服务端回一个
//! 语焉不详的 400，用户只看到「合成失败」—— 这里提前把不合规的值折回默认档，
//! 让朗读继续可用，而不是整段发不出声。

/// 一个 SSML 偏移量（`+10%` / `-5Hz`）的归一化结果。
///
/// 接受 `+10%`、`-5Hz`、`10%`（自动补 `+`）。单位必须落在 `units` 里 ——
/// Edge 的 pitch 是 `Hz`，微软朗读的 pitch 是 `%`，两个通道不能混。
pub fn normalize_offset(value: Option<&str>, default: &str, units: &[&str]) -> String {
    let Some(raw) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return default.to_string();
    };
    match parse_offset(raw, units) {
        Some(normalized) => normalized,
        None => {
            tracing::warn!("忽略不合法的 SSML 参数 {raw:?}，回退到 {default}");
            default.to_string()
        }
    }
}

/// 真正的解析：`[+-]?数字单位`。数字可带小数（`+1.5%`），不接受指数与空白。
fn parse_offset(raw: &str, units: &[&str]) -> Option<String> {
    let (sign, rest) = match raw.as_bytes().first()? {
        b'+' => ('+', &raw[1..]),
        b'-' => ('-', &raw[1..]),
        _ => ('+', raw),
    };
    let (number, unit) = units
        .iter()
        .find_map(|unit| rest.strip_suffix(unit).map(|number| (number, *unit)))?;
    // 只认十进制：`1e9` 这类关掉（SSML 不接受），空串也挡下。
    if number.is_empty()
        || !number
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == '.')
        || number.matches('.').count() > 1
    {
        return None;
    }
    Some(format!("{sign}{number}{unit}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERCENT: &[&str] = &["%"];
    const HZ_OR_PERCENT: &[&str] = &["Hz", "%"];

    #[test]
    fn keeps_well_formed_values() {
        assert_eq!(normalize_offset(Some("+10%"), "+0%", PERCENT), "+10%");
        assert_eq!(normalize_offset(Some("-5%"), "+0%", PERCENT), "-5%");
        assert_eq!(normalize_offset(Some(" +1.5% "), "+0%", PERCENT), "+1.5%");
        assert_eq!(normalize_offset(Some("-10Hz"), "+0Hz", HZ_OR_PERCENT), "-10Hz");
    }

    #[test]
    fn adds_the_missing_sign() {
        // 前端老版本/手写调用可能不带符号，SSML 要求带符号，这里补上。
        assert_eq!(normalize_offset(Some("10%"), "+0%", PERCENT), "+10%");
        assert_eq!(normalize_offset(Some("5Hz"), "+0Hz", HZ_OR_PERCENT), "+5Hz");
    }

    #[test]
    fn falls_back_on_garbage() {
        // 这些原样拼进 SSML 会让服务端回一个难懂的 400，必须挡在本地。
        for bad in ["abc", "10", "%", "+", "+%", "10%%", "1e9%", "+1.2.3%", "10 px"] {
            assert_eq!(
                normalize_offset(Some(bad), "+0%", PERCENT),
                "+0%",
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn rejects_unit_from_another_channel() {
        // 微软朗读的 pitch 只吃百分比，Edge 的只吃 Hz —— 混用必须挡下。
        assert_eq!(normalize_offset(Some("+10Hz"), "+0%", PERCENT), "+0%");
    }

    #[test]
    fn empty_and_missing_use_default() {
        assert_eq!(normalize_offset(None, "+0%", PERCENT), "+0%");
        assert_eq!(normalize_offset(Some(""), "+0%", PERCENT), "+0%");
        assert_eq!(normalize_offset(Some("   "), "+0%", PERCENT), "+0%");
    }
}
