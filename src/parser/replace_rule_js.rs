//! 在沙盒里执行 Legado 的 `@js:` 净化规则。
//!
//! 为什么需要独立一层：Legado 的 `replacement` 允许写成 `@js:` 开头的脚本，脚本把
//! 匹配到的文本当 `result`，返回值才是替换结果。这类规则无法用 `String.replace`
//! 的字符串替换表达（例如「星号修复」要按上下文把 `ꁘꁘ` 还原成对应的脏话）。
//!
//! 为什么不在前端跑：`new Function` 会把导进来的第三方规则直接放进渲染进程，那里能
//! 摸到 DOM、`window.__TAURI_INTERNALS__`，等于把整个 IPC 面暴露给随机规则。所以放
//! 在 rquickjs 沙盒里，与书源规则同一个执行模型。
//!
//! 求值模型与 Legado 对齐：对每条规则做一次 `text.replace(regex, ...)`；脚本规则用
//! `new Function('result', code)` 逐匹配求值，普通规则仍走字符串替换。规则顺序即数组
//! 顺序（调用方已按 `order` 排好）。

use rquickjs::{Context, Runtime};

/// rquickjs 的栈上限，与书源脚本保持一致。
const JS_MAX_STACK_SIZE: usize = 1024 * 1024;

/// 一条待执行的规则。`pattern` / `flags` 已由前端从 Java 正则降级为 JS 正则。
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceRuleStep {
    pub pattern: String,
    pub flags: String,
    pub replacement: String,
    #[serde(default)]
    pub is_script: bool,
}

/// 执行整条规则链，返回处理后的正文。
pub fn apply_rules(text: &str, rules: &[ReplaceRuleStep]) -> Result<String, String> {
    if text.is_empty() || rules.is_empty() {
        return Ok(text.to_string());
    }

    let runtime = Runtime::new().map_err(|error| format!("初始化脚本引擎失败: {error}"))?;
    runtime.set_max_stack_size(JS_MAX_STACK_SIZE);
    let context = Context::full(&runtime).map_err(|error| format!("初始化脚本上下文失败: {error}"))?;

    // 输入以 JSON 字面量注入：JSON 是 JS 的子集，直接当表达式求值即可，不必手工转义。
    // U+2028/U+2029 在旧引擎里不能出现在字符串字面量中，先转成转义序列。
    let text_literal = json_literal(&serde_json::Value::String(text.to_string()));
    let rules_literal = json_literal(
        &serde_json::to_value(rules).map_err(|error| format!("规则序列化失败: {error}"))?,
    );

    context.with(|ctx| {
        let setup = format!(
            "globalThis.__readerInput = {text_literal};\nglobalThis.__readerRules = {rules_literal};"
        );
        ctx.eval::<(), _>(setup.as_bytes())
            .map_err(|error| format!("注入规则失败: {error}"))?;

        let script = format!("({PROCESSOR})");
        ctx.eval::<String, _>(script.as_bytes())
            .map_err(|error| format!("执行净化脚本失败: {error}"))
    })
}

fn json_literal(value: &serde_json::Value) -> String {
    let raw = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string());
    raw.replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// 在沙盒里跑完整条链。
///
/// 脚本规则用 `eval` 而不是 `new Function(code)`：Legado 在 Rhino 上执行替换脚本时，
/// 取的是**最后一个表达式语句的值**（脚本里没有 `return` 也照样有返回值，例如
/// `R = result; R == "１" ? "1" : R`）。`new Function` 的函数体不返回最后表达式的值，
/// 直接把 `code` 当函数体喂进去会得到 `undefined`，于是整段正文被替换成空 —— 这是
/// 必须用 eval 语义的原因。`eval` 写在函数里且接收 `result`，同时满足变量绑定与
/// 完成值语义。
const PROCESSOR: &str = r#"
(function () {
  var out = globalThis.__readerInput;
  var rules = globalThis.__readerRules;
  for (var i = 0; i < rules.length; i++) {
    var rule = rules[i];
    var re = new RegExp(rule.pattern, rule.flags);
    if (rule.isScript) {
      var fn = new Function('result', 'return eval(' + JSON.stringify(rule.replacement) + ')');
      out = out.replace(re, function (match) {
        var value = fn(String(match));
        return value === undefined || value === null ? '' : String(value);
      });
    } else {
      out = out.replace(re, rule.replacement);
    }
  }
  return out;
})()
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(pattern: &str, replacement: &str) -> ReplaceRuleStep {
        ReplaceRuleStep {
            pattern: pattern.to_string(),
            flags: "gm".to_string(),
            replacement: replacement.to_string(),
            is_script: false,
        }
    }

    fn script(pattern: &str, code: &str) -> ReplaceRuleStep {
        ReplaceRuleStep {
            pattern: pattern.to_string(),
            flags: "gm".to_string(),
            replacement: code.to_string(),
            is_script: true,
        }
    }

    #[test]
    fn applies_plain_rules_in_order() {
        let rules = vec![plain("aaa", "b"), plain("b+", "c")];
        assert_eq!(apply_rules("aaa aaa", &rules).unwrap(), "c c");
    }

    #[test]
    fn script_sees_matched_text_as_result() {
        // 全角数字转半角：与真实规则同形态（只依赖 result）
        let code = r#"R = result; R == "１" ? "1" : R == "２" ? "2" : R"#;
        let rules = vec![script("[０-９]", code)];
        assert_eq!(apply_rules("１２", &rules).unwrap(), "12");
    }

    #[test]
    fn script_can_reuse_globals_across_matches() {
        // 「星号修复」那类写法：脚本体内定义辅助函数
        let code = r#"
            function M(x) { return result.search(x) == -1 ? false : true }
            M("ꁘꁘꁘ") ? "混帐" : result
        "#;
        let rules = vec![script("ꁘ+", code)];
        assert_eq!(apply_rules("aꁘꁘꁘb", &rules).unwrap(), "a混帐b");
    }

    #[test]
    fn script_returning_non_string_is_coerced() {
        let rules = vec![script("\\d", "42")];
        assert_eq!(apply_rules("x1y", &rules).unwrap(), "x42y");
    }

    #[test]
    fn script_returning_null_drops_the_match() {
        let rules = vec![script("广告", "null")];
        assert_eq!(apply_rules("正文广告正文", &rules).unwrap(), "正文正文");
    }

    #[test]
    fn broken_script_reports_error_instead_of_panicking() {
        let rules = vec![script("x", "this is not javascript(")];
        assert!(apply_rules("x", &rules).is_err());
    }

    #[test]
    fn plain_and_script_rules_mix_in_declared_order() {
        let rules = vec![
            plain("甲", "乙"),
            script("乙", r#"result + "丙""#),
            plain("丙", "丁"),
        ];
        assert_eq!(apply_rules("甲", &rules).unwrap(), "乙丁");
    }

    #[test]
    fn text_with_quotes_and_newlines_survives_injection() {
        let rules = vec![plain("x", "y")];
        let input = "a\"b\\c\nd\u{2028}e";
        let out = apply_rules(input, &rules).unwrap();
        assert_eq!(out, input);
    }
}
