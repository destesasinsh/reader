// 微软朗读（微软翻译客户端演示接口）API。
//
// 这是第 5 个朗读通道，比 Edge 通道多了「风格 style」：音色自带 StyleList，
// 同一个音色可以说成新闻/客服/撒娇等语气，还能调风格强度 styleDegree。
// 后端负责协议细节（微软翻译客户端演示接口），前端只做音色/风格选择与参数下发。
//
// 与 edgeTts 一样：音频经 IPC 二进制通道返回、没有 HTTP 头，MIME 靠魔数嗅探
// （返回 mp3）；错误是 AppError 序列化后的字符串，会带 `bad request: ` 前缀。

import { invokeEnvelope, invokeRaw } from './invoke'
import { blobFromAudioPayload, toError } from './httpTts'

/** 微软朗读只支持的音质档位（顺序即 UI 里的从低到高）。 */
export type MsTtsFormat = '48k' | '96k' | '192k'

export interface MsTtsVoice {
  /** 稳定音色 id，如 `zh-CN-YunjianNeural`。 */
  shortName: string
  /** 如 `zh-CN` / `en-US`。 */
  locale: string
  gender: string
  friendlyName: string
  /** 该音色支持的风格（来自服务端的 StyleList）；空数组表示只能用默认语气。 */
  styles: string[]
}

export interface SynthesizeMsTtsParams {
  text: string
  voice: string
  /** 空/不传 = 无风格。 */
  style?: string
  /** 风格强度，默认 1.0（仅在有 style 时生效）。 */
  styleDegree?: number
  /** 角色，默认 default（仅在有 style 时生效）。前端暂无 UI，保留给调用方。 */
  role?: string
  /** 带符号百分比串，如 `+40%` / `-10%` / `+0%`。 */
  rate?: string
  /** 带符号百分比串（注意这个通道的 pitch 也是百分比，不是 Hz）。 */
  pitch?: string
  /** 带符号百分比串，如 `-30%` / `+0%`。 */
  volume?: string
  format?: MsTtsFormat
}

/** 列出可用音色（后端已按中文优先排序，zh-CN 在最前）。 */
export function listMsTtsVoices(): Promise<MsTtsVoice[]> {
  return invokeEnvelope<MsTtsVoice[]>('ms_tts_list_voices', {})
}

/**
 * 合成一段文本，返回可直接喂给 `<audio>` 的 Blob。
 *
 * MIME 嗅探与错误前缀剥离都复用 httpTts 的实现：所有二进制朗读通道的错误形态
 * 一致，各写一份迟早会出现「某个通道 toast 带 bad request:」。
 */
export async function synthesizeMsTts(params: SynthesizeMsTtsParams): Promise<Blob> {
  let payload: ArrayBuffer | Uint8Array | number[]
  try {
    payload = await invokeRaw<ArrayBuffer | Uint8Array | number[]>('ms_tts_synthesize', {
      req: {
        text: params.text,
        voice: params.voice,
        // 可选参数不传就不塞进 req：让后端走默认值，别把 undefined 序列化成
        // 字符串塞进 SSML 属性（style 为空表示无风格，不能传空串以外的占位）。
        ...(params.style ? { style: params.style } : {}),
        ...(params.style ? { styleDegree: params.styleDegree ?? 1 } : {}),
        ...(params.role ? { role: params.role } : {}),
        ...(params.rate ? { rate: params.rate } : {}),
        ...(params.pitch ? { pitch: params.pitch } : {}),
        ...(params.volume ? { volume: params.volume } : {}),
        ...(params.format ? { format: params.format } : {}),
      },
    })
  } catch (error) {
    throw toError(error, '微软朗读请求失败')
  }
  return blobFromAudioPayload(payload)
}
