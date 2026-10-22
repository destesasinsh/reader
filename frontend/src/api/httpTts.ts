// 在线朗读引擎（Legado httpTTS）API。
//
// 为什么合成必须走后端 IPC、而不是在 WebView 里直接 fetch 引擎地址：
// 1. 引擎 URL 里的模板（`{{java.encodeURI(speakText)}}`、语速四则运算、JS 片段）
//    要用 rquickjs 沙盒求值，前端没有等价能力；后端展开后才能拿到真实 URL。
// 2. 这个仓库是 Tauri 桌面应用（origin `http://tauri.localhost`），音频站点的
//    CORS 头并不可控，跨域读 `response.arrayBuffer()` 会被拦；后端 reqwest
//    直接拿字节，再经 IPC 二进制通道返回。
// 3. 后端还会补 `header`、处理 POST body、识别错误页（返回 HTML/JSON 报错的
//    200 响应），这些逻辑放在前端会与书源请求重复一遍。

import { invokeEnvelope, invokeRaw } from './invoke'
import type { HttpTts } from '../types'

/** 保存结果。`skipped` 是后端按「缺 name/url」筛掉的条数，必须显式告知用户。 */
export interface HttpTtsSaveResult {
  saved: number
  skipped: number
}

/** 导入时允许残缺：id 由后端补发，name/url 由后端校验（缺了计入 skipped）。 */
export type HttpTtsDraft = Partial<HttpTts>

/** 列出已导入的在线朗读引擎（后端按 name 升序返回）。 */
export function getHttpTtsList() {
  return invokeEnvelope<HttpTts[]>('get_http_tts_list', {})
}

/**
 * 保存/导入引擎列表。
 *
 * 后端接受裸数组、`{data:[...]}` 包装或单个对象，并按 name 优先 upsert
 * （同名覆盖保留原 id）。这里原样把解析结果交出去，宽容性交给后端。
 */
export function saveHttpTtsList(engines: HttpTtsDraft[] | { data: HttpTtsDraft[] }) {
  return invokeEnvelope<HttpTtsSaveResult>('save_http_tts_list', { req: engines })
}

export interface HttpTtsDeleteTarget {
  id?: number | string | null
  name?: string
}

function hasUsableId(id: number | string | null | undefined): boolean {
  if (id == null) return false
  if (typeof id === 'number') return Number.isFinite(id) && id !== 0
  return id.trim() !== '' && id.trim() !== '0'
}

/**
 * 删除一条引擎。
 *
 * 后端按「id 命中 **或** name 命中」删除，不是「优先 id，放弃 name」——
 * 两个键同时传时同名兄弟条目会被一起删掉。所以这里只放一个键：
 * 有可用 id 就只传 id，否则才退回 name。
 */
export function deleteHttpTts(target: HttpTtsDeleteTarget) {
  const req = hasUsableId(target.id)
    ? { id: target.id }
    : target.name?.trim()
      ? { name: target.name.trim() }
      : null
  if (!req) return Promise.reject(new Error('缺少要删除的朗读引擎标识'))
  return invokeEnvelope<{ deleted: number }>('delete_http_tts', { req })
}

/**
 * 读取远程引擎文件。`vec[0]` 是整份 `HttpTts[]` JSON 文本（与
 * `read_remote_source_file` 同形态），所以调用方要自己 `JSON.parse`。
 */
export function readRemoteHttpTtsFile(url: string) {
  return invokeEnvelope<string[]>('read_remote_http_tts_file', { req: { url } })
}

export interface SynthesizeHttpTtsParams {
  ttsId: number | string
  /** 调用方交来的一段文本（缓冲流按块调用），一次请求。 */
  text: string
  /** 倍速，对应 `speechConfig.speechRate`；后端换算成 Legado 的 speakSpeed 档位。 */
  speed?: number
  /** 引擎配置里的 contentType，仅在魔数嗅探失败时兜底。 */
  contentType?: string
}

/**
 * AppError 经 Tauri Serialize 后带上的英文分类前缀。
 *
 * `src/error/error.rs` 里 `AppError::BadRequest` 的 Display 是
 * `"bad request: {0}"`，于是前端 reject 字符串实际长这样：
 * `bad request: 引擎请求失败：...`。不剥掉的话用户看到的 toast 永远以
 * `bad request: ` 开头（英文、无意义）。
 */
const APP_ERROR_PREFIXES = ['bad request: ', 'not found: ', 'internal error: ']

/** 剥掉一层 AppError 英文前缀；剥完为空时保留原文，绝不吞掉错误内容。 */
function stripAppErrorPrefix(message: string): string {
  const trimmed = message.trim()
  for (const prefix of APP_ERROR_PREFIXES) {
    if (trimmed.toLowerCase().startsWith(prefix)) {
      const rest = trimmed.slice(prefix.length).trim()
      return rest || trimmed
    }
  }
  return trimmed
}

/**
 * Tauri reject（AppError 序列化后的字符串）→ Error，并剥掉英文分类前缀。
 *
 * 导出给其它朗读通道（如 `edgeTts.ts`）复用：所有 command 的错误形态一致，
 * 各自再写一份剥离逻辑迟早会出现「某个通道 toast 带 bad request:」的问题。
 */
export function toError(error: unknown, fallback: string): Error {
  if (error instanceof Error) return error
  // Tauri 的 reject 值来自 AppError 的自定义 Serialize，是一个消息字符串。
  const message = typeof error === 'string' ? stripAppErrorPrefix(error) : ''
  return new Error(message || fallback)
}

function toAudioBytes(payload: ArrayBuffer | Uint8Array | number[]): ArrayBuffer {
  if (payload instanceof ArrayBuffer) return payload
  if (payload instanceof Uint8Array) {
    return payload.buffer.slice(payload.byteOffset, payload.byteOffset + payload.byteLength) as ArrayBuffer
  }
  // 兜底：部分平台会把二进制当数字数组送过来（不应发生，但不能静默出空音频）。
  return new Uint8Array(payload).buffer
}

function startsWithAscii(bytes: Uint8Array, offset: number, text: string): boolean {
  if (bytes.length < offset + text.length) return false
  for (let i = 0; i < text.length; i += 1) {
    if (bytes[offset + i] !== text.charCodeAt(i)) return false
  }
  return true
}

/**
 * 按魔数嗅探音频 MIME。
 *
 * 为什么非做不可：二进制从 IPC 回来时只有裸字节，没有 HTTP 响应头；而 Legado
 * 引擎配置里 `contentType` 大多数是空串。MIME 给错（比如一律 `audio/mpeg`）
 * 时 `<audio>` 不会报可用的错误，而是**静默 onerror / 直接无声**，用户只会
 * 觉得「朗读没反应」。
 *
 * 返回空串表示没认出来，由调用方用配置里的 contentType 或默认值兜底。
 */
export function sniffAudioMime(bytes: ArrayBuffer): string {
  const view = new Uint8Array(bytes)
  // WAV: "RIFF" + 4 字节长度 + "WAVE"
  if (startsWithAscii(view, 0, 'RIFF') && startsWithAscii(view, 8, 'WAVE')) return 'audio/wav'
  // AIFF/AIFC: "FORM" + 4 字节长度 + "AIFF"/"AIFC"
  if (startsWithAscii(view, 0, 'FORM') && (startsWithAscii(view, 8, 'AIFF') || startsWithAscii(view, 8, 'AIFC'))) {
    return 'audio/aiff'
  }
  if (startsWithAscii(view, 0, 'OggS')) return 'audio/ogg'
  if (startsWithAscii(view, 0, 'fLaC')) return 'audio/flac'
  // MP4 容器（m4a/aac）：第 4..8 字节固定是 "ftyp"。后端也认这种，前端不能漏。
  if (startsWithAscii(view, 4, 'ftyp')) return 'audio/mp4'
  // MP3: ID3 标签，或裸 MPEG 帧的 11 位同步字（0xFF 后高 3 位全 1）。
  if (startsWithAscii(view, 0, 'ID3')) return 'audio/mpeg'
  if (view.length >= 2 && view[0] === 0xff && (view[1] & 0xe0) === 0xe0) return 'audio/mpeg'
  return ''
}

/** 把配置里的 contentType 收成 `audio/*`；带 charset 等参数时只取主类型。 */
export function normalizeAudioContentType(contentType?: string): string {
  const mime = (contentType || '').split(';')[0].trim().toLowerCase()
  return mime.startsWith('audio/') ? mime : ''
}

/**
 * 二进制 → Blob：MIME 先嗅探，再退配置值，最后按 mp3 兜底。
 *
 * 导出给 edgeTts 等其它二进制朗读命令复用：它们同样拿不到 HTTP 头，只能靠
 * 魔数判断格式（edge 返回 mp3，嗅探结果为 audio/mpeg）。
 */
export function blobFromAudioPayload(
  payload: ArrayBuffer | Uint8Array | number[],
  fallbackContentType?: string,
): Blob {
  const bytes = toAudioBytes(payload)
  if (!bytes.byteLength) throw new Error('在线朗读引擎返回了空音频')
  const mime = sniffAudioMime(bytes) || normalizeAudioContentType(fallbackContentType) || 'audio/mpeg'
  return new Blob([bytes], { type: mime })
}

/**
 * 合成一段文本，返回可直接喂给 `<audio>` 的 Blob。
 *
 * 后端失败时返回的是 AppError 序列化后的字符串（可读中文），这里统一包成
 * Error，避免上层 `onError` 收到字符串后 `.message` 为空。
 */
export async function synthesizeHttpTts(params: SynthesizeHttpTtsParams): Promise<Blob> {
  let payload: ArrayBuffer | Uint8Array | number[]
  try {
    payload = await invokeRaw<ArrayBuffer | Uint8Array | number[]>('synthesize_http_tts', {
      req: {
        ttsId: params.ttsId,
        text: params.text,
        speed: params.speed ?? 1,
      },
    })
  } catch (error) {
    throw toError(error, '在线朗读请求失败')
  }
  return blobFromAudioPayload(payload, params.contentType)
}
