use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Legado「朗读引擎」(`httpTTS`)条目。
///
/// 字段与 Legado 导出的朗读引擎 JSON 一一对应（camelCase）。这些集合由用户各自
/// 整理，字段覆盖并不一致（有的缺 `header`、有的缺 `contentType`），所以整份
/// 结构标了 `#[serde(default)]`：缺项按「未设置」处理，而不是让整份导入失败。
/// 类型也做宽容处理 —— `id` 见过字符串形态、`header` 见过对象形态，直接按
/// 声明类型反序列化会整条丢弃。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HttpTts {
    /// Legado 用导入时刻的毫秒时间戳当 id；0 表示「尚未分配」，保存时由服务补发。
    #[serde(deserialize_with = "deserialize_id")]
    pub id: i64,
    pub name: String,
    /// 朗读地址模板。三种形态（见 `parser::http_tts_template`）：
    /// 纯 URL、URL + `,{"method":"POST","body":"..."}` 请求配置、`@js:`/`<js>` 脚本。
    pub url: String,
    /// 请求头（JSON 对象文本）。值里可以含 `{{...}}` 模板。
    #[serde(deserialize_with = "deserialize_string_option")]
    pub header: Option<String>,
    /// Legado 的并发率（`次数/毫秒` 或 `毫秒` 间隔）。朗读时的实际限速见
    /// `service::http_tts_throttle`；导入/回写都原样保留。
    #[serde(deserialize_with = "deserialize_string_option")]
    pub concurrent_rate: Option<String>,
    /// Legado 的合成并发线程数（1–8）。缺省/0/负数表示「引擎没声明」——此时不限
    /// 并发（本应用的前端缓冲流自己控制在 2 个），只有写了具体数字才生效，见
    /// `service::http_tts_throttle::concurrency_limit`。
    #[serde(deserialize_with = "deserialize_i64_option")]
    pub synthesis_thread_count: Option<i64>,
    #[serde(deserialize_with = "deserialize_string_option")]
    pub content_type: Option<String>,
    #[serde(deserialize_with = "deserialize_bool_option")]
    pub enabled_cookie_jar: Option<bool>,
    #[serde(deserialize_with = "deserialize_string_option")]
    pub login_url: Option<String>,
    /// Legado 的登录界面 JSON；朗读引擎用不到登录，原样存着即可。
    #[serde(deserialize_with = "deserialize_string_option")]
    pub login_ui: Option<String>,
    #[serde(deserialize_with = "deserialize_string_option")]
    pub login_check_js: Option<String>,
    #[serde(deserialize_with = "deserialize_i64_option")]
    pub last_update_time: Option<i64>,
}

/// `id` 的宽容反序列化：远程集合里既有数字 `1652146476375`，也见过字符串
/// `"1652146476375"`。非法值退化成 0（保存时补发新 id），而不是丢掉整条引擎。
fn deserialize_id<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(value_to_i64(&Value::deserialize(deserializer)?).unwrap_or(0))
}

fn deserialize_i64_option<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Null) => None,
        Some(value) => value_to_i64(&value),
    })
}

fn value_to_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|n| n as i64)),
        // 空串 / "null" 在导入数据里表示「没有值」，不是 0。
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() || text.eq_ignore_ascii_case("null") {
                None
            } else {
                text.parse::<i64>()
                    .ok()
                    .or_else(|| text.parse::<f64>().ok().map(|n| n as i64))
            }
        }
        _ => None,
    }
}

/// 可选文本字段的宽容反序列化：数字/布尔/对象都转成文本。
///
/// `header` 在少数集合里是 JSON 对象而不是 JSON 文本；`loginUi` 可能是数组。
/// 统一序列化回文本后，渲染层再按需 `JSON.parse`，比在反序列化阶段报错更抗脏数据。
fn deserialize_string_option<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text),
        Some(value) => Some(value.to_string()),
    })
}

/// 布尔字段的宽容反序列化：导入数据里既有 `false`，也有 `"false"` / `"1"` / `1`。
fn deserialize_bool_option<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Null) => None,
        Some(Value::Bool(value)) => Some(value),
        Some(Value::Number(number)) => Some(number.as_f64().is_some_and(|n| n != 0.0)),
        Some(Value::String(text)) => match text.trim().to_ascii_lowercase().as_str() {
            "" | "null" => None,
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 真实远程集合里的形态：所有字段都在，id 是数字，多数可选字段是空串。
    #[test]
    fn deserializes_real_legado_entry() {
        let raw = json!({
            "concurrentRate": "0",
            "contentType": "",
            "enabledCookieJar": false,
            "header": "",
            "id": 1652146476375i64,
            "lastUpdateTime": 1695006617414i64,
            "loginCheckJs": "",
            "loginUi": "",
            "loginUrl": "",
            "name": "示例引擎",
            "url": "https://example.test/tts?text={{speakText}}"
        });
        let tts: HttpTts = serde_json::from_value(raw).unwrap();
        assert_eq!(tts.id, 1652146476375);
        assert_eq!(tts.name, "示例引擎");
        assert_eq!(tts.url, "https://example.test/tts?text={{speakText}}");
        assert_eq!(tts.concurrent_rate.as_deref(), Some("0"));
        assert_eq!(tts.enabled_cookie_jar, Some(false));
        assert_eq!(tts.last_update_time, Some(1695006617414));
    }

    /// 缺字段、类型不对的条目也要能读进来 —— 一份集合里混着几十条不同来源，
    /// 一条脏数据不该让整份导入失败。
    #[test]
    fn tolerates_missing_and_mis_typed_fields() {
        let tts: HttpTts = serde_json::from_value(json!({
            "id": "1660388912246",
            "name": "A 引擎",
            "url": "https://example.test/a",
            "header": { "User-Agent": "demo" },
            "enabledCookieJar": "false",
            "loginUi": ["input", "x"],
            "lastUpdateTime": "1695006617416",
            "unknownField": 1
        }))
        .unwrap();
        assert_eq!(tts.id, 1660388912246);
        assert_eq!(tts.enabled_cookie_jar, Some(false));
        assert_eq!(tts.last_update_time, Some(1695006617416));
        // 对象形态的 header / 数组形态的 loginUi 统一折成文本保留，而不是丢弃。
        assert_eq!(tts.header.as_deref(), Some(r#"{"User-Agent":"demo"}"#));
        assert_eq!(tts.login_ui.as_deref(), Some(r#"["input","x"]"#));
        // 完全缺失的可选字段是 None，不是解析失败。
        assert_eq!(tts.login_check_js, None);

        let minimal: HttpTts = serde_json::from_value(json!({"name": "x", "url": "y"})).unwrap();
        assert_eq!(minimal.id, 0);
        assert_eq!(minimal.header, None);
    }

    /// IPC 出参必须是 camelCase，前端 `HttpTts` 接口按这些键读。
    #[test]
    fn serializes_camel_case_fields() {
        let tts = HttpTts {
            id: 7,
            name: "n".to_string(),
            url: "u".to_string(),
            concurrent_rate: Some("0".to_string()),
            enabled_cookie_jar: Some(true),
            last_update_time: Some(1),
            ..HttpTts::default()
        };
        let value = serde_json::to_value(&tts).unwrap();
        for key in [
            "id",
            "name",
            "url",
            "concurrentRate",
            "synthesisThreadCount",
            "contentType",
            "enabledCookieJar",
            "header",
            "loginUrl",
            "loginUi",
            "loginCheckJs",
            "lastUpdateTime",
        ] {
            assert!(value.get(key).is_some(), "missing key {key}: {value}");
        }
    }
}
