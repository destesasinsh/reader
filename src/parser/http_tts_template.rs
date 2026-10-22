use std::collections::HashMap;

use serde_json::{json, Value};

use crate::crawler::url_analyzer::{parse_source_headers, split_url_options};
use crate::error::error::AppError;
use crate::model::book_source::BookSource;
use crate::model::http_tts::HttpTts;
use crate::parser::js::{eval_js_with_bindings, with_source};

/// `source.getLoginInfoMap()` 的 Java `Map` 语义补丁。
///
/// Legado 里它返回 `Map<String, String>`（有 `get(k)`）；prelude 的实现是
/// `JSON.parse(...)`，普通对象没有 `get`，朗读模板 `source.getLoginInfoMap().get('AppKey')`
/// 会直接 TypeError。这里**只在本次求值的作用域内**给对象补上 Map 风格方法：
/// 书源的登录态语义与朗读无关，不该为了朗读去改公共 prelude 的行为。
const LOGIN_INFO_MAP_SHIM: &str = r#"(function () {
  if (typeof source === 'undefined' || !source) return;
  var original = source.getLoginInfoMap;
  if (typeof original !== 'function') return;
  source.getLoginInfoMap = function () {
    var map = original();
    if (!map || typeof map.get === 'function') return map || {};
    function def(name, fn) {
      Object.defineProperty(map, name, { value: fn, enumerable: false, writable: true, configurable: true });
    }
    def('get', function (k) { var v = map[k]; return v === undefined ? null : v; });
    def('containsKey', function (k) { return Object.prototype.hasOwnProperty.call(map, k); });
    def('isEmpty', function () { return Object.keys(map).length === 0; });
    def('size', function () { return Object.keys(map).length; });
    return map;
  };
})();
"#;

/// 朗读地址里 `,{...}` 请求配置（**模板展开前**拆分出来的原始形态）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HttpTtsConfigTemplate {
    /// 已大写，默认 `GET`。
    pub method: String,
    /// 请求体模板（POST 用）。
    pub body: Option<String>,
    /// 额外请求头，值里仍可含 `{{...}}`。
    pub headers: Vec<(String, String)>,
    /// 响应编码，只在解码错误信息时用得到。
    pub charset: Option<String>,
    /// Legado 的 WebView 渲染开关。桌面端不跑 WebView，命中时直接给出可读错误。
    pub web_view: bool,
}

/// 朗读地址的三种形态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpTtsUrlKind {
    /// 普通地址 + 可选的 `,{...}` 配置；两类文本都在渲染阶段展开模板。
    Template {
        url: String,
        config: Option<HttpTtsConfigTemplate>,
    },
    /// `@js:` / `<js>...</js>`：整条地址由脚本求值得出。
    Script(String),
}

/// 渲染完成的请求：全部是最终值，命令层只管发出去。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedHttpTtsRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub charset: Option<String>,
}

/// 渲染一次朗读请求：拆分地址 → 展开模板 → 合并请求头。
///
/// `speak_speed` 是 Legado 的 0~10 语速档位（不是倍速），由命令层从用户倍速折算。
pub fn render_http_tts_request(
    tts: &HttpTts,
    speak_text: &str,
    speak_speed: i64,
) -> Result<RenderedHttpTtsRequest, AppError> {
    let text = speak_text.trim();
    if text.is_empty() {
        return Err(AppError::BadRequest("朗读文本不能为空".to_string()));
    }

    // 一次 with_source 包住整条渲染链路：`source.*` 绑定只在一段连续求值内需要生效，
    // 也避免每个 `{{...}}` 表达式都重建一次书源上下文。
    with_template_source(|| match parse_http_tts_url(&tts.url)? {
        HttpTtsUrlKind::Template { url, config } => {
            render_template_request(tts, &url, config.as_ref(), text, speak_speed)
        }
        HttpTtsUrlKind::Script(script) => {
            render_script_request(tts, &script, text, speak_speed)
        }
    })
}

/// 把朗读引擎的 `url` 拆成「地址模板 + 请求配置」或「脚本」。
///
/// 拆分配置必须在**模板展开前**做：书里的句子可能含 `,{`（ASCII 逗号加大括号），
/// 展开后按 `,{` 去找配置会把正文误当成 JSON。反过来，配置里的 JSON 字符串本身
/// 含 `{{...}}` 也没关系 —— 拆分器认识字符串字面量（见 `split_url_options`）。
pub fn parse_http_tts_url(raw: &str) -> Result<HttpTtsUrlKind, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("朗读引擎地址为空".to_string()));
    }
    if let Some(script) = strip_js_wrapper(trimmed) {
        return Ok(HttpTtsUrlKind::Script(script.to_string()));
    }
    let (url, config) = split_url_config(trimmed)?;
    Ok(HttpTtsUrlKind::Template {
        url: url.to_string(),
        config,
    })
}

/// 展开一段含 `{{...}}` 的模板。
///
/// 每个 `{{表达式}}` 交给现有的 rquickjs 引擎求值（`speakText` / `speakSpeed`
/// 以 bindings 注入），这里只负责找括号边界，不解析表达式本身。
pub fn expand_template(
    template: &str,
    speak_text: &str,
    speak_speed: i64,
) -> Result<String, AppError> {
    with_template_source(|| expand_template_inner(template, speak_text, speak_speed))
}

fn expand_template_inner(
    template: &str,
    speak_text: &str,
    speak_speed: i64,
) -> Result<String, AppError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        let Some(close) = find_template_close(after_open) else {
            // 没有收尾 `}}`：把剩下的原文原样保留（书源里的半截模板不该让朗读失败）。
            // 注意 `out` 已经写过 `{{` 之前的前缀，这里只能从 `start` 起追加；
            // 若写成 `out.push_str(rest)`，前缀会被重复一遍（实测 `abc{{speakText`
            // 会变成 `abcabc{{speakText`）。
            out.push_str(&rest[start..]);
            return Ok(out);
        };
        let expression = after_open[..close].trim();
        out.push_str(&eval_template_expression(
            expression,
            speak_text,
            speak_speed,
        )?);
        rest = &after_open[close + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// 找出与当前 `{{` 配对的那个 `}}`，返回它在 `source` 里的起始字节位置。
///
/// 不能简单地 `find("}}")`：模板里可能出现
/// - 嵌套的 `{{{{speakText}}}}`（内层是一段块语句，整体仍是一个表达式）；
/// - 字符串字面量里的大括号，如 `{{'{{' + speakText + '}}'}}`。
///
/// 所以这里做「括号配平 + 跳过字符串字面量」的扫描。找不到配对的 `}}` 返回 `None`，
/// 调用方按「半截模板」原样保留。
fn find_template_close(source: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 1usize;
    let mut quote: Option<u8> = None;
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        let byte = bytes[index];
        if let Some(active) = quote {
            if byte == b'\\' {
                index += 2;
                continue;
            }
            if byte == active {
                quote = None;
            }
            index += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' | b'`' => {
                quote = Some(byte);
                index += 1;
            }
            b'{' if bytes[index + 1] == b'{' => {
                depth += 1;
                index += 2;
            }
            b'}' if bytes[index + 1] == b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
                index += 2;
            }
            _ => index += 1,
        }
    }
    None
}

/// 求值单个 `{{...}}` 表达式。
///
/// 外部块语句 `{ ... }` 的完成值就是最后一条表达式语句的值，所以直接把表达式
/// 接在 shim 之后即可，不必包装成函数（包装会破坏 `;`、注释、对象字面量等写法）。
fn eval_template_expression(
    expression: &str,
    speak_text: &str,
    speak_speed: i64,
) -> Result<String, AppError> {
    let expression = expression.trim().trim_end_matches(';').trim_end();
    if expression.is_empty() {
        return Ok(String::new());
    }
    eval_js(&format!("{expression}"), speak_text, speak_speed)
        .map_err(|err| {
            AppError::BadRequest(format!("朗读引擎模板求值失败（{expression}）: {err}"))
        })
}

/// 求值 `@js:` / `<js>` 形态的地址脚本。脚本结果本身不再过模板（见 `render_script_request`）。
fn eval_script(script: &str, speak_text: &str, speak_speed: i64) -> Result<String, AppError> {
    eval_js(script, speak_text, speak_speed)
        .map_err(|err| AppError::BadRequest(format!("朗读引擎地址脚本求值失败: {err}")))
}

/// 把脚本体交给 rquickjs：`speakText` / `speakSpeed` 以 bindings 注入，
/// `source` 由 `with_source` 提供（`source.getLoginInfoMap()` 依赖它）。
fn eval_js(script: &str, speak_text: &str, speak_speed: i64) -> anyhow::Result<String> {
    let mut bindings: HashMap<String, Value> = HashMap::new();
    bindings.insert("speakText".to_string(), Value::String(speak_text.to_string()));
    bindings.insert("speakSpeed".to_string(), json!(speak_speed));
    let script = format!("{LOGIN_INFO_MAP_SHIM}\n{script}");
    eval_js_with_bindings(&script, "", "", &bindings)
}

/// 在「朗读引擎」这个合成书源上下文里执行 f。
///
/// 模板求值需要 `source` 绑定（登录信息等），而 `eval_js_with_bindings` 的
/// `source` 由 `with_source` 提供。合成书源用固定地址，上下文可跨表达式复用；
/// 名字只影响 JS 里可见的 `source.bookSourceName`，与真实书源无关。
fn with_template_source<T>(f: impl FnOnce() -> T) -> T {
    with_source(&template_source(), f)
}

fn template_source() -> BookSource {
    BookSource {
        book_source_url: "legado-http-tts://local".to_string(),
        book_source_name: "Legado 朗读引擎".to_string(),
        ..BookSource::default()
    }
}

fn render_template_request(
    tts: &HttpTts,
    url_template: &str,
    config: Option<&HttpTtsConfigTemplate>,
    text: &str,
    speak_speed: i64,
) -> Result<RenderedHttpTtsRequest, AppError> {
    let url = expand_template_inner(url_template, text, speak_speed)?;
    let url = normalize_url(&url)?;

    let mut headers = expand_header_defaults(tts.header.as_deref(), text, speak_speed)?;
    let mut method = "GET".to_string();
    let mut body = None;
    let mut charset = None;

    if let Some(config) = config {
        if config.web_view {
            return Err(unsupported_web_view());
        }
        method = config.method.clone();
        charset = config.charset.clone();
        for (key, value) in &config.headers {
            let value = expand_template_inner(value, text, speak_speed)?;
            upsert_header(&mut headers, key.clone(), value);
        }
        if let Some(raw_body) = &config.body {
            body = Some(expand_template_inner(raw_body, text, speak_speed)?);
        }
    }

    apply_default_content_type(&mut headers, &method, body.as_deref());
    Ok(RenderedHttpTtsRequest {
        url,
        method,
        headers,
        body,
        charset,
    })
}

fn render_script_request(
    tts: &HttpTts,
    script: &str,
    text: &str,
    speak_speed: i64,
) -> Result<RenderedHttpTtsRequest, AppError> {
    let produced = eval_script(script, text, speak_speed)?;
    // 脚本结果仍可能带 `,{...}` 配置（Legado 允许脚本返回完整请求描述）。
    let (url, config) = split_url_config(&produced)?;
    let url = normalize_url(url)?;

    let mut headers = expand_header_defaults(tts.header.as_deref(), text, speak_speed)?;
    let mut method = "GET".to_string();
    let mut body = None;
    let mut charset = None;

    if let Some(config) = config {
        if config.web_view {
            return Err(unsupported_web_view());
        }
        method = config.method;
        charset = config.charset;
        for (key, value) in config.headers {
            upsert_header(&mut headers, key, value);
        }
        // 脚本产出的值是最终值，不能再过模板：否则正文里恰好出现的 `{{` 会被
        // 当成模板重新求值（脚本通常已经自己做过 encodeURIComponent）。
        body = config.body;
    }

    apply_default_content_type(&mut headers, &method, body.as_deref());
    Ok(RenderedHttpTtsRequest {
        url,
        method,
        headers,
        body,
        charset,
    })
}

/// 识别 `@js:` / `<js>...</js>` 形态，返回脚本体（两侧空白裁掉）。
///
/// 缺尾标签也按脚本体处理：集合里见过手工删掉 `</js>` 的条目，报「地址无效」
/// 不如照脚本求值一次。
fn strip_js_wrapper(raw: &str) -> Option<&str> {
    if let Some(rest) = raw.strip_prefix("@js:") {
        return Some(rest.trim());
    }
    if let Some(rest) = raw.strip_prefix("<js>") {
        return Some(rest.strip_suffix("</js>").unwrap_or(rest).trim());
    }
    None
}

/// 拆分 `,{...}` 请求配置并解析成结构体。///
/// 定位交给现有的 `split_url_options`（它就是 Legado 的 `url,{...}` 语法解析器，
/// 认识字符串字面量、不会把 URL 自身或模板里的逗号误当分隔符）。
fn split_url_config(raw: &str) -> Result<(&str, Option<HttpTtsConfigTemplate>), AppError> {
    let (url, options) = split_url_options(raw);
    let Some(options) = options else {
        return Ok((url, None));
    };
    let value: Value = serde_json::from_str(options).map_err(|err| {
        AppError::BadRequest(format!("朗读引擎请求配置不是合法 JSON: {err}"))
    })?;
    let object = value.as_object().ok_or_else(|| {
        AppError::BadRequest("朗读引擎请求配置必须是 JSON 对象".to_string())
    })?;

    let method = object
        .get("method")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|method| !method.is_empty())
        .map(|method| method.to_ascii_uppercase())
        .unwrap_or_else(|| "GET".to_string());
    if !SUPPORTED_METHODS.contains(&method.as_str()) {
        return Err(AppError::BadRequest(format!(
            "不支持的朗读引擎请求方法: {method}"
        )));
    }
    let body = object.get("body").map(|value| match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    });
    let headers = match object.get("headers") {
        Some(Value::String(raw)) => parse_source_headers(raw),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(key, value)| (key.clone(), header_value_to_string(value)))
            .collect(),
        _ => Vec::new(),
    };
    let charset = object
        .get("charset")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|charset| !charset.is_empty())
        .map(str::to_string);
    let web_view = object.get("webView").is_some_and(is_truthy);

    Ok((
        url,
        Some(HttpTtsConfigTemplate {
            method,
            body,
            headers,
            charset,
            web_view,
        }),
    ))
}

const SUPPORTED_METHODS: [&str; 6] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];

/// 朗读引擎的 `header` 字段默认请求头，值同样过模板。
///
/// 非法 JSON 不报错：真实集合里见过随手填的纯文本，丢掉默认头仍然能发请求，
/// 比整条引擎不可用要好。
fn expand_header_defaults(
    header: Option<&str>,
    text: &str,
    speak_speed: i64,
) -> Result<Vec<(String, String)>, AppError> {
    let Some(header) = header.map(str::trim).filter(|header| !header.is_empty()) else {
        return Ok(Vec::new());
    };
    let mut headers = Vec::new();
    for (key, value) in parse_source_headers(header) {
        let value = expand_template_inner(&value, text, speak_speed)?;
        upsert_header(&mut headers, key, value);
    }
    Ok(headers)
}

fn header_value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => matches!(
            text.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        ),
        _ => false,
    }
}

fn upsert_header(headers: &mut Vec<(String, String)>, key: String, value: String) {
    let key = key.trim();
    if key.is_empty() {
        return;
    }
    if let Some(existing) = headers
        .iter_mut()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
    {
        existing.1 = value;
    } else {
        headers.push((key.to_string(), value));
    }
}

/// POST 未显式给 Content-Type 时补 form 编码。
///
/// 百度等 TTS 接口的 body 就是 `a=1&b=2` 表单，缺 Content-Type 会被网关当成
/// 未知类型直接拒绝；配置里显式给了 Content-Type 时以配置为准。
fn apply_default_content_type(headers: &mut Vec<(String, String)>, method: &str, body: Option<&str>) {
    if body.is_none() || !matches!(method, "POST" | "PUT" | "PATCH") {
        return;
    }
    if headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
    {
        return;
    }
    headers.push((
        "Content-Type".to_string(),
        "application/x-www-form-urlencoded".to_string(),
    ));
}

/// 校验并规整最终地址：只允许 http(s)，兼容 `//host/path` 协议相对写法。
fn normalize_url(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("朗读引擎地址为空".to_string()));
    }
    let url = if let Some(rest) = trimmed.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        trimmed.to_string()
    };
    let url = dedupe_query_params(&url);
    let parsed = url::Url::parse(&url)
        .map_err(|_| AppError::BadRequest(format!("朗读引擎地址无效: {url}")))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(AppError::BadRequest(format!(
            "朗读引擎地址仅支持 HTTP/HTTPS: {url}"
        )));
    }
    Ok(url)
}

/// 去掉重复的查询参数：**同名保留最后一次的值**，位置沿用第一次出现处。
///
/// 为什么必须做：在线集合里 71/84 条引擎的地址把 `voiceId` 写了两遍（基础地址自带一个
/// 占位音色，后面又追加了真实音色）。dds 的网关对重复参数直接回 `400 Bad Request`，
/// 于是这些引擎单次朗读也必然失败 —— 不是"音色选错了"，而是请求根本没被接受。
/// 追加的真实音色在末尾，所以"后者覆盖前者"正好还原作者意图。
///
/// 刻意在**字符串层面**处理而不走 `Url::query_pairs` + `query_pairs_mut`：那会先解码
/// 再重新编码，`text=%25E4...` 这类双重编码（模板要求）可能在往返中变形。
fn dedupe_query_params(url: &str) -> String {
    let Some((base, rest)) = url.split_once('?') else {
        return url.to_string();
    };
    let (query, fragment) = match rest.split_once('#') {
        Some((query, fragment)) => (query, Some(fragment)),
        None => (rest, None),
    };

    let mut order: Vec<String> = Vec::new();
    let mut parts: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for segment in query.split('&') {
        if segment.is_empty() {
            continue;
        }
        let key = segment.split_once('=').map_or(segment, |(key, _)| key);
        if parts.insert(key.to_string(), segment.to_string()).is_none() {
            order.push(key.to_string());
        }
    }

    let mut rebuilt = String::with_capacity(url.len());
    rebuilt.push_str(base);
    rebuilt.push('?');
    for (index, key) in order.iter().enumerate() {
        if index > 0 {
            rebuilt.push('&');
        }
        if let Some(segment) = parts.get(key) {
            rebuilt.push_str(segment);
        }
    }
    if let Some(fragment) = fragment {
        rebuilt.push('#');
        rebuilt.push_str(fragment);
    }
    rebuilt
}

fn unsupported_web_view() -> AppError {
    AppError::BadRequest("该朗读引擎依赖 WebView 渲染，桌面版暂不支持".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tts(url: &str) -> HttpTts {
        HttpTts {
            id: 1,
            name: "测试引擎".to_string(),
            url: url.to_string(),
            ..HttpTts::default()
        }
    }

    /// 形态一：纯 URL（含模板）。
    #[test]
    fn splits_plain_url() {
        let raw = "https://dds.dui.ai/runtime/v1/synthesize?voiceId=x&text={{java.encodeURI(java.encodeURI(speakText))}}&audioType=wav";
        let parsed = parse_http_tts_url(raw).unwrap();
        assert_eq!(
            parsed,
            HttpTtsUrlKind::Template {
                url: raw.to_string(),
                config: None,
            }
        );
    }

    /// 形态二：URL + `{"method","body"}` 配置后缀。
    #[test]
    fn splits_url_with_json_config() {
        let raw = r#"http://tsn.baidu.com/text2audio,{"method":"POST","body":"tex={{java.encodeURI(java.encodeURI(speakText))}}&spd={{(speakSpeed + 5) / 10 + 4}}"}"#;
        let parsed = parse_http_tts_url(raw).unwrap();
        let HttpTtsUrlKind::Template { url, config } = parsed else {
            panic!("expected template url");
        };
        assert_eq!(url, "http://tsn.baidu.com/text2audio");
        let config = config.unwrap();
        assert_eq!(config.method, "POST");
        assert_eq!(
            config.body.as_deref(),
            Some("tex={{java.encodeURI(java.encodeURI(speakText))}}&spd={{(speakSpeed + 5) / 10 + 4}}")
        );
        assert!(config.headers.is_empty());
        assert!(!config.web_view);
    }

    /// 地址前半段自身的逗号/查询串不能被误判成配置分隔符。
    #[test]
    fn keeps_commas_inside_url() {
        let raw = "https://example.test/tts?voice=a,b,c&text={{speakText}}";
        let HttpTtsUrlKind::Template { url, config } = parse_http_tts_url(raw).unwrap() else {
            panic!("expected template url");
        };
        assert_eq!(url, raw);
        assert!(config.is_none());
    }

    /// 形态三：`@js:` / `<js>` 脚本地址。
    #[test]
    fn splits_js_url() {
        assert_eq!(
            parse_http_tts_url("@js:'https://example.test/' + speakText").unwrap(),
            HttpTtsUrlKind::Script("'https://example.test/' + speakText".to_string())
        );
        assert_eq!(
            parse_http_tts_url("<js>'https://example.test/a'</js>").unwrap(),
            HttpTtsUrlKind::Script("'https://example.test/a'".to_string())
        );
    }

    /// `,{...}` 存在但不是合法 JSON 时明确报错，而不是当成地址的一部分发出去。
    #[test]
    fn rejects_invalid_json_config() {
        let error = parse_http_tts_url("https://example.test/tts,{method:POST}").unwrap_err();
        assert!(
            error.to_string().contains("不是合法 JSON"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn expands_java_encode_uri_twice() {
        // 思必驰等引擎要求 text 双重编码：第一层编码中文，第二层编码 '%' 本身。
        let out = expand_template("text={{java.encodeURI(java.encodeURI(speakText))}}", "你好 &x", 5)
            .unwrap();
        assert_eq!(out, "text=%25E4%25BD%25A0%25E5%25A5%25BD%2520%2526x");
    }

    #[test]
    fn expands_encode_uri_component_and_parenthesized_forms() {
        assert_eq!(
            expand_template("{{encodeURIComponent(speakText)}}", "你好 &x", 5).unwrap(),
            "%E4%BD%A0%E5%A5%BD%20%26x"
        );
        assert_eq!(
            expand_template("{{(java.encodeURI(speakText))}}", "你好", 5).unwrap(),
            "%E4%BD%A0%E5%A5%BD"
        );
    }

    #[test]
    fn expands_bare_speak_text() {
        assert_eq!(expand_template("q={{speakText}}", "第一句", 5).unwrap(), "q=第一句");
        // 表达式两侧的空格要忽略，否则 JS 会把它们算进字符串比较里。
        assert_eq!(expand_template("{{ speakText }}", "x", 5).unwrap(), "x");
    }

    #[test]
    fn expands_speed_arithmetic() {
        // Legado 语速档位 0~10；模板里的取值全是围绕这个档位做的线性变换。
        assert_eq!(expand_template("{{(speakSpeed + 5) / 10 + 4}}", "t", 5).unwrap(), "5");
        assert_eq!(expand_template("{{(speakSpeed + 4) / 10 + 3}}", "t", 6).unwrap(), "4");
        assert_eq!(expand_template("{{String((speakSpeed + 5) / 10 + 4)}}", "t", 5).unwrap(), "5");
        assert_eq!(expand_template("{{(speakSpeed) * 20 - 400}}", "t", 5).unwrap(), "-300");
        assert_eq!(expand_template("{{speakSpeed + 55}}", "t", 5).unwrap(), "60");
    }

    #[test]
    fn expands_xml_escape_chain() {
        let out = expand_template(
            "{{String(speakText).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')}}",
            "甲<&乙",
            5,
        )
        .unwrap();
        assert_eq!(out, "甲&lt;&amp;乙");
    }

    /// 登录信息在桌面端为空：模板仍要能展开成空串，而不是抛 `get is not a function`。
    #[test]
    fn expands_login_info_map_get() {
        assert_eq!(
            expand_template("appKey={{source.getLoginInfoMap().get('AppKey')}}", "t", 5).unwrap(),
            "appKey="
        );
    }

    /// 回归用例：没有收尾 `}}` 时只能追加 `start..`，不能追加整段 `rest`。
    #[test]
    fn keeps_dangling_open_brace_once() {
        assert_eq!(
            expand_template("abc{{speakText", "x", 5).unwrap(),
            "abc{{speakText"
        );
        // 已展开的前缀 + 半截模板同时存在时，前缀同样不能重复。
        assert_eq!(
            expand_template("{{speakText}}后{{speakSpeed", "x", 5).unwrap(),
            "x后{{speakSpeed"
        );
    }

    #[test]
    fn expands_multiple_and_stray_braces() {
        assert_eq!(expand_template("{{speakText}}|{{speakSpeed}}", "x", 5).unwrap(), "x|5");
        assert_eq!(expand_template("a}}b{{speakText}}", "x", 5).unwrap(), "a}}bx");
        assert_eq!(expand_template("a{{}}b", "x", 5).unwrap(), "ab");
    }

    /// 嵌套花括号：内层 `{{...}}` 不能提前结束外层模板。
    #[test]
    fn handles_nested_braces() {
        assert_eq!(expand_template("{{{{speakText}}}}", "x", 5).unwrap(), "x");
        // 字符串字面量里的大括号同样不能被当成表达式边界。
        assert_eq!(
            expand_template("{{'{{' + speakText + '}}'}}", "x", 5).unwrap(),
            "{{x}}"
        );
    }

    #[test]
    fn renders_baidu_post_request() {
        let tts = tts(r#"http://tsn.baidu.com/text2audio,{"method":"POST","body":"tex={{java.encodeURI(java.encodeURI(speakText))}}&spd={{(speakSpeed + 5) / 10 + 4}}"}"#);
        let rendered = render_http_tts_request(&tts, "你好", 5).unwrap();
        assert_eq!(rendered.url, "http://tsn.baidu.com/text2audio");
        assert_eq!(rendered.method, "POST");
        assert_eq!(
            rendered.body.as_deref(),
            Some("tex=%25E4%25BD%25A0%25E5%25A5%25BD&spd=5")
        );
        // 百度要求表单 Content-Type，配置没给时需要补上。
        assert!(rendered
            .headers
            .iter()
            .any(|(key, value)| key == "Content-Type"
                && value == "application/x-www-form-urlencoded"));
    }

    #[test]
    fn renders_get_request_with_tts_header() {
        let mut tts = tts("https://example.test/tts?text={{encodeURIComponent(speakText)}}");
        // 引擎级默认头 + JSON 对象形态都要能解析。
        tts.header = Some(r#"{"User-Agent":"demo-agent","X-App":"reader"}"#.to_string());
        let rendered = render_http_tts_request(&tts, "你好 世界", 5).unwrap();
        assert_eq!(rendered.url, "https://example.test/tts?text=%E4%BD%A0%E5%A5%BD%20%E4%B8%96%E7%95%8C");
        assert_eq!(rendered.method, "GET");
        assert_eq!(rendered.body, None);
        assert!(rendered
            .headers
            .iter()
            .any(|(key, value)| key == "User-Agent" && value == "demo-agent"));
        // GET 请求不该被补 Content-Type。
        assert!(!rendered
            .headers
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case("content-type")));
    }

    #[test]
    fn config_headers_override_tts_defaults() {
        let mut tts = tts(
            r#"https://example.test/tts,{"headers":{"User-Agent":"config-ua","X-Extra":"1"},"method":"post"}"#,
        );
        tts.header = Some(r#"{"User-Agent":"default-ua","X-Keep":"yes"}"#.to_string());
        let rendered = render_http_tts_request(&tts, "t", 5).unwrap();
        assert_eq!(rendered.method, "POST");
        let user_agent: Vec<&str> = rendered
            .headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case("user-agent"))
            .map(|(_, value)| value.as_str())
            .collect();
        assert_eq!(user_agent, vec!["config-ua"]);
        assert!(rendered
            .headers
            .iter()
            .any(|(key, value)| key == "X-Keep" && value == "yes"));
        assert!(rendered
            .headers
            .iter()
            .any(|(key, value)| key == "X-Extra" && value == "1"));
    }

    /// 引擎级 header 是坏数据（非 JSON 文本）时不能整条失败，地址仍然可用。
    #[test]
    fn tolerates_malformed_tts_header() {
        let mut tts = tts("https://example.test/tts?text={{speakText}}");
        tts.header = Some("this is not a header json".to_string());
        let rendered = render_http_tts_request(&tts, "t", 5).unwrap();
        assert_eq!(rendered.url, "https://example.test/tts?text=t");
    }

    /// 真实集合里 URL 带尾随换行（`...&audioType=wav\n\n`），必须裁掉。
    #[test]
    fn trims_trailing_whitespace_of_url() {
        let tts = tts("https://example.test/tts?text={{speakText}}\n\n");
        assert_eq!(
            render_http_tts_request(&tts, "t", 5).unwrap().url,
            "https://example.test/tts?text=t"
        );
    }

    #[test]
    fn renders_js_url_script() {
        let tts = tts("<js>'https://example.test/tts?text=' + encodeURIComponent(speakText) + '&spd=' + speakSpeed</js>");
        let rendered = render_http_tts_request(&tts, "你好", 5).unwrap();
        assert_eq!(
            rendered.url,
            "https://example.test/tts?text=%E4%BD%A0%E5%A5%BD&spd=5"
        );
    }

    #[test]
    fn renders_js_script_with_config_suffix() {
        let tts = tts(
            r#"@js:'https://example.test/tts,' + JSON.stringify({method:'POST', body:'t=' + speakText})"#,
        );
        let rendered = render_http_tts_request(&tts, "abc", 5).unwrap();
        assert_eq!(rendered.url, "https://example.test/tts");
        assert_eq!(rendered.method, "POST");
        // 脚本产出的 body 是最终值，不应再被模板二次求值。
        assert_eq!(rendered.body.as_deref(), Some("t=abc"));
    }

    #[test]
    fn rejects_empty_text_and_bad_url() {
        assert!(render_http_tts_request(&tts("https://example.test/tts"), "   ", 5).is_err());
        assert!(render_http_tts_request(&tts(""), "t", 5).is_err());
        assert!(render_http_tts_request(&tts("ftp://example.test/tts"), "t", 5).is_err());
        assert!(render_http_tts_request(&tts("{{speakText}}"), "t", 5).is_err());
    }

    /// 协议相对地址补成 https —— 少数集合里这么写。
    #[test]
    fn accepts_protocol_relative_url() {
        let tts = tts("//example.test/tts?text={{speakText}}");
        assert_eq!(
            render_http_tts_request(&tts, "t", 5).unwrap().url,
            "https://example.test/tts?text=t"
        );
    }

    #[test]
    fn rejects_web_view_engines_with_readable_error() {
        let tts = tts(r#"https://example.test/tts,{"webView":true}"#);
        let error = render_http_tts_request(&tts, "t", 5).unwrap_err();
        assert!(error.to_string().contains("WebView"), "{error}");
    }

    /// 在线集合里 71/84 条引擎把 voiceId 写了两遍；dds 网关对重复参数直接 400，
    /// 所以必须去重，且保留**后追加**的那个真实音色。
    #[test]
    fn duplicates_in_query_keep_last_value() {
        let tts = tts(
            "https://example.test/tts?voiceId=placeholder&text={{speakText}}&voiceId=real",
        );
        let url = render_http_tts_request(&tts, "t", 5).unwrap().url;
        assert_eq!(url, "https://example.test/tts?voiceId=real&text=t");
    }

    /// 去重必须在字符串层面完成：双重编码的 text 不能被解码再编码。
    #[test]
    fn dedupe_preserves_double_encoded_text() {
        let tts = tts(
            "https://example.test/tts?voiceId=a&text={{java.encodeURI(java.encodeURI(speakText))}}&voiceId=b",
        );
        let url = render_http_tts_request(&tts, "你好", 5).unwrap().url;
        assert!(url.starts_with("https://example.test/tts?voiceId=b&text=%25E4%25BD%25A0"), "{url}");
        assert_eq!(url.matches("voiceId=").count(), 1, "{url}");
    }

    #[test]
    fn dedupe_keeps_fragment_and_unrelated_params() {
        let tts = tts("https://example.test/tts?a=1&b={{speakText}}&a=2#frag");
        let url = render_http_tts_request(&tts, "t", 5).unwrap().url;
        assert_eq!(url, "https://example.test/tts?a=2&b=t#frag");
    }
}
