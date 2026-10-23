//! 微软朗读（微软翻译 Android 客户端演示接口）通道。
//!
//! 与 `edge_tts` 的区别：这条链路走微软翻译客户端的公开演示接口，先签名换
//! `{region, token}`，再用「普通令牌」（不是 Bearer）调 Azure 语音合成端点。
//! 好处是 **支持 `mstts:express-as` 的 style / styledegree / role**，音色也更多
//! （958 个，zh-CN 84 个）。代价是 token 只有约 10 分钟寿命，且签名算法对大小写
//! 极其敏感（见 `signature_payload` 的注释）。
//!
//! 这也是微软的**非官方演示接口**，可能随时调整；对外错误一律折成可读中文并
//! 提示改用其它朗读通道，不把英文协议错误抛给用户。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

use crate::error::error::AppError;

/// 换 token 的端点（POST，空 body）。
const ENDPOINT_URL: &str = "https://dev.microsofttranslator.com/apps/endpoint?api-version=1.0";
/// 参与签名的是「去掉 scheme」的同一路径。
const ENDPOINT_PATH: &str = "dev.microsofttranslator.com/apps/endpoint?api-version=1.0";
/// 签名身份串，服务端按它校验客户端类型。
const APP_ID: &str = "MSTranslatorAndroidApp";
/// 演示客户端的硬编码签名密钥（Base64）。
const SIGN_KEY_B64: &str = "oik6PdDdMnOXemTbwvMn9de/h9lFnfBaCWbGMMZqqoSaQaqUOqjVGm5NqsmjcBI1x+sS9ugjB55HEJWRiFXYFw==";
/// 微软翻译 Android 客户端的固定头，缺一个都可能 401。
const CLIENT_VERSION: &str = "4.0.530a 5fe1dc6c";
const CLIENT_USER_ID: &str = "0f04d16a175c411e";
const HOME_GEOGRAPHIC_REGION: &str = "zh-Hans-CN";
const CLIENT_TRACE_ID: &str = "aab069b9-70a7-4844-a734-96cd78d94be9";
const USER_AGENT: &str = "okhttp/4.5.0";
/// token 名义 10 分钟有效；提前到 9 分钟主动刷新，避免卡在边界上失败。
const TOKEN_TTL: Duration = Duration::from_secs(9 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// 演示通道实测可用的三档 outputFormat（比 Edge 通道多一档 48kHz/192kbps）。
pub const OUTPUT_FORMAT_48K: &str = "audio-24khz-48kbitrate-mono-mp3";
pub const OUTPUT_FORMAT_96K: &str = "audio-24khz-96kbitrate-mono-mp3";
pub const OUTPUT_FORMAT_192K: &str = "audio-48khz-192kbitrate-mono-mp3";

// ───────────────────────────── 错误文案 ─────────────────────────────

/// 统一的「通道不可用」错误：中文说明 + 替代方案。
fn unavailable(detail: impl std::fmt::Display) -> AppError {
    AppError::BadRequest(format!(
        "微软朗读演示接口暂时不可用（{detail}）。这是微软翻译客户端的公开演示接口，可能已调整校验，请改用其它朗读通道。"
    ))
}

fn http_status_error(action: &str, status: reqwest::StatusCode) -> AppError {
    if should_refresh_token(status) {
        unavailable(format!("{action}被拒绝（{status}）"))
    } else {
        unavailable(format!("{action}返回 HTTP {status}"))
    }
}

fn http_transport_error(action: &str, error: &reqwest::Error) -> AppError {
    tracing::warn!("ms demo tts http error during {action}: {error:?}");
    let detail = if error.is_timeout() {
        format!("{action}连接超时")
    } else if error.is_connect() {
        format!("{action}无法建立连接")
    } else {
        format!("{action}网络请求失败")
    };
    unavailable(detail)
}

/// 只有 401/403 才值得清缓存重取 token；其它状态码重试也是同样结果。
pub fn should_refresh_token(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 401 | 403)
}

// ───────────────────────────── 签名 ─────────────────────────────

/// Java `URLEncoder.encode(value, "UTF-8")` 的等价实现。
///
/// 保留 `A-Za-z0-9` 与 `-_.*`，空格编码成 `+`（form 语义），其余字节 `%XX` 大写。
/// 这段 URL 里只有字母/数字/`.`/`-`/`/`/`?`/`=`，所以实际产出是
/// `dev.microsofttranslator.com%2Fapps%2Fendpoint%3Fapi-version%3D1.0`。
pub fn url_encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() * 3);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// 签名日期：US locale 的 `EEE, dd MMM yyyy HH:mm:ss` 先转小写，再接**大写** `GMT`。
///
/// 两个易错点：日期整体小写（`Thu`→`thu`、`Oct`→`oct`），但后缀 `GMT` 保持大写；
/// 且与后缀之间没有空格。服务端逐字符校验，写成 `Thu, ... GMT` 会直接 401。
pub fn signature_date(now: chrono::DateTime<chrono::Utc>) -> String {
    format!(
        "{}GMT",
        now.format("%a, %d %b %Y %H:%M:%S").to_string().to_lowercase()
    )
}

/// 待签名 payload：`MSTranslatorAndroidApp + encodeUrl + date + uuid`，**整体再转小写**。
///
/// 整体小写会把 URL-encode 出来的 `%2F/%3F/%3D` 也变成 `%2f/%3f/%3d`，这不是冗余
/// 步骤 —— 漏掉就会签名不一致，服务端只回 401 不说原因。
pub fn signature_payload(uuid: &str, date: &str) -> String {
    format!(
        "{APP_ID}{}{date}{uuid}",
        url_encode_component(ENDPOINT_PATH)
    )
    .to_lowercase()
}

/// 组装 `X-MT-Signature`：`MSTranslatorAndroidApp::<sig>::<date>::<uuid>`。
///
/// 签名 = Base64(HMAC-SHA256(payload 的 UTF-8 字节, 硬编码 key))，标准 Base64 无换行。
pub fn signature_header(uuid: &str, date: &str) -> String {
    let key = base64::engine::general_purpose::STANDARD
        .decode(SIGN_KEY_B64)
        .expect("内置签名密钥必须是合法 base64");
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).expect("HMAC 接受任意长度密钥");
    mac.update(signature_payload(uuid, date).as_bytes());
    let signature =
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    format!("{APP_ID}::{signature}::{date}::{uuid}")
}

// ───────────────────────────── token 缓存 ─────────────────────────────

/// 换到的区域 + 令牌。`region` 决定合成/音色列表的域名前缀。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub region: String,
    pub token: String,
}

#[derive(Debug, Clone)]
struct CachedEndpoint {
    region: String,
    token: String,
    fetched_at: Instant,
}

static ENDPOINT_CACHE: OnceLock<Mutex<Option<CachedEndpoint>>> = OnceLock::new();

fn endpoint_cache() -> &'static Mutex<Option<CachedEndpoint>> {
    ENDPOINT_CACHE.get_or_init(|| Mutex::new(None))
}

/// token 是否已过期。单独提出来是为了单测能注入 `fetched_at`（不用 sleep）。
pub fn is_endpoint_expired(fetched_at: Instant) -> bool {
    fetched_at.elapsed() >= TOKEN_TTL
}

fn cached_endpoint() -> Option<Endpoint> {
    let guard = endpoint_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cached = guard.as_ref()?;
    if is_endpoint_expired(cached.fetched_at) {
        return None;
    }
    Some(Endpoint {
        region: cached.region.clone(),
        token: cached.token.clone(),
    })
}

fn store_endpoint(endpoint: &Endpoint) {
    *endpoint_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(CachedEndpoint {
        region: endpoint.region.clone(),
        token: endpoint.token.clone(),
        fetched_at: Instant::now(),
    });
}

/// 清掉 token 缓存；401/403 后会调用它再重取一次。
pub fn invalidate_endpoint_cache() {
    *endpoint_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// 取可用 endpoint：缓存新鲜就用缓存，否则重新签名换 token。
pub async fn endpoint_for_request(client: &reqwest::Client) -> Result<Endpoint, AppError> {
    if let Some(endpoint) = cached_endpoint() {
        return Ok(endpoint);
    }
    let endpoint = fetch_endpoint(client).await?;
    store_endpoint(&endpoint);
    Ok(endpoint)
}

async fn fetch_endpoint(client: &reqwest::Client) -> Result<Endpoint, AppError> {
    let uuid = uuid::Uuid::new_v4().simple().to_string();
    let date = signature_date(chrono::Utc::now());
    let signature = signature_header(&uuid, &date);

    let response = client
        .post(ENDPOINT_URL)
        .timeout(REQUEST_TIMEOUT)
        .header("Accept-Language", "zh-Hans")
        .header("X-ClientVersion", CLIENT_VERSION)
        .header("X-UserId", CLIENT_USER_ID)
        .header("X-HomeGeographicRegion", HOME_GEOGRAPHIC_REGION)
        .header("X-ClientTraceId", CLIENT_TRACE_ID)
        .header("X-MT-Signature", signature)
        .header("User-Agent", USER_AGENT)
        .header("Content-Type", "application/json; charset=utf-8")
        // 空 body；reqwest 会自动带 Content-Length: 0。
        .body(Vec::new())
        .send()
        .await
        .map_err(|error| http_transport_error("获取朗读令牌", &error))?;

    let status = response.status();
    if !status.is_success() {
        return Err(http_status_error("获取朗读令牌", status));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|_| unavailable("令牌响应不是合法 JSON"))?;
    let region = value
        .get("r")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let token = value
        .get("t")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if region.is_empty() || token.is_empty() {
        return Err(unavailable("令牌响应缺少区域或 token"));
    }
    Ok(Endpoint { region, token })
}

// ───────────────────────────── 音质 / SSML ─────────────────────────────

/// 把调用方给的音质参数归一化成白名单里的 outputFormat。
///
/// 实测只支持三档（48k/96k/192k），其它值静默回退 48k —— 音质是体验偏好，
/// 不能因为历史脏值让整句朗读失败。
pub fn normalize_output_format(requested: Option<&str>) -> &'static str {
    let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
        return OUTPUT_FORMAT_48K;
    };
    let lowered = requested.to_ascii_lowercase();
    for (format, alias) in [
        (OUTPUT_FORMAT_48K, "48k"),
        (OUTPUT_FORMAT_96K, "96k"),
        (OUTPUT_FORMAT_192K, "192k"),
    ] {
        if lowered == format || lowered == alias {
            return format;
        }
    }
    let digits = lowered
        .strip_suffix("kbps")
        .or_else(|| lowered.strip_suffix('k'))
        .unwrap_or(lowered.as_str())
        .trim();
    match digits.parse::<u32>() {
        Ok(192) => OUTPUT_FORMAT_192K,
        Ok(96) => OUTPUT_FORMAT_96K,
        _ => OUTPUT_FORMAT_48K,
    }
}

/// 风格强度只能取 0.01~2（Azure 的 styledegree 范围）；非有限值回默认 1.0。
pub fn normalize_style_degree(value: f64) -> f64 {
    if !value.is_finite() {
        return 1.0;
    }
    value.clamp(0.01, 2.0)
}

/// styledegree 去掉无意义的小数零：1.0 → `1`，1.5 → `1.5`。
fn format_style_degree(value: f64) -> String {
    let degree = normalize_style_degree(value);
    if degree.fract() == 0.0 {
        format!("{}", degree as i64)
    } else {
        format!("{degree}")
    }
}

/// `general` / 空串都等价于「无风格」，不能输出 `mstts:express-as` 节点
/// （服务端会按未知风格处理，结果要么报错要么听感完全不对）。
pub fn has_style(style: &str) -> bool {
    let style = style.trim();
    !style.is_empty() && !style.eq_ignore_ascii_case("general")
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
    value.replace('&', "&amp;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// 正文清洗：只做 XML 转义，**不改动正文内容**。
///
/// 旧实现会把 `/` 与 `\` 直接删掉（照抄插件原实现：这两个字符偶尔让演示接口返回
/// 空音频）。代价是「1/2」「A\B」、URL、代码片段在朗读里被读错且不可恢复 —— 为了
/// 一个偶发问题牺牲所有正文不值得。
///
/// 折中办法：正常情况按原文朗读；**只有真的收到空音频**时才换成
/// [`neutralize_slash_like`] 的变体重试一次（见 [`synthesize`]）。
pub fn sanitize_tts_text(text: &str) -> String {
    escape_ssml(text)
}

/// 空音频兜底：把 `/`、`\` 换成外形相近的全角字符再重试。
///
/// 用替换而不是删除：`1/2` 至少还是「1／2」，读出来仍能听懂；直接删会变成 `12`，
/// 语义完全变了。
fn neutralize_slash_like(text: &str) -> String {
    text.replace('/', "／").replace('\\', "＼")
}

/// 语音的 locale：`zh-CN-YunjianNeural` → `zh-CN`。
pub fn voice_locale(voice: &str) -> String {
    let mut parts = voice.trim().split('-');
    match (parts.next(), parts.next()) {
        (Some(language), Some(region)) if !language.is_empty() && !region.is_empty() => {
            format!("{language}-{region}")
        }
        _ => "zh-CN".to_string(),
    }
}

/// 生成 SSML。无风格时输出简化形态（不带 mstts 命名空间的节点也没问题，
/// 但命名空间保留可让服务端少一次分支解析）。
#[allow(clippy::too_many_arguments)]
pub fn build_ssml(
    voice: &str,
    style: &str,
    style_degree: f64,
    role: &str,
    rate: &str,
    pitch: &str,
    volume: &str,
    text: &str,
) -> String {
    let escaped_text = sanitize_tts_text(text);
    let prosody = format!(
        "<prosody rate=\"{}\" pitch=\"{}\" volume=\"{}\">{escaped_text}</prosody>",
        escape_ssml_attribute(rate),
        escape_ssml_attribute(pitch),
        escape_ssml_attribute(volume),
    );
    let body = if has_style(style) {
        format!(
            "<mstts:express-as style=\"{}\" styledegree=\"{}\" role=\"{}\">{prosody}</mstts:express-as>",
            escape_ssml_attribute(style.trim()),
            format_style_degree(style_degree),
            escape_ssml_attribute(role),
        )
    } else {
        prosody
    };
    format!(
        "<speak xmlns=\"http://www.w3.org/2001/10/synthesis\" xmlns:mstts=\"http://www.w3.org/2001/mstts\" version=\"1.0\" xml:lang=\"{}\"><voice name=\"{}\">{body}</voice></speak>",
        escape_ssml_attribute(&voice_locale(voice)),
        escape_ssml_attribute(voice.trim()),
    )
}

// ───────────────────────────── 音色列表 ─────────────────────────────

/// 解析 `/cognitiveservices/voices/list` 的响应。
///
/// 原始字段是 PascalCase：`ShortName / Locale / Gender / LocalName / DisplayName /
/// StyleList`。`friendlyName` 取 `LocalName`（中文音色就是「云健」这类本地化名），
/// 缺失时依次退 `DisplayName`、shortName。多数音色没有 `StyleList`，按空数组处理。
pub fn parse_voice_list(payload: &[u8]) -> Result<Vec<Value>, AppError> {
    let data: Value = serde_json::from_slice(payload)
        .map_err(|error| unavailable(format!("音色列表解析失败（{error}）")))?;
    let Some(items) = data.as_array() else {
        return Err(unavailable("音色列表格式不正确"));
    };
    let mut voices: Vec<Value> = items
        .iter()
        .filter_map(|item| {
            let short_name = item.get("ShortName").and_then(Value::as_str)?.trim();
            if short_name.is_empty() {
                return None;
            }
            let local_name = item
                .get("LocalName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let display_name = item
                .get("DisplayName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let friendly = if !local_name.is_empty() {
                local_name
            } else if !display_name.is_empty() {
                display_name
            } else {
                short_name
            };
            let styles: Vec<Value> = item
                .get("StyleList")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .map(|style| Value::String(style.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            Some(json!({
                "shortName": short_name,
                "locale": item.get("Locale").and_then(Value::as_str).unwrap_or_default(),
                "gender": item.get("Gender").and_then(Value::as_str).unwrap_or_default(),
                "friendlyName": friendly,
                "styles": styles,
            }))
        })
        .collect();
    if voices.is_empty() {
        return Err(unavailable("音色列表为空"));
    }
    voices.sort_by(|left, right| {
        let left_locale = left["locale"].as_str().unwrap_or_default();
        let right_locale = right["locale"].as_str().unwrap_or_default();
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

static VOICE_CACHE: crate::service::voice_cache::VoiceCache =
    crate::service::voice_cache::VoiceCache::new();

/// 音色列表（进程内缓存一天，失败不缓存；刷新失败时退回旧列表）。
pub async fn list_voices(client: &reqwest::Client) -> Result<Vec<Value>, AppError> {
    if let Some(cached) = VOICE_CACHE.fresh() {
        return Ok(cached);
    }

    let mut endpoint = endpoint_for_request(client).await?;
    let mut response = get_voice_list(client, &endpoint).await?;
    if should_refresh_token(response.status()) {
        invalidate_endpoint_cache();
        endpoint = endpoint_for_request(client).await?;
        response = get_voice_list(client, &endpoint).await?;
        if should_refresh_token(response.status()) {
            if let Some(stale) = VOICE_CACHE.any() {
                tracing::warn!("刷新微软朗读音色列表的 token 失败，继续使用缓存");
                return Ok(stale);
            }
            return Err(http_status_error("获取音色列表", response.status()));
        }
    }
    let status = response.status();
    if !status.is_success() {
        // 刷新失败：退回旧列表（可能只是临时断网/被限流），旧音色照样能用。
        if let Some(stale) = VOICE_CACHE.any() {
            tracing::warn!("刷新微软朗读音色列表失败（{status}），继续使用缓存");
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

async fn get_voice_list(
    client: &reqwest::Client,
    endpoint: &Endpoint,
) -> Result<reqwest::Response, AppError> {
    client
        .get(format!(
            "https://{}.tts.speech.microsoft.com/cognitiveservices/voices/list",
            endpoint.region
        ))
        .timeout(REQUEST_TIMEOUT)
        .header("Authorization", &endpoint.token)
        .header("Content-Type", "application/json")
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|error| http_transport_error("获取音色列表", &error))
}

// ───────────────────────────── 合成 ─────────────────────────────

/// 一次合成的全部参数。`output_format` 应已经过 [`normalize_output_format`]。
pub struct SynthesisParams<'a> {
    pub text: &'a str,
    pub voice: &'a str,
    pub style: &'a str,
    pub style_degree: f64,
    pub role: &'a str,
    pub rate: &'a str,
    pub pitch: &'a str,
    pub volume: &'a str,
    pub output_format: &'a str,
}

/// 合成一段文本，返回 mp3 字节。
///
/// 401/403 时清 token 缓存重取一次再重试（只重试一次）；仍失败就报中文错误。
pub async fn synthesize(
    client: &reqwest::Client,
    params: &SynthesisParams<'_>,
) -> Result<Vec<u8>, AppError> {
    let text = params.text.trim();
    if text.is_empty() {
        return Err(AppError::BadRequest("朗读文本为空".to_string()));
    }
    // 纯标点/纯引号在服务端是「无内容可读」而不是故障；发请求前就拦下，
    // 避免误报成「演示接口暂时不可用，请改用其它朗读通道」。
    if !crate::util::speakable_text::has_speakable_content(text) {
        return Err(AppError::BadRequest(
            "该片段没有可朗读的内容".to_string(),
        ));
    }
    if params.voice.trim().is_empty() {
        return Err(AppError::BadRequest("请选择音色".to_string()));
    }
    let output_format = normalize_output_format(Some(params.output_format));
    let ssml = build_ssml(
        params.voice,
        params.style,
        params.style_degree,
        params.role,
        params.rate,
        params.pitch,
        params.volume,
        text,
    );

    let mut endpoint = endpoint_for_request(client).await?;
    let mut response = post_synthesis(client, &endpoint, output_format, &ssml).await?;
    if should_refresh_token(response.status()) {
        invalidate_endpoint_cache();
        endpoint = endpoint_for_request(client).await?;
        response = post_synthesis(client, &endpoint, output_format, &ssml).await?;
        if should_refresh_token(response.status()) {
            return Err(http_status_error("微软朗读合成", response.status()));
        }
    }
    let status = response.status();
    if !status.is_success() {
        return Err(http_status_error("微软朗读合成", status));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| http_transport_error("读取合成音频", &error))?;

    // 空音频兜底：正文里带 `/` / `\` 时演示接口偶发返回空响应（旧实现直接删字符，
    // 代价是「1/2」被读成「12」）。这里只在真的拿到空音频时才换成全角字符重试一次，
    // 正常文本完全不受影响。
    if bytes.is_empty() && text.contains(['/', '\\']) {
        let retry_ssml = build_ssml(
            params.voice,
            params.style,
            params.style_degree,
            params.role,
            params.rate,
            params.pitch,
            params.volume,
            &neutralize_slash_like(text),
        );
        let retry = post_synthesis(client, &endpoint, output_format, &retry_ssml).await?;
        if retry.status().is_success() {
            let retry_bytes = retry
                .bytes()
                .await
                .map_err(|error| http_transport_error("读取合成音频", &error))?;
            if !retry_bytes.is_empty() && looks_like_audio(&retry_bytes) {
                return Ok(retry_bytes.to_vec());
            }
        }
        return Err(unavailable("没有返回音频数据"));
    }

    if bytes.is_empty() {
        return Err(unavailable("没有返回音频数据"));
    }
    // 出错时服务端有时仍回 200（错误页/JSON），只信状态码会把 HTML 当 mp3 播。
    if !looks_like_audio(&bytes) {
        return Err(unavailable("返回的不是音频（可能是错误页）"));
    }
    Ok(bytes.to_vec())
}

async fn post_synthesis(
    client: &reqwest::Client,
    endpoint: &Endpoint,
    output_format: &str,
    ssml: &str,
) -> Result<reqwest::Response, AppError> {
    client
        .post(format!(
            "https://{}.tts.speech.microsoft.com/cognitiveservices/v1",
            endpoint.region
        ))
        .timeout(REQUEST_TIMEOUT)
        // 注意：这个令牌直接放 Authorization，**不能**加 "Bearer " 前缀。
        .header("Authorization", &endpoint.token)
        .header("Content-Type", "application/ssml+xml")
        .header("X-Microsoft-OutputFormat", output_format)
        .header("User-Agent", USER_AGENT)
        .body(ssml.to_string())
        .send()
        .await
        .map_err(|error| http_transport_error("微软朗读合成", &error))
}

/// mp3 魔数识别：ID3 标签或 MPEG 帧同步（0xFF 后高 3 位全 1）。
fn looks_like_audio(bytes: &[u8]) -> bool {
    bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// 由一份独立的 Python（hmac/hashlib/base64）实现算出的期望值：
    /// 固定 uuid + 固定 date，逐字符固定整个 X-MT-Signature 头。
    #[test]
    fn signature_matches_independent_fixture() {
        let uuid = "0123456789abcdef0123456789abcdef";
        let date = "thu, 20 oct 2026 12:34:56GMT";
        assert_eq!(
            signature_header(uuid, date),
            "MSTranslatorAndroidApp::yDLpONujVtDMKqvCnFjM3YNkYrjLx7xo61qcLzXcST0=::thu, 20 oct 2026 12:34:56GMT::0123456789abcdef0123456789abcdef"
        );
    }

    /// payload 必须整体小写：URL-encode 的 `%2F/%3F/%3D` 也要变成小写 hex，
    /// 这是最容易漏的一步（漏了就是 401）。
    #[test]
    fn signature_payload_is_fully_lowercased() {
        let payload = signature_payload(
            "0123456789abcdef0123456789abcdef",
            "thu, 20 oct 2026 12:34:56GMT",
        );
        assert_eq!(
            payload,
            "mstranslatorandroidappdev.microsofttranslator.com%2fapps%2fendpoint%3fapi-version%3d1.0thu, 20 oct 2026 12:34:56gmt0123456789abcdef0123456789abcdef"
        );
        assert!(!payload.chars().any(|ch| ch.is_ascii_uppercase()));
    }

    #[test]
    fn signature_date_is_lowercase_with_uppercase_gmt_suffix() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 10, 20, 12, 34, 56).unwrap();
        assert_eq!(signature_date(now), "tue, 20 oct 2026 12:34:56GMT");
        // 日期部分全小写、GMT 大写、且两者之间没有空格。
        assert!(signature_date(now).ends_with("56GMT"));
    }

    #[test]
    fn url_encode_matches_java_url_encoder() {
        assert_eq!(
            url_encode_component(ENDPOINT_PATH),
            "dev.microsofttranslator.com%2Fapps%2Fendpoint%3Fapi-version%3D1.0"
        );
        // Java form 语义：空格是 '+'，保留 `-_.*`（`*` 也不编码）。
        assert_eq!(
            url_encode_component("a b-c_d.e*f/g?h=i"),
            "a+b-c_d.e*f%2Fg%3Fh%3Di"
        );
    }

    #[test]
    fn builds_ssml_without_style() {
        let ssml = build_ssml(
            "zh-CN-YunjianNeural",
            "",
            1.0,
            "default",
            "+0%",
            "+0%",
            "+0%",
            "你好",
        );
        assert!(ssml.starts_with("<speak xmlns=\"http://www.w3.org/2001/10/synthesis\""));
        assert!(ssml.contains("xml:lang=\"zh-CN\""));
        assert!(ssml.contains("<voice name=\"zh-CN-YunjianNeural\">"));
        assert!(ssml.contains("<prosody rate=\"+0%\" pitch=\"+0%\" volume=\"+0%\">你好</prosody>"));
        assert!(!ssml.contains("mstts:express-as"));
        assert!(ssml.ends_with("</voice></speak>"));

        // "general" 等同于无风格。
        let general = build_ssml(
            "zh-CN-YunjianNeural",
            "general",
            1.0,
            "default",
            "+0%",
            "+0%",
            "+0%",
            "x",
        );
        assert!(!general.contains("mstts:express-as"));
    }

    #[test]
    fn builds_ssml_with_style_degree_and_role() {
        let ssml = build_ssml(
            "zh-CN-YunjianNeural",
            "narration-relaxed",
            1.5,
            "Girl",
            "+40%",
            "-10%",
            "+20%",
            "正文",
        );
        assert!(ssml.contains("xmlns:mstts=\"http://www.w3.org/2001/mstts\""));
        assert!(ssml.contains(
            "<mstts:express-as style=\"narration-relaxed\" styledegree=\"1.5\" role=\"Girl\">"
        ));
        assert!(ssml.contains("<prosody rate=\"+40%\" pitch=\"-10%\" volume=\"+20%\">正文</prosody>"));
        // 默认强度 1.0 输出成 "1" 而不是 "1.0"。
        let integer_degree = build_ssml(
            "zh-CN-YunjianNeural",
            "cheerful",
            1.0,
            "default",
            "+0%",
            "+0%",
            "+0%",
            "x",
        );
        assert!(integer_degree.contains("styledegree=\"1\" role=\"default\""));
        // 越界强度夹到 0.01~2。
        let clamped = build_ssml(
            "zh-CN-YunjianNeural",
            "cheerful",
            9.0,
            "default",
            "+0%",
            "+0%",
            "+0%",
            "x",
        );
        assert!(clamped.contains("styledegree=\"2\""));
    }

    #[test]
    fn ssml_escapes_text_without_dropping_characters() {
        let ssml = build_ssml(
            "zh-CN-YunjianNeural",
            "",
            1.0,
            "default",
            "+0%",
            "+0%",
            "+0%",
            r#"甲<&乙 3/4 C:\tmp "q" 's'"#,
        );
        // 只转义，不删字符：`/`、`\` 必须原样留在正文里（旧实现会把「3/4」读成「34」）。
        assert!(ssml.contains("甲&lt;&amp;乙 3/4 C:\\tmp &quot;q&quot; &apos;s&apos;"));
        assert!(ssml.contains("3/4"), "斜杠不能被删：{ssml}");
        assert!(ssml.contains("C:\\tmp"), "反斜杠不能被删：{ssml}");
    }

    /// 空音频兜底把斜杠换成全角，而不是删掉 —— 「1/2」读成「1／2」还能听懂。
    #[test]
    fn slash_fallback_replaces_instead_of_deleting() {
        assert_eq!(neutralize_slash_like("3/4"), "3／4");
        assert_eq!(neutralize_slash_like(r"C:\tmp"), "C:＼tmp");
        // 不含斜杠的文本不受影响
        assert_eq!(neutralize_slash_like("普通正文"), "普通正文");
    }

    #[test]
    fn normalizes_output_format_to_whitelist() {
        for requested in [
            "96k",
            "96K",
            "96Kbps",
            "96",
            "audio-24khz-96kbitrate-mono-mp3",
            "AUDIO-24KHZ-96KBITRATE-MONO-MP3",
        ] {
            assert_eq!(
                normalize_output_format(Some(requested)),
                OUTPUT_FORMAT_96K,
                "requested = {requested}"
            );
        }
        for requested in [
            "192k",
            "192Kbps",
            "192",
            "audio-48khz-192kbitrate-mono-mp3",
        ] {
            assert_eq!(
                normalize_output_format(Some(requested)),
                OUTPUT_FORMAT_192K,
                "requested = {requested}"
            );
        }
        for requested in ["48k", "48Kbps", "48", OUTPUT_FORMAT_48K] {
            assert_eq!(
                normalize_output_format(Some(requested)),
                OUTPUT_FORMAT_48K,
                "requested = {requested}"
            );
        }
        // 缺失 / 空白 / 白名单之外的值一律回退 48k。
        for requested in [
            None,
            Some(""),
            Some("   "),
            Some("64k"),
            Some("128k"),
            Some("audio-48khz-192kbitrate-mono-pcm"),
            Some("garbage"),
        ] {
            assert_eq!(
                normalize_output_format(requested),
                OUTPUT_FORMAT_48K,
                "requested = {requested:?}"
            );
        }
    }

    const VOICES_FIXTURE: &str = r#"[
      {
        "ShortName": "zh-CN-YunjianNeural",
        "Locale": "zh-CN",
        "Gender": "Male",
        "LocalName": "云健",
        "DisplayName": "Yunjian",
        "StyleList": ["narration-relaxed", "cheerful", "sad"]
      },
      {
        "ShortName": "zh-CN-XiaoxiaoNeural",
        "Locale": "zh-CN",
        "Gender": "Female",
        "LocalName": "晓晓"
      },
      {
        "ShortName": "en-US-AriaNeural",
        "Locale": "en-US",
        "Gender": "Female",
        "LocalName": "",
        "DisplayName": "Aria",
        "StyleList": ["cheerful"]
      },
      {
        "ShortName": "ja-JP-NanamiNeural",
        "Locale": "ja-JP",
        "Gender": "Female",
        "LocalName": "七海"
      }
    ]"#;

    #[test]
    fn parses_voice_list_with_styles_and_sorts_zh_cn_first() {
        let voices = parse_voice_list(VOICES_FIXTURE.as_bytes()).unwrap();
        assert_eq!(voices.len(), 4);
        let short_names: Vec<&str> = voices
            .iter()
            .map(|voice| voice["shortName"].as_str().unwrap())
            .collect();
        assert_eq!(
            short_names,
            vec![
                "zh-CN-XiaoxiaoNeural",
                "zh-CN-YunjianNeural",
                "en-US-AriaNeural",
                "ja-JP-NanamiNeural",
            ]
        );
        let yunjian = &voices[1];
        assert_eq!(yunjian["friendlyName"], "云健");
        assert_eq!(yunjian["gender"], "Male");
        assert_eq!(
            yunjian["styles"],
            json!(["narration-relaxed", "cheerful", "sad"])
        );
        // 没有 StyleList 的音色是空数组；LocalName 为空时退 DisplayName。
        assert_eq!(voices[0]["styles"], json!([]));
        assert_eq!(voices[2]["friendlyName"], "Aria");
    }

    #[test]
    fn rejects_invalid_voice_payloads() {
        assert!(parse_voice_list(b"not json").is_err());
        assert!(parse_voice_list(b"{}").is_err());
        assert!(parse_voice_list(b"[]").is_err());
        assert!(parse_voice_list(br#"[{"Locale":"zh-CN"}]"#).is_err());
    }

    #[test]
    fn endpoint_token_expires_after_nine_minutes() {
        // 8 分钟：仍新鲜；10 分钟：已过期。用 Instant 减法注入时间，不 sleep。
        assert!(!is_endpoint_expired(
            Instant::now() - Duration::from_secs(8 * 60)
        ));
        assert!(is_endpoint_expired(
            Instant::now() - Duration::from_secs(10 * 60)
        ));
        assert!(!is_endpoint_expired(Instant::now()));
    }

    /// 缓存 seed → 命中 → 失效 → 落空。放在同一个测试里，避免并行测试互相清缓存。
    #[test]
    fn endpoint_cache_seed_and_invalidate() {
        let endpoint = Endpoint {
            region: "southeastasia".to_string(),
            token: "token".to_string(),
        };
        store_endpoint(&endpoint);
        assert_eq!(cached_endpoint(), Some(endpoint));
        invalidate_endpoint_cache();
        assert_eq!(cached_endpoint(), None);
    }

    #[test]
    fn only_401_403_refresh_token() {
        for status in [401u16, 403] {
            assert!(should_refresh_token(
                reqwest::StatusCode::from_u16(status).unwrap()
            ));
        }
        for status in [200u16, 400, 404, 429, 500] {
            assert!(!should_refresh_token(
                reqwest::StatusCode::from_u16(status).unwrap()
            ));
        }
    }

    #[test]
    fn errors_are_readable_chinese() {
        let message = unavailable("获取朗读令牌被拒绝（403）").to_string();
        assert!(message.contains("微软朗读演示接口暂时不可用"));
        assert!(message.contains("请改用其它朗读通道"));
    }

    /// 纯标点/纯引号：服务端不返回音频是正常行为，必须在**发请求前**用准确文案
    /// 拦下；文案不能是「接口不可用」，否则会把用户误导去换通道。
    #[tokio::test]
    async fn punctuation_only_text_reports_no_speakable_content() {
        let client = reqwest::Client::new();
        let params_for = |text: &'static str| SynthesisParams {
            text,
            voice: "zh-CN-YunjianNeural",
            style: "",
            style_degree: 1.0,
            role: "default",
            rate: "+0%",
            pitch: "+0%",
            volume: "+0%",
            output_format: OUTPUT_FORMAT_48K,
        };
        for text in ["“”", "“", "，", "。", "……", "——", "，。"] {
            let error = synthesize(&client, &params_for(text)).await.unwrap_err();
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
        let error = synthesize(&client, &params_for("\u{3000}\u{3000}"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("朗读文本为空"), "{error}");
    }

    /// 真实网络验证（默认 ignore，手动跑）：
    /// `cargo test -p reader-rust --lib -j 2 -- --ignored ms_demo_tts_network`
    ///
    /// 同时验证 token、音色数量、有/无风格、风格强度、三档音质。
    #[tokio::test]
    #[ignore = "真实网络：手动验证微软朗读演示接口（需外网）"]
    async fn ms_demo_tts_network_synthesizes_real_audio() {
        let client = reqwest::Client::builder()
            .build()
            .expect("build http client");

        let endpoint = endpoint_for_request(&client).await.expect("endpoint");
        println!(
            "MS_TTS_REGION={} MS_TTS_TOKEN_LEN={}",
            endpoint.region,
            endpoint.token.len()
        );
        assert!(!endpoint.region.is_empty());
        assert!(endpoint.token.len() > 500, "token 长度异常");

        let voices = list_voices(&client).await.expect("voices");
        let zh_count = voices
            .iter()
            .filter(|voice| voice["locale"].as_str().unwrap_or_default().starts_with("zh-CN"))
            .count();
        println!(
            "MS_TTS_VOICES={} MS_TTS_ZH_CN={}",
            voices.len(),
            zh_count
        );
        assert!(voices.len() > 900, "音色数量异常：{}", voices.len());
        assert!(zh_count >= 80, "中文音色数量异常：{zh_count}");
        assert!(voices.iter().any(|voice| {
            voice["shortName"] == "zh-CN-YunjianNeural"
                && voice["styles"]
                    .as_array()
                    .is_some_and(|styles| styles.iter().any(|style| style == "narration-relaxed"))
        }));

        let text = "他推开门，看见她站在窗边。";
        async fn synth(
            client: &reqwest::Client,
            text: &str,
            style: &str,
            degree: f64,
            format: &str,
        ) -> Vec<u8> {
            let params = SynthesisParams {
                text,
                voice: "zh-CN-YunjianNeural",
                style,
                style_degree: degree,
                role: "default",
                rate: "+0%",
                pitch: "+0%",
                volume: "+0%",
                output_format: format,
            };
            synthesize(client, &params).await.expect("synthesize")
        }

        let plain = synth(&client, text, "", 1.0, OUTPUT_FORMAT_48K).await;
        let narration = synth(&client, text, "narration-relaxed", 1.0, OUTPUT_FORMAT_48K).await;
        let cheerful = synth(&client, text, "cheerful", 1.0, OUTPUT_FORMAT_48K).await;
        let sad = synth(&client, text, "sad", 1.5, OUTPUT_FORMAT_48K).await;
        let high = synth(&client, text, "narration-relaxed", 1.0, OUTPUT_FORMAT_96K).await;
        let studio = synth(&client, text, "narration-relaxed", 1.0, OUTPUT_FORMAT_192K).await;
        println!(
            "MS_TTS_BYTES plain={} narration={} cheerful={} sad15={} 48k={} 96k={} 192k={}",
            plain.len(),
            narration.len(),
            cheerful.len(),
            sad.len(),
            narration.len(),
            high.len(),
            studio.len()
        );

        for (label, audio) in [
            ("plain", &plain),
            ("narration", &narration),
            ("cheerful", &cheerful),
            ("sad", &sad),
            ("96k", &high),
            ("192k", &studio),
        ] {
            assert!(audio.len() > 1000, "{label} 音频字节数异常：{}", audio.len());
            assert!(
                audio.starts_with(b"ID3")
                    || (audio.len() >= 2 && audio[0] == 0xFF && (audio[1] & 0xE0) == 0xE0),
                "{label} 不是 mp3：{:02X?}",
                &audio[..audio.len().min(8)]
            );
        }
        assert!(high.len() > narration.len(), "96k 应大于 48k");
        assert!(studio.len() > high.len(), "192k 应大于 96k");

        // 纯标点文本：本地就给出「没有可朗读的内容」，不冒充程序接口故障。
        let punctuation = synthesize(
            &client,
            &SynthesisParams {
                text: "“”",
                voice: "zh-CN-YunjianNeural",
                style: "",
                style_degree: 1.0,
                role: "default",
                rate: "+0%",
                pitch: "+0%",
                volume: "+0%",
                output_format: OUTPUT_FORMAT_48K,
            },
        )
        .await
        .unwrap_err();
        println!("MS_TTS_PUNCTUATION_ERROR={punctuation}");
        assert!(punctuation.to_string().contains("该片段没有可朗读的内容"));
    }
}
