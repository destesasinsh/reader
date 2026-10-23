use std::time::Duration;

use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::AppState;
use crate::error::error::{ApiResponse, AppError};
use crate::model::http_tts::HttpTts;
use crate::parser::http_tts_template::render_http_tts_request;
use crate::service::http_tts_service::HttpTtsService;

/// 单机桌面只有 default 命名空间，与其它 command 一致。
const USER_NS: &str = "default";

#[derive(Debug, Deserialize)]
pub struct RemoteHttpTtsParam {
    pub url: String,
}

/// `get_http_tts_list` 的共享实现 (Tauri command 与 Web HTTP handler 共用)。
pub async fn get_http_tts_list_core(state: &AppState) -> Result<ApiResponse<Value>, AppError> {
    let list = state.http_tts_service.list(USER_NS).await?;
    Ok(ApiResponse::ok(
        serde_json::to_value(list).unwrap_or_default(),
    ))
}

/// `get_http_tts_list` 的 Tauri 入口: 只是 core 的薄壳。
#[tauri::command]
pub async fn get_http_tts_list(
    state: tauri::State<'_, AppState>,
) -> Result<ApiResponse<Value>, AppError> {
    get_http_tts_list_core(&state).await
}

/// 保存/导入朗读引擎的共享实现。
///
/// 请求体宽容接受三种形态：裸数组、`{data:[...]}`（在线集合的常见包装）、单个对象
/// （用户手工粘贴一条）。无法反序列化的条目计入 `skipped` —— 导入是一份几十条的
/// 集合，不该因为其中一条脏数据让整份导入失败。
pub async fn save_http_tts_list_core(
    state: &AppState,
    req: Value,
) -> Result<ApiResponse<Value>, AppError> {
    let mut items = Vec::new();
    let mut invalid = 0usize;
    for value in extract_tts_payload(req)? {
        match serde_json::from_value::<HttpTts>(value) {
            Ok(tts) => items.push(tts),
            Err(_) => invalid += 1,
        }
    }
    let summary = state.http_tts_service.save_many(USER_NS, items).await?;
    Ok(ApiResponse::ok(json!({
        "saved": summary.saved,
        "skipped": summary.skipped + invalid,
    })))
}

/// `save_http_tts_list` 的 Tauri 入口: 只是 core 的薄壳。
#[tauri::command]
pub async fn save_http_tts_list(
    state: tauri::State<'_, AppState>,
    req: Value,
) -> Result<ApiResponse<Value>, AppError> {
    save_http_tts_list_core(&state, req).await
}

/// 删除一条的共享实现。`id` 与 `name` 任一命中即可删 —— 前端列表项两个值都有，
/// 传哪个都行。
pub async fn delete_http_tts_core(
    state: &AppState,
    req: Value,
) -> Result<ApiResponse<Value>, AppError> {
    let id = req.get("id").and_then(value_to_i64).unwrap_or(0);
    let name = req
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if id == 0 && name.is_empty() {
        return Err(AppError::BadRequest(
            "请提供要删除的朗读引擎 id 或名称".to_string(),
        ));
    }
    let deleted = state.http_tts_service.delete(USER_NS, id, &name).await?;
    Ok(ApiResponse::ok(json!({ "deleted": deleted })))
}

/// `delete_http_tts` 的 Tauri 入口: 只是 core 的薄壳。
#[tauri::command]
pub async fn delete_http_tts(
    state: tauri::State<'_, AppState>,
    req: Value,
) -> Result<ApiResponse<Value>, AppError> {
    delete_http_tts_core(&state, req).await
}

/// 从直链拉取朗读引擎集合的共享实现。返回形态与 `read_remote_source_file` 一致：
/// `vec[0]` 是整份 `HttpTts[]` 的 JSON 文本，前端 `JSON.parse` 后即可展示/保存。
///
/// 下载走 `api::remote_text`：与净化规则的远程导入共用同一份"边读边计数"的上限
/// 逻辑。这里以前是 `.text()` 一把梭 —— 分块响应能直接把内存吃干。
pub async fn read_remote_http_tts_file_core(
    state: &AppState,
    req: RemoteHttpTtsParam,
) -> Result<ApiResponse<Vec<String>>, AppError> {
    if req.url.trim().is_empty() {
        return Err(AppError::BadRequest(
            "请输入朗读引擎文件的地址".to_string(),
        ));
    }
    let text = crate::api::remote_text::fetch_capped_text(
        &state.book_service.http_client(),
        &req.url,
        crate::api::remote_text::MAX_REMOTE_TEXT_BYTES,
    )
    .await?;

    let payload: Value =
        serde_json::from_str(&text).map_err(|_| invalid_http_tts_json())?;
    let values = extract_tts_payload(payload).map_err(|_| invalid_http_tts_json())?;
    // 只保留能解析的条目：远程集合偶尔混入半截数据，逐条丢弃比整份失败有用。
    let list: Vec<HttpTts> = values
        .into_iter()
        .filter_map(|value| serde_json::from_value::<HttpTts>(value).ok())
        .collect();
    if list.is_empty() {
        return Err(invalid_http_tts_json());
    }
    let json_str = serde_json::to_string(&list)
        .map_err(|error| AppError::BadRequest(format!("序列化朗读引擎失败: {error}")))?;
    Ok(ApiResponse::ok(vec![json_str]))
}

/// `read_remote_http_tts_file` 的 Tauri 入口: 只是 core 的薄壳。
#[tauri::command]
pub async fn read_remote_http_tts_file(
    state: tauri::State<'_, AppState>,
    req: RemoteHttpTtsParam,
) -> Result<ApiResponse<Vec<String>>, AppError> {
    read_remote_http_tts_file_core(&state, req).await
}

/// 合成一句朗读音频的共享实现，返回二进制（前端用 `invokeRaw` 拿 ArrayBuffer
/// 喂给 `URL.createObjectURL`）。
///
/// 必须走后端：WebView 里直接 `fetch` 朗读接口会被 CORS 与混合内容挡住，而且
/// 模板求值依赖后端已有的 rquickjs 环境。
pub async fn synthesize_http_tts_core(
    state: &AppState,
    req: Value,
) -> Result<tauri::ipc::Response, AppError> {
    let request = SynthesizeRequest::from_value(&req)?;
    let client = state.book_service.http_client();
    let bytes = synthesize_segment_bytes(
        &state.http_tts_service,
        &client,
        &request.tts_id,
        &request.text,
        request.speak_speed,
    )
    .await?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// `synthesize_http_tts` 的 Tauri 入口: 只是 core 的薄壳。
#[tauri::command]
pub async fn synthesize_http_tts(
    state: tauri::State<'_, AppState>,
    req: Value,
) -> Result<tauri::ipc::Response, AppError> {
    synthesize_http_tts_core(&state, req).await
}

/// 合成单个片段的音频字节：取引擎 → 渲染模板 → 发请求 → 校验响应。
///
/// 单独成函数是为了让 `synthesize_http_tts` 只做参数解析：缓冲流会高频调用这条
/// 命令，网络与响应校验的细节收在一处，改行为时不容易漏掉某个分支。
async fn synthesize_segment_bytes(
    service: &HttpTtsService,
    client: &reqwest::Client,
    tts_id: &str,
    text: &str,
    speak_speed: i64,
) -> Result<Vec<u8>, AppError> {
    let Some(tts) = service.find(USER_NS, tts_id).await? else {
        return Err(AppError::BadRequest(format!("未找到朗读引擎: {tts_id}")));
    };

    let rendered = render_http_tts_request(&tts, text, speak_speed)?;
    let method = reqwest::Method::from_bytes(rendered.method.as_bytes()).map_err(|_| {
        AppError::BadRequest(format!("不支持的朗读引擎请求方法: {}", rendered.method))
    })?;

    // 尊重引擎自己的 concurrentRate（Legado 的 AnalyzeUrl 同样走 ConcurrentRateLimiter）。
    // 缓冲流默认 2 并发，双音色时是两个引擎交替；不少在线引擎明确写了间隔，
    // 不加限速就是对第三方网关违约，轻则 429、重则封 IP。
    let rate = crate::service::http_tts_throttle::parse_concurrent_rate(
        tts.concurrent_rate.as_deref(),
    );
    if let Some(rate) = rate {
        let throttle_key = format!("httpTts:{}", tts.id);
        // 循环而不是只等一轮：等待期间可能有别的请求把新窗口又占满。
        while let Some(wait) =
            crate::service::http_tts_throttle::acquire(&throttle_key, rate)
        {
            tokio::time::sleep(wait).await;
        }
    }

    // 再按引擎声明的 synthesisThreadCount 限并发。**先取并发许可再发请求**，且
    // 许可必须活到函数结束：它代表"这个引擎此刻有一条在飞"，提前 drop 就等于没限。
    // 与上面的限速是两道独立的闸 —— 写了间隔又写了线程数的引擎两个都要过。
    let slot_key = format!("httpTtsSlot:{}", tts.id);
    let _slot = match crate::service::http_tts_throttle::concurrency_limit(
        tts.synthesis_thread_count,
    ) {
        Some(limit) => crate::service::http_tts_throttle::acquire_slot(&slot_key, limit).await,
        None => None,
    };

    let mut outgoing = client
        .request(method, &rendered.url)
        .timeout(Duration::from_secs(120));
    for (key, value) in &rendered.headers {
        outgoing = outgoing.header(key, value);
    }
    // GET/HEAD 带 body 会被部分网关拒绝；配置里的 body 只对写方法生效。
    if !matches!(rendered.method.as_str(), "GET" | "HEAD") {
        if let Some(body) = rendered.body {
            outgoing = outgoing.body(body);
        }
    }

    let response = outgoing.send().await.map_err(|error| {
        let hint = if error.is_connect() || error.is_timeout() {
            "，请检查网络或 HTTP 代理设置"
        } else {
            ""
        };
        AppError::BadRequest(format!("朗读引擎连接失败: {error}{hint}"))
    })?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| AppError::BadRequest(format!("读取朗读引擎响应失败: {error}")))?;

    if !status.is_success() {
        return Err(AppError::BadRequest(format!(
            "朗读引擎请求失败 ({status}): {}",
            summarize_error_body(&bytes, rendered.charset.as_deref()),
        )));
    }
    if bytes.is_empty() {
        return Err(AppError::BadRequest(
            "朗读引擎返回了空音频".to_string(),
        ));
    }
    if !looks_like_audio(&bytes) {
        // 有些引擎返回裸 PCM 等没有魔数的音频，但会声明 audio/*；魔数识别不了时
        // 以 Content-Type 兜底，避免把正常音频误判成错误页。
        if content_type.starts_with("audio/") {
            return Ok(bytes.to_vec());
        }
        return Err(AppError::BadRequest(format!(
            "朗读引擎返回的不是音频（{}）: {}",
            if content_type.is_empty() {
                "未知类型"
            } else {
                content_type.as_str()
            },
            summarize_error_body(&bytes, rendered.charset.as_deref()),
        )));
    }

    Ok(bytes.to_vec())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SynthesizeRequest {
    tts_id: String,
    text: String,
    /// Legado 的 0~10 语速档位。
    speak_speed: i64,
}

impl SynthesizeRequest {
    fn from_value(req: &Value) -> Result<Self, AppError> {
        let Some(object) = req.as_object() else {
            return Err(AppError::BadRequest(
                "朗读请求参数格式不正确".to_string(),
            ));
        };
        // ttsId 可能是数字（新前端存 number）或字符串（旧配置/手写调用）。
        let tts_id = object
            .get("ttsId")
            .and_then(value_to_text)
            .unwrap_or_default()
            .trim()
            .to_string();
        if tts_id.is_empty() {
            return Err(AppError::BadRequest("请先选择朗读引擎".to_string()));
        }
        let text = object
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if text.trim().is_empty() {
            return Err(AppError::BadRequest("朗读文本不能为空".to_string()));
        }
        let speed = object.get("speed").and_then(value_to_f64);
        let speak_speed = object.get("speakSpeed").and_then(value_to_i64);
        Ok(Self {
            tts_id,
            text,
            speak_speed: resolve_speak_speed(speed, speak_speed),
        })
    }
}

/// 把用户倍速折算成 Legado 的 0~10 语速档位。
///
/// 朗读模板按档位取值（如 `(speakSpeed + 5) / 10 + 4`），而前端存的是倍速
/// （1.0 = 正常速度 = 第 5 档）。所以 `gear = round(rate * 5)`：1.0x→5、
/// 0.6x→3、2.0x→10。显式给了 `speakSpeed` 时以它为准（老调用方/调试用）。
fn resolve_speak_speed(speed: Option<f64>, speak_speed: Option<i64>) -> i64 {
    if let Some(gear) = speak_speed {
        return gear.clamp(0, 10);
    }
    let rate = speed.filter(|rate| rate.is_finite()).unwrap_or(1.0);
    ((rate * 5.0).round() as i64).clamp(0, 10)
}

/// 从保存/导入请求里取出条目数组。
fn extract_tts_payload(payload: Value) -> Result<Vec<Value>, AppError> {
    match payload {
        Value::Array(items) => Ok(items),
        Value::Object(object) => {
            for key in ["data", "list", "httpTtsList", "ttsList"] {
                if let Some(Value::Array(items)) = object.get(key) {
                    return Ok(items.clone());
                }
            }
            // 单条引擎对象（用户手工复制一条时就是这种形态）。
            if object.contains_key("name") || object.contains_key("url") {
                return Ok(vec![Value::Object(object)]);
            }
            Err(AppError::BadRequest("朗读引擎数据格式不正确".to_string()))
        }
        _ => Err(AppError::BadRequest(
            "朗读引擎数据格式不正确".to_string(),
        )),
    }
}

fn invalid_http_tts_json() -> AppError {
    // 文案会直接显示在中文界面上（toast / 导入结果），不要留英文。
    AppError::BadRequest(
        "朗读引擎文件格式不正确：应为 JSON 数组，或含 data 数组的对象".to_string(),
    )
}

fn value_to_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|number| number as i64)),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn value_to_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn value_to_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// 按音频文件头判断响应是不是真音频。
///
/// 朗读接口出错时常常仍返回 HTTP 200（错误页/JSON），只信状态码会把 HTML 当
/// mp3 播出来 —— WebView 那边表现为「没有声音」，很难定位。所以这里按魔数识别
/// wav/mp3/ogg/flac/aiff/m4a。
fn looks_like_audio(bytes: &[u8]) -> bool {
    if bytes.len() >= 12 {
        if &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
            return true;
        }
        if &bytes[0..4] == b"FORM" && (&bytes[8..12] == b"AIFF" || &bytes[8..12] == b"AIFC") {
            return true;
        }
        // MP4 容器（m4a/aac）：第 4..8 字节固定是 ftyp。
        if &bytes[4..8] == b"ftyp" {
            return true;
        }
    }
    if bytes.starts_with(b"ID3") || bytes.starts_with(b"OggS") || bytes.starts_with(b"fLaC") {
        return true;
    }
    // MP3 帧同步：0xFF 后跟 0xE0 掩码置位。没有 ID3 标签的裸 MP3 走这里。
    bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0
}

/// 从错误响应里提炼一句可读的中文提示。
fn summarize_error_body(bytes: &[u8], charset: Option<&str>) -> String {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        for pointer in [
            "/error/message",
            "/error/msg",
            "/error",
            "/err_msg",
            "/errmsg",
            "/detail",
            "/message",
        ] {
            if let Some(candidate) = value.pointer(pointer) {
                let text = candidate
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| candidate.to_string());
                if !text.trim().is_empty() {
                    return truncate_message(&text);
                }
            }
        }
    }
    let text = decode_text(bytes, charset);
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "服务未返回错误详情".to_string()
    } else {
        truncate_message(&collapsed)
    }
}

fn decode_text(bytes: &[u8], charset: Option<&str>) -> String {
    match charset.and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes())) {
        Some(encoding) => {
            let (text, _, _) = encoding.decode(bytes);
            text.into_owned()
        }
        // 没声明编码时按 UTF-8：现代接口与错误页基本都是 UTF-8。
        None => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn truncate_message(message: &str) -> String {
    const MAX_CHARS: usize = 300;
    let mut chars = message.chars();
    let truncated = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_playback_rate_into_legado_gear() {
        // 默认 1.0x 必须落在 Legado 的中档 5，不能是 0。
        assert_eq!(resolve_speak_speed(None, None), 5);
        assert_eq!(resolve_speak_speed(Some(1.0), None), 5);
        assert_eq!(resolve_speak_speed(Some(0.6), None), 3);
        assert_eq!(resolve_speak_speed(Some(0.5), None), 3);
        assert_eq!(resolve_speak_speed(Some(2.0), None), 10);
        // 超出范围要夹住，不能把 0~10 的模板算成负数/两位数。
        assert_eq!(resolve_speak_speed(Some(3.0), None), 10);
        assert_eq!(resolve_speak_speed(Some(-1.0), None), 0);
        assert_eq!(resolve_speak_speed(Some(f64::NAN), None), 5);
        // 显式档位优先，并同样夹到 0~10。
        assert_eq!(resolve_speak_speed(Some(1.0), Some(7)), 7);
        assert_eq!(resolve_speak_speed(None, Some(20)), 10);
        assert_eq!(resolve_speak_speed(None, Some(0)), 0);
    }

    #[test]
    fn parses_synthesize_request_leniently() {
        let request = SynthesizeRequest::from_value(&json!({
            "ttsId": 1652146476375i64,
            "text": "  你好世界  ",
            "speed": 1.2
        }))
        .unwrap();
        assert_eq!(request.tts_id, "1652146476375");
        assert_eq!(request.text, "  你好世界  ");
        assert_eq!(request.speak_speed, 6);

        // 字符串 id + 显式 speakSpeed（老调用方）也要能读。
        let request = SynthesizeRequest::from_value(&json!({
            "ttsId": "abc",
            "text": "x",
            "speakSpeed": "8"
        }))
        .unwrap();
        assert_eq!(request.tts_id, "abc");
        assert_eq!(request.speak_speed, 8);

        assert!(SynthesizeRequest::from_value(&json!({"text": "x"})).is_err());
        assert!(SynthesizeRequest::from_value(&json!({"ttsId": 1, "text": "  "})).is_err());
        assert!(SynthesizeRequest::from_value(&json!([1, 2])).is_err());
    }

    #[test]
    fn extracts_payload_shapes() {
        let items = extract_tts_payload(json!([{"name": "a"}, {"name": "b"}])).unwrap();
        assert_eq!(items.len(), 2);
        let items = extract_tts_payload(json!({"data": [{"name": "a"}]})).unwrap();
        assert_eq!(items.len(), 1);
        let items = extract_tts_payload(json!({"name": "a", "url": "u"})).unwrap();
        assert_eq!(items.len(), 1);
        assert!(extract_tts_payload(json!({"foo": 1})).is_err());
        assert!(extract_tts_payload(json!("text")).is_err());
    }

    #[test]
    fn recognizes_audio_magic_numbers() {
        assert!(looks_like_audio(b"RIFF\x00\x00\x00\x00WAVEfmt "));
        assert!(looks_like_audio(b"ID3\x03\x00\x00\x00"));
        assert!(looks_like_audio(&[0xFF, 0xFB, 0x90, 0x00]));
        assert!(looks_like_audio(b"OggS\x00\x02"));
        assert!(looks_like_audio(b"fLaC\x00\x00\x00\x22"));
        assert!(looks_like_audio(b"FORM\x00\x00\x00\x00AIFF"));
        assert!(looks_like_audio(b"\x00\x00\x00\x20ftypM4A "));
        // 错误页/JSON 不能被当成音频。
        assert!(!looks_like_audio(br#"{"error":"no voice"}"#));
        assert!(!looks_like_audio(b"<html><body>error</body></html>"));
        assert!(!looks_like_audio(b""));
    }

    #[test]
    fn summarizes_json_and_html_errors() {
        assert_eq!(
            summarize_error_body(
                r#"{"error":{"message":"合成失败: 参数错误"}}"#.as_bytes(),
                None
            ),
            "合成失败: 参数错误"
        );
        assert_eq!(
            summarize_error_body(br#"{"message":"rate limited"}"#, None),
            "rate limited"
        );
        assert_eq!(
            summarize_error_body(b"<html>\n  <body>bad gateway</body>\n</html>", None),
            "<html> <body>bad gateway</body> </html>"
        );
        assert_eq!(summarize_error_body(b"  ", None), "服务未返回错误详情");
    }

    /// 声明了 GBK 的错误页要按 GBK 解码，否则中文全是乱码。
    #[test]
    fn decodes_error_body_with_declared_charset() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"error: ");
        // GBK 的「错误」= B4 ED CE F3
        bytes.extend_from_slice(&[0xB4, 0xED, 0xCE, 0xF3]);
        assert!(decode_text(&bytes, Some("gbk")).contains("错误"));
    }
}
