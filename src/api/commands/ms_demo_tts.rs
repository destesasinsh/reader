use serde::Deserialize;
use serde_json::Value;

use crate::api::AppState;
use crate::error::error::{ApiResponse, AppError};
use crate::service::ms_demo_tts;

/// 微软朗读合成参数。
///
/// 可选字段的默认值与前端约定一致：`styleDegree=1.0`、`role=default`，
/// `rate`/`pitch`/`volume` 都是带符号百分比串（这个通道的 pitch 也是百分比，
/// 不是 Edge 通道的 Hz），缺省 `+0%`；`format` 归一化到 48k/96k/192k。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MsDemoTtsRequest {
    pub text: String,
    pub voice: String,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub style_degree: Option<f64>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub rate: Option<String>,
    #[serde(default)]
    pub pitch: Option<String>,
    #[serde(default)]
    pub volume: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug)]
struct NormalizedMsDemoTtsRequest {
    text: String,
    voice: String,
    style: String,
    style_degree: f64,
    role: String,
    rate: String,
    pitch: String,
    volume: String,
    /// 已归一化成白名单里的 outputFormat（&'static str，无需再校验）。
    output_format: &'static str,
}

impl MsDemoTtsRequest {
    fn normalize(self) -> Result<NormalizedMsDemoTtsRequest, AppError> {
        let text = self.text.trim().to_string();
        if text.is_empty() {
            return Err(AppError::BadRequest("朗读文本为空".to_string()));
        }
        let voice = self.voice.trim().to_string();
        if voice.is_empty() {
            return Err(AppError::BadRequest("请选择音色".to_string()));
        }
        Ok(NormalizedMsDemoTtsRequest {
            text,
            voice,
            // 空 style 表示无风格（不输出 mstts:express-as 节点）。
            style: self.style.unwrap_or_default().trim().to_string(),
            style_degree: ms_demo_tts::normalize_style_degree(
                self.style_degree.unwrap_or(1.0),
            ),
            // role 是枚举名（`default` / `YoungAdultFemale` …），不是偏移量，
            // 只做 trim 与空值兜底。
            role: self
                .role
                .map(|role| role.trim().to_string())
                .filter(|role| !role.is_empty())
                .unwrap_or_else(|| "default".to_string()),
            // 这个通道的三个参数都是百分比（pitch 也是，与 Edge 的 Hz 不同）。
            rate: crate::service::tts_param::normalize_offset(self.rate.as_deref(), "+0%", &["%"]),
            pitch: crate::service::tts_param::normalize_offset(
                self.pitch.as_deref(),
                "+0%",
                &["%"],
            ),
            volume: crate::service::tts_param::normalize_offset(
                self.volume.as_deref(),
                "+0%",
                &["%"],
            ),
            output_format: ms_demo_tts::normalize_output_format(self.format.as_deref()),
        })
    }
}

/// 列出微软朗读音色的共享实现（进程内缓存，第一次成功拉取后复用）。
pub async fn ms_tts_list_voices_core(
    state: &AppState,
) -> Result<ApiResponse<Vec<Value>>, AppError> {
    let client = state.book_service.http_client();
    Ok(ApiResponse::ok(ms_demo_tts::list_voices(&client).await?))
}

/// `ms_tts_list_voices` 的 Tauri 入口: 只是 core 的薄壳。
#[tauri::command]
pub async fn ms_tts_list_voices(
    state: tauri::State<'_, AppState>,
) -> Result<ApiResponse<Vec<Value>>, AppError> {
    ms_tts_list_voices_core(&state).await
}

/// 合成一段微软朗读音频的共享实现（mp3 字节，前端 `invokeRaw` 取 ArrayBuffer）。
///
/// 支持风格：`style` + `styleDegree` + `role`，这是本通道相对 Edge 的主要优势。
/// 合成本身只用请求参数，但 HTTP client 取自 `AppState`（复用应用的代理配置）。
pub async fn ms_tts_synthesize_core(
    state: &AppState,
    req: MsDemoTtsRequest,
) -> Result<tauri::ipc::Response, AppError> {
    let request = req.normalize()?;
    let client = state.book_service.http_client();
    let params = ms_demo_tts::SynthesisParams {
        text: &request.text,
        voice: &request.voice,
        style: &request.style,
        style_degree: request.style_degree,
        role: &request.role,
        rate: &request.rate,
        pitch: &request.pitch,
        volume: &request.volume,
        output_format: request.output_format,
    };
    let audio = ms_demo_tts::synthesize(&client, &params).await?;
    Ok(tauri::ipc::Response::new(audio))
}

/// `ms_tts_synthesize` 的 Tauri 入口: 只是 core 的薄壳。
#[tauri::command]
pub async fn ms_tts_synthesize(
    state: tauri::State<'_, AppState>,
    req: MsDemoTtsRequest,
) -> Result<tauri::ipc::Response, AppError> {
    ms_tts_synthesize_core(&state, req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(text: &str, voice: &str) -> MsDemoTtsRequest {
        MsDemoTtsRequest {
            text: text.to_string(),
            voice: voice.to_string(),
            style: None,
            style_degree: None,
            role: None,
            rate: None,
            pitch: None,
            volume: None,
            format: None,
        }
    }

    #[test]
    fn applies_defaults_for_missing_optional_params() {
        let normalized = request("  你好世界  ", " zh-CN-YunjianNeural ")
            .normalize()
            .unwrap();
        assert_eq!(normalized.text, "你好世界");
        assert_eq!(normalized.voice, "zh-CN-YunjianNeural");
        assert_eq!(normalized.style, "");
        assert_eq!(normalized.style_degree, 1.0);
        assert_eq!(normalized.role, "default");
        assert_eq!(normalized.rate, "+0%");
        assert_eq!(normalized.pitch, "+0%");
        assert_eq!(normalized.volume, "+0%");
        assert_eq!(normalized.output_format, ms_demo_tts::OUTPUT_FORMAT_48K);
    }

    #[test]
    fn keeps_explicit_style_and_ssml_params() {
        let normalized = MsDemoTtsRequest {
            text: "正文".to_string(),
            voice: "zh-CN-YunjianNeural".to_string(),
            style: Some(" narration-relaxed ".to_string()),
            style_degree: Some(1.5),
            role: Some("Girl".to_string()),
            rate: Some("+40%".to_string()),
            pitch: Some("-10%".to_string()),
            volume: Some("+20%".to_string()),
            format: Some("192k".to_string()),
        }
        .normalize()
        .unwrap();
        assert_eq!(normalized.style, "narration-relaxed");
        assert_eq!(normalized.style_degree, 1.5);
        assert_eq!(normalized.role, "Girl");
        assert_eq!(normalized.rate, "+40%");
        assert_eq!(normalized.pitch, "-10%");
        assert_eq!(normalized.volume, "+20%");
        assert_eq!(
            normalized.output_format,
            ms_demo_tts::OUTPUT_FORMAT_192K
        );
    }

    #[test]
    fn clamps_style_degree_and_defaults_blank_role() {
        let normalized = MsDemoTtsRequest {
            text: "x".to_string(),
            voice: "zh-CN-YunjianNeural".to_string(),
            style: Some("cheerful".to_string()),
            style_degree: Some(9.0),
            role: Some("  ".to_string()),
            rate: None,
            pitch: None,
            volume: None,
            format: None,
        }
        .normalize()
        .unwrap();
        // 越界强度夹到 2.0；空 role 回默认 default。
        assert_eq!(normalized.style_degree, 2.0);
        assert_eq!(normalized.role, "default");
    }

    #[test]
    fn normalizes_quality_format() {
        for (requested, expected) in [
            ("48k", ms_demo_tts::OUTPUT_FORMAT_48K),
            ("96k", ms_demo_tts::OUTPUT_FORMAT_96K),
            ("96Kbps", ms_demo_tts::OUTPUT_FORMAT_96K),
            ("192k", ms_demo_tts::OUTPUT_FORMAT_192K),
            (
                "audio-48khz-192kbitrate-mono-mp3",
                ms_demo_tts::OUTPUT_FORMAT_192K,
            ),
            ("128k", ms_demo_tts::OUTPUT_FORMAT_48K),
            ("garbage", ms_demo_tts::OUTPUT_FORMAT_48K),
        ] {
            let normalized = MsDemoTtsRequest {
                text: "x".to_string(),
                voice: "zh-CN-YunjianNeural".to_string(),
                style: None,
                style_degree: None,
                role: None,
                rate: None,
                pitch: None,
                volume: None,
                format: Some(requested.to_string()),
            }
            .normalize()
            .unwrap();
            assert_eq!(
                normalized.output_format, expected,
                "format = {requested}"
            );
        }
    }

    #[test]
    fn rejects_empty_text_and_voice() {
        let error = request("   ", "zh-CN-YunjianNeural").normalize().unwrap_err();
        assert!(error.to_string().contains("朗读文本为空"), "{error}");

        let error = request("x", " ").normalize().unwrap_err();
        assert!(error.to_string().contains("请选择音色"), "{error}");
    }

    #[test]
    fn deserializes_camel_case_payload() {
        // 前端 `invokeRaw('ms_tts_synthesize', { req: {...} })` 的形态。
        let request: MsDemoTtsRequest = serde_json::from_value(serde_json::json!({
            "text": "正文",
            "voice": "zh-CN-YunjianNeural",
            "style": "narration-relaxed",
            "styleDegree": 1.5,
            "role": "default",
            "rate": "+40%",
            "pitch": "-10%",
            "volume": "+20%",
            "format": "96k"
        }))
        .unwrap();
        assert_eq!(request.style.as_deref(), Some("narration-relaxed"));
        assert_eq!(request.style_degree, Some(1.5));
        assert_eq!(request.format.as_deref(), Some("96k"));
        assert_eq!(
            request.normalize().unwrap().output_format,
            ms_demo_tts::OUTPUT_FORMAT_96K
        );
    }
}
