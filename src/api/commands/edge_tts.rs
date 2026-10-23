use serde::Deserialize;
use serde_json::Value;

use crate::api::AppState;
use crate::error::error::{ApiResponse, AppError};
use crate::service::edge_tts;

/// Edge 朗读合成参数。
///
/// `rate` / `pitch` / `volume` 直接是 SSML 属性形态（`+10%` / `-5Hz` / `+0%`），
/// 缺省时用系统默认值。前端做的是「档位→百分比」的换算，后端不再二次解释。
/// `format` 是音质档（`48k` / `96k` 或完整 outputFormat 串），缺失/非法回退 48k。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeTtsSynthesizeRequest {
    pub text: String,
    pub voice: String,
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
struct NormalizedEdgeTtsRequest {
    text: String,
    voice: String,
    rate: String,
    pitch: String,
    volume: String,
    /// 已归一化成端点白名单里的 outputFormat（&'static str，无需再校验）。
    output_format: &'static str,
}

impl EdgeTtsSynthesizeRequest {
    fn normalize(self) -> Result<NormalizedEdgeTtsRequest, AppError> {
        let text = self.text.trim().to_string();
        if text.is_empty() {
            return Err(AppError::BadRequest("朗读文本为空".to_string()));
        }
        let voice = self.voice.trim().to_string();
        if voice.is_empty() {
            return Err(AppError::BadRequest(
                "请先选择 Edge 朗读音色".to_string(),
            ));
        }
        Ok(NormalizedEdgeTtsRequest {
            text,
            voice,
            // pitch 用 `Hz`，rate/volume 用 `%` —— 单位由通道决定，不能互串。
            rate: crate::service::tts_param::normalize_offset(self.rate.as_deref(), "+0%", &["%"]),
            pitch: crate::service::tts_param::normalize_offset(
                self.pitch.as_deref(),
                "+0Hz",
                &["Hz"],
            ),
            volume: crate::service::tts_param::normalize_offset(
                self.volume.as_deref(),
                "+0%",
                &["%"],
            ),
            output_format: edge_tts::normalize_output_format(self.format.as_deref()),
        })
    }
}

/// 列出 Edge 朗读音色的共享实现（进程内缓存，第一次成功拉取后复用）。
pub async fn edge_tts_list_voices_core(
    state: &AppState,
) -> Result<ApiResponse<Vec<Value>>, AppError> {
    let client = state.book_service.http_client();
    Ok(ApiResponse::ok(edge_tts::list_voices(&client).await?))
}

/// `edge_tts_list_voices` 的 Tauri 入口: 只是 core 的薄壳。
#[tauri::command]
pub async fn edge_tts_list_voices(
    state: tauri::State<'_, AppState>,
) -> Result<ApiResponse<Vec<Value>>, AppError> {
    edge_tts_list_voices_core(&state).await
}

/// 合成一段 Edge 朗读音频的共享实现（mp3 字节，前端 `invokeRaw` 取 ArrayBuffer）。
///
/// 合成本身只依赖请求参数，但**代理必须来自 `AppState`**：WebSocket 由
/// tungstenite 自己建 TCP，拿不到 `reqwest::Client` 上的代理配置；不显式传下去，
/// 「设置里配了代理」的用户会看到「音色能列、合成必失败」。
pub async fn edge_tts_synthesize_core(
    state: &AppState,
    req: EdgeTtsSynthesizeRequest,
) -> Result<tauri::ipc::Response, AppError> {
    let request = req.normalize()?;
    let proxy = state.book_service.active_proxy();
    let audio = edge_tts::synthesize(
        &request.text,
        &request.voice,
        &request.rate,
        &request.pitch,
        &request.volume,
        request.output_format,
        proxy.as_deref(),
    )
    .await?;
    Ok(tauri::ipc::Response::new(audio))
}

/// `edge_tts_synthesize` 的 Tauri 入口: 只是 core 的薄壳。
#[tauri::command]
pub async fn edge_tts_synthesize(
    state: tauri::State<'_, AppState>,
    req: EdgeTtsSynthesizeRequest,
) -> Result<tauri::ipc::Response, AppError> {
    edge_tts_synthesize_core(&state, req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_defaults_for_missing_or_blank_params() {
        let request = EdgeTtsSynthesizeRequest {
            text: "  你好世界  ".to_string(),
            voice: " zh-CN-XiaoxiaoNeural ".to_string(),
            rate: None,
            pitch: Some("".to_string()),
            volume: Some("   ".to_string()),
            format: None,
        }
        .normalize()
        .unwrap();
        assert_eq!(request.text, "你好世界");
        assert_eq!(request.voice, "zh-CN-XiaoxiaoNeural");
        assert_eq!(request.rate, "+0%");
        assert_eq!(request.pitch, "+0Hz");
        assert_eq!(request.volume, "+0%");
        // 音质缺失 → 默认 48k。
        assert_eq!(request.output_format, edge_tts::OUTPUT_FORMAT_48K);
    }

    #[test]
    fn keeps_explicit_ssml_params() {
        let request = EdgeTtsSynthesizeRequest {
            text: "x".to_string(),
            voice: "zh-CN-YunxiNeural".to_string(),
            rate: Some("+20%".to_string()),
            pitch: Some("-10Hz".to_string()),
            volume: Some("-30%".to_string()),
            format: None,
        }
        .normalize()
        .unwrap();
        assert_eq!(request.rate, "+20%");
        assert_eq!(request.pitch, "-10Hz");
        assert_eq!(request.volume, "-30%");
    }

    #[test]
    fn rejects_empty_text_and_voice() {
        let empty_text = EdgeTtsSynthesizeRequest {
            text: "   ".to_string(),
            voice: "zh-CN-XiaoxiaoNeural".to_string(),
            rate: None,
            pitch: None,
            volume: None,
            format: None,
        }
        .normalize();
        assert!(empty_text.is_err());
        assert!(empty_text
            .unwrap_err()
            .to_string()
            .contains("朗读文本为空"));

        let empty_voice = EdgeTtsSynthesizeRequest {
            text: "x".to_string(),
            voice: "".to_string(),
            rate: None,
            pitch: None,
            volume: None,
            format: None,
        }
        .normalize();
        assert!(empty_voice.is_err());
        assert!(empty_voice
            .unwrap_err()
            .to_string()
            .contains("请先选择 Edge 朗读音色"));
    }

    #[test]
    fn deserializes_camel_case_payload() {
        // 前端 `invoke('edge_tts_synthesize', { req: {...} })` 的形态。
        let request: EdgeTtsSynthesizeRequest = serde_json::from_value(serde_json::json!({
            "text": "正文",
            "voice": "zh-CN-XiaoxiaoNeural",
            "rate": "+10%",
            "format": "96k"
        }))
        .unwrap();
        assert_eq!(request.rate.as_deref(), Some("+10%"));
        assert_eq!(request.pitch, None);
        assert_eq!(request.format.as_deref(), Some("96k"));
        assert_eq!(
            request.normalize().unwrap().output_format,
            edge_tts::OUTPUT_FORMAT_96K
        );
    }

    fn normalized_with_format(format: Option<&str>) -> NormalizedEdgeTtsRequest {
        EdgeTtsSynthesizeRequest {
            text: "正文".to_string(),
            voice: "zh-CN-XiaoxiaoNeural".to_string(),
            rate: None,
            pitch: None,
            volume: None,
            format: format.map(str::to_string),
        }
        .normalize()
        .unwrap()
    }

    /// 音质归一化在命令层落地：前端传的档位/简写/完整串都要映射到白名单，
    /// 非法值静默回退 48k（不能因为音质参数让朗读失败）。
    #[test]
    fn normalizes_quality_format() {
        for requested in [
            Some("96k"),
            Some("96K"),
            Some("96Kbps"),
            Some("audio-24khz-96kbitrate-mono-mp3"),
        ] {
            assert_eq!(
                normalized_with_format(requested).output_format,
                edge_tts::OUTPUT_FORMAT_96K,
                "format = {requested:?}"
            );
        }
        for requested in [
            None,
            Some(""),
            Some("48k"),
            Some("48Kbps"),
            Some("audio-24khz-48kbitrate-mono-mp3"),
            // 历史脏值 / 端点不支持的档位。
            Some("128k"),
            Some("48khz"),
            Some("audio-24khz-96kbitrate-mono-pcm"),
            Some("garbage"),
        ] {
            assert_eq!(
                normalized_with_format(requested).output_format,
                edge_tts::OUTPUT_FORMAT_48K,
                "format = {requested:?}"
            );
        }
    }
}
