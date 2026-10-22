// Edge 朗读（edge-tts）API。
//
// 这是微软 Edge「大声朗读」背后的同一套在线语音服务：后端用 WebSocket + DRM
// token 拿音频，所以前端只需要「选音色 + 发文本」两条命令，不需要 API Key。
//
// 与在线朗读引擎（httpTts）一样，音频经 IPC 二进制通道回来、没有 HTTP 头，
// 因此 Blob 的 MIME 只能靠魔数嗅探（edge 返回 mp3）；错误消息同样是 AppError
// 序列化后的字符串，会带 `bad request: ` 之类的英文前缀。

import { invokeEnvelope, invokeRaw } from './invoke'
import { blobFromAudioPayload, toError } from './httpTts'

export interface EdgeTtsVoice {
  /** Edge 的稳定音色 id，如 `zh-CN-YunxiNeural`。 */
  shortName: string
  /** 如 `zh-CN` / `en-US`。 */
  locale: string
  /** `Male` / `Female`。 */
  gender: string
  /** 可读名称（中文音色会带中文名）。 */
  friendlyName: string
}

/** Edge 音质档位：只支持这两种 outputFormat（其它采样率/码率会被服务端拒绝）。 */
export type EdgeTtsFormat = '48k' | '96k'

export interface SynthesizeEdgeTtsParams {
  text: string
  /** `shortName`，例如 `zh-CN-YunxiNeural`。 */
  voice: string
  /** 带符号的百分比，如 `-20%` / `+15%` / `+0%`；不传则用服务端默认。 */
  rate?: string
  /** 带符号的音调，如 `-20Hz` / `+15Hz` / `+0Hz`；不传则用服务端默认。 */
  pitch?: string
  /** 带符号的音量，如 `-30%` / `+20%` / `+0%`；不传则用服务端默认。 */
  volume?: string
  /** 音质档位；后端认 `96k` / `96Kbps` / 完整 outputFormat 串。不传用后端默认 48k。 */
  format?: EdgeTtsFormat
}

/** 列出可用音色。后端已按中文优先排序（zh-CN 在最前）。 */
export function listEdgeTtsVoices(): Promise<EdgeTtsVoice[]> {
  return invokeEnvelope<EdgeTtsVoice[]>('edge_tts_list_voices', {})
}

/**
 * 合成一段文本，返回可直接喂给 `<audio>` 的 Blob。
 *
 * 失败时把 Tauri 的字符串 reject 统一包成 Error 并剥掉英文前缀，避免 toast 里
 * 出现 `bad request: Edge 朗读服务暂时不可用…` 这种半截英文。
 */
export async function synthesizeEdgeTts(params: SynthesizeEdgeTtsParams): Promise<Blob> {
  let payload: ArrayBuffer | Uint8Array | number[]
  try {
    payload = await invokeRaw<ArrayBuffer | Uint8Array | number[]>('edge_tts_synthesize', {
      req: {
        text: params.text,
        voice: params.voice,
        // rate / pitch / volume 都可省略：省略时交给后端默认值（+0% / +0Hz），
        // 别把 undefined 序列化成字符串塞进 SSML 属性。
        ...(params.rate ? { rate: params.rate } : {}),
        ...(params.pitch ? { pitch: params.pitch } : {}),
        ...(params.volume ? { volume: params.volume } : {}),
        ...(params.format ? { format: params.format } : {}),
      },
    })
  } catch (error) {
    throw toError(error, 'Edge 朗读请求失败')
  }
  return blobFromAudioPayload(payload)
}
