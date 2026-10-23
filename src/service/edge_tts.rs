//! Microsoft Edge 在线朗读（edge-tts）后端通道。
//!
//! 注意：这不是 HTTP API —— 合成走 WebSocket，而且没有 API key，靠 `Sec-MS-GEC`
//! token 做反滥用校验（算法见 `edge_tts_drm`，已按参考实现逐位移植）。这是微软
//! 给 Edge「大声朗读」用的**非官方公共接口**，随时可能调整，所以对外错误一律
//! 折成可读中文并提示改用其它朗读通道，绝不把英文握手错误抛给用户。
//!
//! 协议参考：本机安装的 `edge_tts` 7.3.1（`communicate.py` / `constants.py`）。

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::{client::ClientRequestBuilder, Message};

use crate::error::error::AppError;use crate::service::edge_tts_drm;

const BASE_URL: &str = "speech.platform.bing.com/consumer/speech/synthesize/readaloud";
const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const CHROMIUM_MAJOR_VERSION: &str = "143";
/// 服务端按这个版本号校验 token 算法，必须与参考实现保持一致
/// （参考实现是 `2-{CHROMIUM_FULL_VERSION}`，即 `2-143.0.3650.75`）。
const SEC_MS_GEC_VERSION: &str = "2-143.0.3650.75";
/// Edge readaloud 端点**只接受**这两种 `outputFormat`。
///
/// 实测被服务端拒绝（NoAudioReceived）的还有：48kHz 系列、16/32/64/128/160kbps、
/// PCM/WAV 等，所以音质不是自由参数，只能在这两档里选。
pub const OUTPUT_FORMAT_48K: &str = "audio-24khz-48kbitrate-mono-mp3";
/// 高音质档：同句字节数正好是 48k 的两倍（24kHz 采样率不变，只是码率翻倍）。
pub const OUTPUT_FORMAT_96K: &str = "audio-24khz-96kbitrate-mono-mp3";

/// 把调用方给的音质参数归一化成端点接受的 outputFormat。
///
/// 接受三档写法：简写（`96k` / `96Kbps` / `96`）、完整格式串
/// （`audio-24khz-96kbitrate-mono-mp3`）。**非法值一律回退 48k**：音质只是体验
/// 偏好，不能因为前端传了个历史脏值就让整句朗读失败（服务端会直接 NoAudioReceived）。
pub fn normalize_output_format(requested: Option<&str>) -> &'static str {
    let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
        return OUTPUT_FORMAT_48K;
    };
    let lowered = requested.to_ascii_lowercase();
    if lowered == OUTPUT_FORMAT_96K {
        return OUTPUT_FORMAT_96K;
    }
    if lowered == OUTPUT_FORMAT_48K {
        return OUTPUT_FORMAT_48K;
    }
    // 简写形态：`96k` / `96kbps` / `96`（也涵盖 `48k` 等）。
    let digits = lowered
        .strip_suffix("kbps")
        .or_else(|| lowered.strip_suffix('k'))
        .unwrap_or(lowered.as_str())
        .trim();
    match digits.parse::<u32>() {
        Ok(96) => OUTPUT_FORMAT_96K,
        // 48 / 48k / 48kbps 以及任何非法值都走这里。
        _ => OUTPUT_FORMAT_48K,
    }
}
/// Edge 扩展的 Origin，服务端会校验它像不像官方客户端。
const ORIGIN: &str = "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold";
/// 手写 UA 比 reqwest 默认 UA 更像 Edge 浏览器（服务端对 UA 有过滤）。
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36 Edg/143.0.0.0";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

fn wss_url() -> String {
    format!("wss://{BASE_URL}/edge/v1?TrustedClientToken={TRUSTED_CLIENT_TOKEN}")
}

fn voice_list_url() -> String {
    format!("https://{BASE_URL}/voices/list?trustedclienttoken={TRUSTED_CLIENT_TOKEN}")
}

// ───────────────────────────── 错误文案 ─────────────────────────────

/// 统一的「通道不可用」错误：中文说明 + 明确的替代方案。
///
/// 微软调整校验时，用户拿到的必须是「换通道」而不是一串 TLS/握手英文错误。
fn channel_unavailable(detail: impl std::fmt::Display) -> AppError {
    AppError::BadRequest(format!(
        "Edge 朗读服务暂时不可用（{detail}）。可能是微软调整了校验，请改用其它朗读通道。"
    ))
}

fn http_status_error(action: &str, status: reqwest::StatusCode) -> AppError {
    let detail = if status == reqwest::StatusCode::FORBIDDEN {
        format!("{action}被拒绝（403）")
    } else {
        format!("{action}返回 HTTP {status}")
    };
    channel_unavailable(detail)
}

fn http_transport_error(action: &str, error: &reqwest::Error) -> AppError {
    tracing::warn!("edge tts http error during {action}: {error:?}");
    let detail = if error.is_timeout() {
        format!("{action}连接超时")
    } else if error.is_connect() {
        format!("{action}无法建立连接")
    } else {
        format!("{action}网络请求失败")
    };
    channel_unavailable(detail)
}

fn websocket_error(error: tokio_tungstenite::tungstenite::Error) -> AppError {
    use tokio_tungstenite::tungstenite::Error as WsError;
    tracing::warn!("edge tts websocket error: {error:?}");
    let detail = match &error {
        WsError::Http(response) => match response.status().as_u16() {
            403 => "WebSocket 握手被拒绝（403）".to_string(),
            status => format!("WebSocket 握手返回 HTTP {status}"),
        },
        WsError::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => {
            "WebSocket 连接超时".to_string()
        }
        WsError::Url(_) => "服务地址无效".to_string(),
        _ => "WebSocket 连接中断".to_string(),
    };
    channel_unavailable(detail)
}

// ───────────────────────────── 请求构造 ─────────────────────────────

/// 32 位大写 hex 的 MUID，用作 `Cookie: muid=...`（服务端会话标识，每次新建即可）。
fn generate_muid() -> String {
    uuid::Uuid::new_v4().simple().to_string().to_ascii_uppercase()
}

/// 32 位小写 hex 的 ConnectionId（uuid4 去横线，与参考实现相同）。
fn connection_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// 构造 WSS 连接地址：token 与版本号都放在 query 上。
fn connection_url(connection_id: &str, sec_ms_gec: &str) -> String {
    format!(
        "{}&ConnectionId={connection_id}&Sec-MS-GEC={sec_ms_gec}&Sec-MS-GEC-Version={SEC_MS_GEC_VERSION}",
        wss_url()
    )
}

/// Edge 协议要求的日期格式（JS `Date.toString()` 风格，UTC）。
///
/// 参考实现直接在末尾再拼一个 `Z`（见 `ssml_headers_plus_data` 的注释：
/// "Microsoft Edge bug"），这里保持同样的畸形格式，服务端只认这个。
fn format_timestamp(now: chrono::DateTime<chrono::Utc>) -> String {
    now.format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)")
        .to_string()
}

/// 第一帧：告诉服务端要什么音频格式、不要字幕边界元数据。
///
/// `output_format` 应来自 [`normalize_output_format`]（端点只认白名单里的两种）。
fn build_speech_config_frame(timestamp: &str, output_format: &str) -> String {
    // 用 replace 而不是 format! 插格式串：这段 JSON 花括号很多，format! 的
    // `{{`/`}}` 转义极易数错，一旦多/少一个括号服务端就直接断开。
    let payload = r#"{"context":{"synthesis":{"audio":{"metadataoptions":{"sentenceBoundaryEnabled":"false","wordBoundaryEnabled":"false"},"outputFormat":"__OUTPUT_FORMAT__"}}}}"#
        .replace("__OUTPUT_FORMAT__", output_format);
    format!(
        "X-Timestamp:{timestamp}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n{payload}\r\n"
    )
}

/// 第二帧：SSML。`timestamp` 后同样要拼 `Z`（Edge 服务端的畸形要求）。
fn build_ssml_frame(request_id: &str, timestamp: &str, ssml: &str) -> String {
    format!(
        "X-RequestId:{request_id}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{timestamp}Z\r\nPath:ssml\r\n\r\n{ssml}"
    )
}

// ───────────────────────────── SSML ─────────────────────────────

/// 服务端不支持的字符区间：垂直制表符等常见于 OCR 出来的 PDF 正文，不清掉会直接
/// 报错。区间与参考实现的 `remove_incompatible_characters` 一致（保留 \t \n \r）。
pub fn remove_incompatible_characters(text: &str) -> String {
    text.chars()
        .map(|ch| {
            let code = ch as u32;
            if (0..=8).contains(&code) || (11..=12).contains(&code) || (14..=31).contains(&code) {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

/// XML 文本节点转义。`&` 必须最先替换，否则会把后面生成的实体再转义一次。
pub fn escape_ssml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn escape_ssml_attribute(value: &str) -> String {
    value.replace('&', "&amp;").replace('\'', "&apos;")
}

/// 从 voice shortName 推出 BCP-47 locale（`zh-CN-XiaoxiaoNeural` → `zh-CN`）。
///
/// 前端只传音色名，而 SSML 的 `xml:lang` 需要 locale；音色名前两段就是语言+地区。
pub fn voice_locale(voice: &str) -> String {
    let mut parts = voice.trim().split('-');
    match (parts.next(), parts.next()) {
        (Some(language), Some(region)) if !language.is_empty() && !region.is_empty() => {
            format!("{language}-{region}")
        }
        _ => "en-US".to_string(),
    }
}

/// 生成完整 SSML。`rate`/`pitch`/`volume` 已是 `+0%` / `+0Hz` 形态。
pub fn build_ssml(
    voice: &str,
    rate: &str,
    pitch: &str,
    volume: &str,
    text: &str,
) -> String {
    let locale = voice_locale(voice);
    let escaped_text = escape_ssml(&remove_incompatible_characters(text));
    format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='{}'><voice name='{}'><prosody pitch='{}' rate='{}' volume='{}'>{escaped_text}</prosody></voice></speak>",
        escape_ssml_attribute(&locale),
        escape_ssml_attribute(voice.trim()),
        escape_ssml_attribute(pitch),
        escape_ssml_attribute(rate),
        escape_ssml_attribute(volume),
    )
}

// ───────────────────────────── 帧解析 ─────────────────────────────

/// 从形如 `Key:value\r\nKey:value` 的头区里取某个头（大小写不敏感）。
fn header_value<'a>(headers: &'a [u8], name: &str) -> Option<&'a str> {
    for line in headers.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(colon) = line.iter().position(|byte| *byte == b':') else {
            continue;
        };
        let Ok(key) = std::str::from_utf8(&line[..colon]) else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case(name) {
            return std::str::from_utf8(&line[colon + 1..]).ok().map(str::trim);
        }
    }
    None
}

/// 解析二进制音频帧：
/// `[2 字节大端头长度 N][N-2 字节头][\r\n][音频字节...]`。
///
/// 返回 `Ok(None)` 表示这是服务端结束时的空帧（无 Content-Type 也没有数据），
/// 正常音频返回 `Ok(Some(slice))`。头长度越界或 Path 不对时报错。
pub fn parse_audio_frame(frame: &[u8]) -> Result<Option<&[u8]>, AppError> {
    if frame.len() < 2 {
        return Err(channel_unavailable("音频帧过短"));
    }
    let header_length = u16::from_be_bytes([frame[0], frame[1]]) as usize;
    // 头长度包含自身 2 字节，且后面还要有 2 字节的空行 CRLF，所以最小值是 2。
    if header_length < 2 || header_length + 2 > frame.len() {
        return Err(channel_unavailable("音频帧头长度越界"));
    }
    let headers = &frame[2..header_length];
    let audio = &frame[header_length + 2..];
    if !header_value(headers, "Path").is_some_and(|path| path == "audio") {
        return Err(channel_unavailable("音频帧缺少 Path:audio"));
    }
    match header_value(headers, "Content-Type") {
        Some(content_type) if content_type.eq_ignore_ascii_case("audio/mpeg") => {
            if audio.is_empty() {
                return Err(channel_unavailable("音频帧没有音频数据"));
            }
            Ok(Some(audio))
        }
        Some(_) => Err(channel_unavailable("音频帧类型不是 audio/mpeg")),
        // 结束时服务端会发一个没有 Content-Type 的空帧，只有空数据才允许跳过。
        None if audio.is_empty() => Ok(None),
        None => Err(channel_unavailable("音频帧缺少 Content-Type")),
    }
}

/// 解析文本帧的头区，返回 `Path` 值（`turn.end` 表示本轮合成结束）。
pub fn parse_text_frame_path(frame: &[u8]) -> Option<String> {
    let separator = frame.windows(4).position(|window| window == b"\r\n\r\n")?;
    header_value(&frame[..separator], "Path").map(str::to_string)
}

// ───────────────────────────── 音色列表 ─────────────────────────────

/// 解析音色列表 JSON（`/voices/list` 的响应）并转成前端要的字段。
///
/// 服务端原始字段是 PascalCase（`ShortName`/`Locale`/`Gender`/`FriendlyName`），
/// 这里映射成 camelCase；排序规则：zh-CN 优先，其余按 locale 升序，同 locale 按
/// shortName 升序（前端下拉需要稳定顺序）。
pub fn parse_voice_list(payload: &[u8]) -> Result<Vec<Value>, AppError> {
    let data: Value = serde_json::from_slice(payload)
        .map_err(|error| channel_unavailable(format!("音色列表解析失败（{error}）")))?;
    let Some(items) = data.as_array() else {
        return Err(channel_unavailable("音色列表格式不正确"));
    };
    let mut voices: Vec<Value> = items
        .iter()
        .filter_map(|item| {
            let short_name = item.get("ShortName").and_then(Value::as_str)?.trim();
            if short_name.is_empty() {
                return None;
            }
            Some(json!({
                "shortName": short_name,
                "locale": item.get("Locale").and_then(Value::as_str).unwrap_or_default(),
                "gender": item.get("Gender").and_then(Value::as_str).unwrap_or_default(),
                "friendlyName": item.get("FriendlyName").and_then(Value::as_str).unwrap_or_default(),
            }))
        })
        .collect();
    if voices.is_empty() {
        return Err(channel_unavailable("音色列表为空"));
    }
    voices.sort_by(|left, right| {
        let left_locale = left["locale"].as_str().unwrap_or_default();
        let right_locale = right["locale"].as_str().unwrap_or_default();
        // zh-CN 排最前：中文朗读是主要使用场景。
        (left_locale != "zh-CN")
            .cmp(&(right_locale != "zh-CN"))
            .then_with(|| left_locale.cmp(right_locale))
            .then_with(|| {
                left["shortName"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["shortName"].as_str().unwrap_or_default())
            })
    });
    Ok(voices)
}

/// 进程内音色缓存：成功后缓存一天（见 `service::voice_cache`），失败不缓存。
///
/// 早先只缓存不设期限 —— 服务端加了新音色，用户必须重启才看得到。
static VOICE_CACHE: crate::service::voice_cache::VoiceCache =
    crate::service::voice_cache::VoiceCache::new();

/// 拉取音色列表（带进程内缓存）。`client` 由命令层传入（复用应用的 HTTP 代理配置）。
pub async fn list_voices(
    client: &reqwest::Client,
) -> Result<Vec<Value>, AppError> {
    if let Some(cached) = VOICE_CACHE.fresh() {
        return Ok(cached);
    }

    let url = format!(
        "{}&Sec-MS-GEC={}&Sec-MS-GEC-Version={SEC_MS_GEC_VERSION}",
        voice_list_url(),
        edge_tts_drm::generate_sec_ms_gec()
    );
    let response = client
        .get(&url)
        .timeout(REQUEST_TIMEOUT)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "*/*")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header(
            "Sec-CH-UA",
            format!(
                "\" Not;A Brand\";v=\"99\", \"Microsoft Edge\";v=\"{CHROMIUM_MAJOR_VERSION}\", \"Chromium\";v=\"{CHROMIUM_MAJOR_VERSION}\""
            ),
        )
        .header("Sec-CH-UA-Mobile", "?0")
        .header("Sec-Fetch-Site", "none")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Dest", "empty")
        .header("Cookie", format!("muid={};", generate_muid()))
        .send()
        .await
        .map_err(|error| http_transport_error("获取音色列表", &error))?;

    let status = response.status();
    if !status.is_success() {
        // 刷新失败：退回旧列表（可能只是临时断网/被限流），旧音色照样能用。
        if let Some(stale) = VOICE_CACHE.any() {
            tracing::warn!("刷新 Edge 音色列表失败（{status}），继续使用缓存");
            return Ok(stale);
        }
        return Err(http_status_error("获取音色列表", status));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| http_transport_error("读取音色列表", &error))?;
    let voices = parse_voice_list(&bytes)?;

    VOICE_CACHE.store(voices.clone());
    Ok(voices)
}

// ───────────────────────────── 合成 ─────────────────────────────

/// 建立 WebSocket 连接（可经 HTTP 代理）。
///
/// `tokio_tungstenite::connect_async` 内部用的是裸 `TcpStream::connect`，完全绕过
/// 应用配置的代理 —— 结果是「音色列表能拉（走 reqwest）、合成必失败（走直连）」。
/// 这里自己建 TCP：有代理时先发 CONNECT 建立隧道，再把手里的流交给
/// `client_async_tls_with_config` 完成 TLS 与 WebSocket 握手。
async fn connect_websocket(
    request: ClientRequestBuilder,
    endpoint: (String, u16),
    proxy: Option<&str>,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ),
    tokio_tungstenite::tungstenite::Error,
> {
    use tokio::io::AsyncWriteExt;
    use tokio_tungstenite::tungstenite::Error as WsError;

    // `ClientRequestBuilder` 不暴露 uri 读取接口，所以 host/port 由调用方传进来
    // （那里刚从同一份 url 解析过）。
    let (host, port) = endpoint;

    let Some(proxy) = proxy.map(str::trim).filter(|value| !value.is_empty()) else {
        let stream = tokio::net::TcpStream::connect((host.as_str(), port))
            .await
            .map_err(WsError::Io)?;
        return tokio_tungstenite::client_async_tls_with_config(request, stream, None, None).await;
    };

    // 代理只支持 http:// 形式（设置页限制如此）。解析失败时报错，不静默退回直连 ——
    // 用户配了代理却悄悄直连，会让"代理是否生效"变得无法判断。
    let proxy_url = url::Url::parse(proxy).map_err(|_| {
        WsError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("代理地址无效: {proxy}"),
        ))
    })?;
    let proxy_host = proxy_url.host_str().unwrap_or_default().to_string();
    if proxy_host.is_empty() {
        return Err(WsError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "代理地址缺少主机名",
        )));
    }
    let proxy_port = proxy_url.port_or_known_default().unwrap_or(8080);
    let mut stream = tokio::net::TcpStream::connect((proxy_host.as_str(), proxy_port))
        .await
        .map_err(WsError::Io)?;

    // HTTP CONNECT 建隧道。注意 `Host` 里要带端口，否则部分代理会连到 80。
    let authority = format!("{host}:{port}");
    let connect_request = format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Connection: keep-alive\r\n\r\n"
    );
    stream
        .write_all(connect_request.as_bytes())
        .await
        .map_err(WsError::Io)?;

    // 只读到响应头结束：之后的字节属于 TLS 握手，多读一个字节都会破坏它。
    let mut response = Vec::with_capacity(128);
    let mut byte = [0u8; 1];
    loop {
        use tokio::io::AsyncReadExt;
        let read = stream.read(&mut byte).await.map_err(WsError::Io)?;
        if read == 0 {
            return Err(WsError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "代理在 CONNECT 阶段断开",
            )));
        }
        response.push(byte[0]);
        if response.ends_with(b"\r\n\r\n") {
            break;
        }
        if response.len() > 8 * 1024 {
            return Err(WsError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "代理返回的响应头过长",
            )));
        }
    }
    let head = String::from_utf8_lossy(&response);
    let status_ok = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .map(|code| code == "200")
        .unwrap_or(false);
    if !status_ok {
        return Err(WsError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            format!("代理拒绝了 CONNECT 请求: {}", head.lines().next().unwrap_or_default()),
        )));
    }

    tokio_tungstenite::client_async_tls_with_config(request, stream, None, None).await
}

/// 用 Edge 朗读合成一段文本，返回 mp3 字节。
///
/// 单轮协议：speech.config → SSML → 收音频帧直到 `turn.end`。整段文本作为一轮
/// 发送（参考实现会按 4096 字节切分多轮；朗读缓冲流每次传一句，远小于该上限，
/// 所以这里不做切分）。
///
/// `output_format` 是调用方请求的音质；非法值在内部回退 48k（见
/// [`normalize_output_format`]），不会因为音质参数让朗读失败。
/// `proxy` 是应用设置里的 HTTP 代理（`None` 表示直连），见 [`connect_websocket`]。
pub async fn synthesize(
    text: &str,
    voice: &str,
    rate: &str,
    pitch: &str,
    volume: &str,
    output_format: &str,
    proxy: Option<&str>,
) -> Result<Vec<u8>, AppError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::BadRequest("朗读文本为空".to_string()));
    }
    // 纯标点/纯引号时服务端本来就不返回音频（正常行为），不能让它一路走到
    // 「没有收到音频数据」而把用户引向「换通道」。这里在发请求前给出准确错误。
    if !crate::util::speakable_text::has_speakable_content(text) {
        return Err(AppError::BadRequest(
            "该片段没有可朗读的内容".to_string(),
        ));
    }
    if voice.trim().is_empty() {
        return Err(AppError::BadRequest("请先选择 Edge 朗读音色".to_string()));
    }
    let output_format = normalize_output_format(Some(output_format));

    let url = connection_url(
        &connection_id(),
        &edge_tts_drm::generate_sec_ms_gec(),
    );
    let uri = url
        .parse::<http::Uri>()
        .map_err(|_| channel_unavailable("服务地址无效"))?;
    let host = uri
        .host()
        .ok_or_else(|| channel_unavailable("服务地址缺少主机名"))?
        .to_string();
    let port = uri
        .port_u16()
        .unwrap_or(if uri.scheme_str() == Some("wss") { 443 } else { 80 });
    let builder = ClientRequestBuilder::new(uri)
        .with_header("User-Agent", USER_AGENT)
        .with_header("Origin", ORIGIN)
        .with_header("Pragma", "no-cache")
        .with_header("Cache-Control", "no-cache")
        .with_header("Accept-Language", "en-US,en;q=0.9")
        .with_header("Cookie", format!("muid={};", generate_muid()));

    let timestamp = format_timestamp(chrono::Utc::now());
    let speech_config = build_speech_config_frame(&timestamp, output_format);
    let ssml = build_ssml(voice, rate, pitch, volume, text);
    let ssml_frame = build_ssml_frame(&connection_id(), &timestamp, &ssml);

    let audio = tokio::time::timeout(REQUEST_TIMEOUT, async move {
        let (mut socket, _response) = connect_websocket(builder, (host, port), proxy.as_deref())
            .await
            .map_err(websocket_error)?;
        socket
            .send(Message::text(speech_config))
            .await
            .map_err(websocket_error)?;
        socket
            .send(Message::text(ssml_frame))
            .await
            .map_err(websocket_error)?;

        let mut audio: Vec<u8> = Vec::new();
        while let Some(message) = socket.next().await {
            let message = message.map_err(websocket_error)?;
            match message {
                Message::Text(text) => {
                    if parse_text_frame_path(text.as_bytes()).as_deref() == Some("turn.end") {
                        break;
                    }
                }
                Message::Binary(frame) => {
                    if let Some(chunk) = parse_audio_frame(&frame)? {
                        audio.extend_from_slice(chunk);
                    }
                }
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
        Ok::<Vec<u8>, AppError>(audio)
    })
    .await
    .map_err(|_| channel_unavailable("合成超时（30 秒）"))??;

    if audio.is_empty() {
        return Err(channel_unavailable("没有收到音频数据"));
    }
    Ok(audio)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOICE_FIXTURE: &str = r#"[
      {
        "Name": "Microsoft Server Speech Text to Speech Voice (en-US, JennyNeural)",
        "ShortName": "en-US-JennyNeural",
        "Gender": "Female",
        "Locale": "en-US",
        "FriendlyName": "Microsoft Jenny Online (Natural) - English (United States)",
        "Status": "GA",
        "VoiceTag": {"ContentCategories": ["News"], "VoicePersonalities": ["Friendly"]}
      },
      {
        "Name": "Microsoft Server Speech Text to Speech Voice (zh-CN, XiaoxiaoNeural)",
        "ShortName": "zh-CN-XiaoxiaoNeural",
        "Gender": "Female",
        "Locale": "zh-CN",
        "FriendlyName": "Microsoft Xiaoxiao Online (Natural) - Chinese (Mainland)"
      },
      {
        "Name": "Microsoft Server Speech Text to Speech Voice (ja-JP, NanamiNeural)",
        "ShortName": "ja-JP-NanamiNeural",
        "Gender": "Female",
        "Locale": "ja-JP",
        "FriendlyName": "Microsoft Nanami Online (Natural) - Japanese (Japan)"
      },
      {
        "Name": "Microsoft Server Speech Text to Speech Voice (zh-CN, YunxiNeural)",
        "ShortName": "zh-CN-YunxiNeural",
        "Gender": "Male",
        "Locale": "zh-CN",
        "FriendlyName": "Microsoft Yunxi Online (Natural) - Chinese (Mainland)"
      }
    ]"#;

    #[test]
    fn parses_voice_list_fields_and_sorts_zh_cn_first() {
        let voices = parse_voice_list(VOICE_FIXTURE.as_bytes()).unwrap();
        assert_eq!(voices.len(), 4);
        let short_names: Vec<&str> = voices
            .iter()
            .map(|voice| voice["shortName"].as_str().unwrap())
            .collect();
        // zh-CN 优先，组内按 shortName；其余按 locale 升序。
        assert_eq!(
            short_names,
            vec![
                "zh-CN-XiaoxiaoNeural",
                "zh-CN-YunxiNeural",
                "en-US-JennyNeural",
                "ja-JP-NanamiNeural",
            ]
        );
        let first = &voices[0];
        assert_eq!(first["locale"], "zh-CN");
        assert_eq!(first["gender"], "Female");
        assert_eq!(
            first["friendlyName"],
            "Microsoft Xiaoxiao Online (Natural) - Chinese (Mainland)"
        );
    }

    #[test]
    fn rejects_invalid_voice_payloads() {
        assert!(parse_voice_list(b"not json").is_err());
        assert!(parse_voice_list(b"{}").is_err());
        assert!(parse_voice_list(b"[]").is_err());
        // 没有 ShortName 的条目被跳过；全被跳过时整体报错。
        assert!(parse_voice_list(br#"[{"Locale":"zh-CN"}]"#).is_err());
    }

    #[test]
    fn builds_ssml_with_escaping_and_params() {
        let ssml = build_ssml("zh-CN-XiaoxiaoNeural", "+10%", "-5Hz", "-20%", "甲<&乙\"丙'丁");
        assert!(ssml.starts_with(
            "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='zh-CN'>"
        ));
        assert!(ssml.contains("<voice name='zh-CN-XiaoxiaoNeural'>"));
        assert!(ssml.contains("pitch='-5Hz' rate='+10%' volume='-20%'"));
        assert!(ssml.contains("甲&lt;&amp;乙&quot;丙&apos;丁"));
        assert!(ssml.ends_with("</prosody></voice></speak>"));
    }

    #[test]
    fn replaces_incompatible_control_characters() {
        // 0-8 / 11-12 / 14-31 换成空格；\t(9) \n(10) \r(13) 保留。
        let cleaned = remove_incompatible_characters("a\u{0}b\u{8}c\u{b}d\u{c}e\u{1f}f\tg\nh\ri");
        assert_eq!(cleaned, "a b c d e f\tg\nh\ri");
        // 先清控制字符再转义，顺序不能反（否则实体里的 & 会被当文本）。
        let ssml = build_ssml("en-US-AriaNeural", "+0%", "+0Hz", "+0%", "a\u{0}<b>");
        assert!(ssml.contains("a &lt;b&gt;"));
    }

    #[test]
    fn derives_locale_from_voice_short_name() {
        assert_eq!(voice_locale("zh-CN-XiaoxiaoNeural"), "zh-CN");
        assert_eq!(voice_locale("en-US-EmmaMultilingualNeural"), "en-US");
        assert_eq!(voice_locale("zh-TW-HsiaoChenNeural"), "zh-TW");
        // 异常输入退化成默认 locale，而不是生成非法 SSML。
        assert_eq!(voice_locale("broken"), "en-US");
        assert_eq!(voice_locale(""), "en-US");
    }

    #[test]
    fn normalizes_output_format_to_whitelist() {
        // 两档都支持：简写 / 带单位 / 完整串 / 大小写混写。
        for requested in [
            "96k",
            "96K",
            "96Kbps",
            "96kbps",
            "96",
            " audio-24khz-96kbitrate-mono-mp3 ",
            "AUDIO-24KHZ-96KBITRATE-MONO-MP3",
        ] {
            assert_eq!(
                normalize_output_format(Some(requested)),
                OUTPUT_FORMAT_96K,
                "requested = {requested}"
            );
        }
        for requested in ["48k", "48Kbps", "48", OUTPUT_FORMAT_48K, "0"] {
            assert_eq!(
                normalize_output_format(Some(requested)),
                OUTPUT_FORMAT_48K,
                "requested = {requested}"
            );
        }
        // 缺失 / 空白 / 白名单之外的值（48kHz、16/32/64/128/160kbps、PCM/WAV…）
        // 全部回退 48k：音质参数不该让朗读失败。
        for requested in [
            None,
            Some(""),
            Some("   "),
            Some("44.1k"),
            Some("16k"),
            Some("32k"),
            Some("64k"),
            Some("128k"),
            Some("160k"),
            Some("48khz"),
            Some("audio-24khz-48kbitrate-mono-pcm"),
            Some("audio-48khz-96kbitrate-mono-mp3"),
            Some("garbage"),
        ] {
            assert_eq!(
                normalize_output_format(requested),
                OUTPUT_FORMAT_48K,
                "requested = {requested:?}"
            );
        }
    }

    fn speech_config_output_format(frame: &str) -> String {
        let payload = frame
            .split("\r\n\r\n")
            .nth(1)
            .expect("speech.config 必须有头体分隔")
            .trim();
        let value: Value = serde_json::from_str(payload).expect("speech.config 必须是合法 JSON");
        value["context"]["synthesis"]["audio"]["outputFormat"]
            .as_str()
            .expect("outputFormat 必须是字符串")
            .to_string()
    }

    #[test]
    fn builds_speech_config_frame() {
        let timestamp = "Wed Oct 20 2026 11:32:00 GMT+0000 (Coordinated Universal Time)";
        let frame = build_speech_config_frame(timestamp, OUTPUT_FORMAT_48K);
        assert!(frame.starts_with("X-Timestamp:Wed Oct 20 2026"));
        assert!(frame.contains("\r\nContent-Type:application/json; charset=utf-8\r\n"));
        assert!(frame.contains("\r\nPath:speech.config\r\n\r\n"));
        assert!(frame.ends_with("\r\n"));

        // 体必须是合法 JSON 且字段齐全 —— 花括号数错一个服务端就直接断开。
        let payload = frame
            .split("\r\n\r\n")
            .nth(1)
            .expect("speech.config 必须有头体分隔")
            .trim();
        let value: Value = serde_json::from_str(payload).expect("speech.config 必须是合法 JSON");
        let audio = &value["context"]["synthesis"]["audio"];
        assert_eq!(audio["outputFormat"], OUTPUT_FORMAT_48K);
        assert_eq!(audio["metadataoptions"]["sentenceBoundaryEnabled"], "false");
        assert_eq!(audio["metadataoptions"]["wordBoundaryEnabled"], "false");

        // 96k 档必须原样替换 outputFormat，不能还写死 48k。
        let hq = build_speech_config_frame(timestamp, OUTPUT_FORMAT_96K);
        assert_eq!(speech_config_output_format(&hq), OUTPUT_FORMAT_96K);
        assert_eq!(speech_config_output_format(&frame), OUTPUT_FORMAT_48K);
    }

    #[test]
    fn builds_ssml_frame_headers() {
        let frame = build_ssml_frame("abc123", "Wed Oct 20 2026 11:32:00 GMT+0000 (Coordinated Universal Time)", "<speak/>");
        assert!(frame.starts_with("X-RequestId:abc123\r\n"));
        assert!(frame.contains("\r\nContent-Type:application/ssml+xml\r\n"));
        // 参考实现会在时间戳后再拼一个 Z（微软的畸形要求），不能"修正"掉。
        assert!(frame.contains("(Coordinated Universal Time)Z\r\n"));
        assert!(frame.ends_with("\r\n\r\n<speak/>"));
    }

    #[test]
    fn builds_connection_url_with_drm_params() {
        let url = connection_url("abc", "TOKEN");
        assert!(url.starts_with("wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1?TrustedClientToken="));
        assert!(url.contains("&ConnectionId=abc"));
        assert!(url.contains("&Sec-MS-GEC=TOKEN"));
        assert!(url.contains("&Sec-MS-GEC-Version=2-143.0.3650.75"));
    }

    /// 真实抓到的二进制帧形态：`[u16be 头长+2][头][\r\n][音频]`。
    #[test]
    fn parses_binary_audio_frame() {
        let headers = b"X-RequestId:abc\r\nContent-Type:audio/mpeg\r\nX-StreamId:XYZ\r\nPath:audio";
        let audio = [0xFF, 0xF3, 0x64, 0xC4, 0x00, 0x00];
        let mut frame = Vec::new();
        frame.extend_from_slice(&((headers.len() + 2) as u16).to_be_bytes());
        frame.extend_from_slice(headers);
        frame.extend_from_slice(b"\r\n");
        frame.extend_from_slice(&audio);
        assert_eq!(parse_audio_frame(&frame).unwrap(), Some(audio.as_slice()));

        // 结束时服务端的空帧（无 Content-Type、无数据）应被跳过而不是报错。
        let end_headers = b"X-RequestId:abc\r\nPath:audio";
        let mut end_frame = Vec::new();
        end_frame.extend_from_slice(&((end_headers.len() + 2) as u16).to_be_bytes());
        end_frame.extend_from_slice(end_headers);
        end_frame.extend_from_slice(b"\r\n");
        assert_eq!(parse_audio_frame(&end_frame).unwrap(), None);
    }

    #[test]
    fn rejects_bad_binary_frames() {
        // 头长度越界（声称 999）。
        let mut frame = 999u16.to_be_bytes().to_vec();
        frame.extend_from_slice(b"X-RequestId:abc\r\nPath:audio\r\n");
        assert!(parse_audio_frame(&frame).is_err());

        // 太短，连头长度都放不下。
        assert!(parse_audio_frame(b"\x00").is_err());

        // Path 不是 audio。
        let headers = b"X-RequestId:abc\r\nPath:turn.end";
        let mut frame = Vec::new();
        frame.extend_from_slice(&((headers.len() + 2) as u16).to_be_bytes());
        frame.extend_from_slice(headers);
        frame.extend_from_slice(b"\r\nxx");
        assert!(parse_audio_frame(&frame).is_err());

        // Content-Type 不对。
        let headers = b"Content-Type:text/html\r\nPath:audio";
        let mut frame = Vec::new();
        frame.extend_from_slice(&((headers.len() + 2) as u16).to_be_bytes());
        frame.extend_from_slice(headers);
        frame.extend_from_slice(b"\r\nxx");
        assert!(parse_audio_frame(&frame).is_err());
    }

    #[test]
    fn parses_text_frame_path() {
        assert_eq!(
            parse_text_frame_path(b"X-RequestId:abc\r\nPath:turn.end\r\n\r\n")
                .as_deref(),
            Some("turn.end")
        );
        assert_eq!(
            parse_text_frame_path(b"X-RequestId:abc\r\nPath:turn.start\r\n\r\n{}")
                .as_deref(),
            Some("turn.start")
        );
        assert_eq!(parse_text_frame_path(b"no headers"), None);
    }

    #[test]
    fn errors_are_readable_chinese_with_fallback_hint() {
        let error = channel_unavailable("WebSocket 握手被拒绝（403）");
        let message = error.to_string();
        assert!(message.contains("Edge 朗读服务暂时不可用"));
        assert!(message.contains("请改用其它朗读通道"));
        // 不能把英文的底层错误抛给用户。
        assert!(!message.contains("WebSocket error"));
        assert!(http_status_error("获取音色列表", reqwest::StatusCode::FORBIDDEN)
            .to_string()
            .contains("403"));
    }

    /// 纯标点/纯引号：服务端不会返回音频（正常行为，没有可朗读内容），
    /// 所以必须在**发请求前**用准确文案拦下；文案不能是「通道不可用」，
    /// 否则会把用户误导去换通道。
    #[tokio::test]
    async fn punctuation_only_text_reports_no_speakable_content() {
        for text in ["“”", "“", "，", "。", "……", "——", "，。"] {
            let error = synthesize(
                text,
                "zh-CN-XiaoxiaoNeural",
                "+0%",
                "+0Hz",
                "+0%",
                OUTPUT_FORMAT_48K,
                None,
            )
            .await
            .unwrap_err();
            let message = error.to_string();
            assert!(
                message.contains("该片段没有可朗读的内容"),
                "text={text:?} -> {message}"
            );
            assert!(
                !message.contains("暂时不可用") && !message.contains("请改用其它朗读通道"),
                "text={text:?} 被误报成通道故障：{message}"
            );
        }
        // 纯空白语义不同：trim 后为空，仍报「朗读文本为空」。
        let error = synthesize(
            "\u{3000}\u{3000}",
            "zh-CN-XiaoxiaoNeural",
            "+0%",
            "+0Hz",
            "+0%",
            OUTPUT_FORMAT_48K,
            None,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("朗读文本为空"), "{error}");
    }

    /// 真实网络验证（默认 ignore，手动跑）：
    /// `cargo test -p reader-rust --lib -j 2 -- --ignored edge_tts_network`
    ///
    /// 同时验证 48k / 96k 两档：96k 的字节数应约为 48k 的两倍（实测正好 2 倍）。
    #[tokio::test]
    #[ignore = "真实网络：手动验证 edge-tts 通道（需外网）"]
    async fn edge_tts_network_synthesizes_real_audio() {
        let client = reqwest::Client::builder()
            .build()
            .expect("build http client");
        let voices = list_voices(&client).await.expect("list voices");
        assert!(voices.len() > 100, "音色数量异常：{}", voices.len());
        assert!(voices.iter().any(|voice| voice["locale"] == "zh-CN"));

        let text = "他推开门，看见她站在窗边。";
        let synthesize_with = |output_format: &'static str| async move {
            synthesize(
                text,
                "zh-CN-XiaoxiaoNeural",
                "+0%",
                "+0Hz",
                "+0%",
                output_format,
                None,
            )
            .await
            .expect("synthesize")
        };

        let standard = synthesize_with(OUTPUT_FORMAT_48K).await;
        let high = synthesize_with(OUTPUT_FORMAT_96K).await;
        for (label, audio) in [("48k", &standard), ("96k", &high)] {
            assert!(audio.len() > 1000, "{label} 音频字节数异常：{}", audio.len());
            assert!(
                audio.starts_with(b"ID3")
                    || (audio.len() >= 2 && audio[0] == 0xFF && (audio[1] & 0xE0) == 0xE0),
                "{label} 不是 mp3 帧/ID3 开头：{:02X?}",
                &audio[..audio.len().min(8)]
            );
        }
        // 96k 是码率翻倍：同一句的字节数应接近两倍（留 20% 余量，避免服务端
        // 对句末静音/padding 的细微差异造成偶发失败）。
        assert!(
            high.len() > standard.len() * 3 / 2,
            "96k 应明显大于 48k：{} vs {}",
            high.len(),
            standard.len()
        );
        println!(
            "EDGE_TTS_REAL_AUDIO_48K_BYTES={} EDGE_TTS_REAL_AUDIO_96K_BYTES={}",
            standard.len(),
            high.len()
        );

        // 纯标点文本：本地就给出「没有可朗读的内容」，不再冒充通道故障。
        let punctuation = synthesize(
            "“”",
            "zh-CN-XiaoxiaoNeural",
            "+0%",
            "+0Hz",
            "+0%",
            OUTPUT_FORMAT_48K,
            None,
        )
        .await
        .unwrap_err();
        println!("EDGE_TTS_PUNCTUATION_ERROR={punctuation}");
        assert!(punctuation.to_string().contains("该片段没有可朗读的内容"));
    }
}
