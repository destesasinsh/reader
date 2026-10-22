import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invokeEnvelope, invokeRaw } from './invoke'
import {
  deleteHttpTts,
  getHttpTtsList,
  normalizeAudioContentType,
  readRemoteHttpTtsFile,
  saveHttpTtsList,
  sniffAudioMime,
  synthesizeHttpTts,
} from './httpTts'

vi.mock('./invoke', () => ({
  invokeEnvelope: vi.fn(),
  invokeRaw: vi.fn(),
}))

/** 用魔数拼一段最小字节，够嗅探即可（不需要真的是可解码音频）。 */
function bytes(...values: Array<number | string>): ArrayBuffer {
  const data = values.flatMap((value) => (
    typeof value === 'string'
      ? Array.from(value, (char) => char.charCodeAt(0))
      : [value]
  ))
  return new Uint8Array(data).buffer
}

function envelopeArgs(): { req: Record<string, unknown> } {
  return (vi.mocked(invokeEnvelope).mock.calls.at(-1)?.[1] || { req: {} }) as { req: Record<string, unknown> }
}

describe('httpTts api', () => {
  beforeEach(() => {
    vi.mocked(invokeEnvelope).mockReset()
    vi.mocked(invokeRaw).mockReset()
  })

  it('lists engines through the envelope command', async () => {
    const engines = [{ id: 1, name: '引擎甲', url: 'https://example.test/tts?text={{speakText}}' }]
    vi.mocked(invokeEnvelope).mockResolvedValue(engines)

    await expect(getHttpTtsList()).resolves.toEqual(engines)
    expect(invokeEnvelope).toHaveBeenCalledWith('get_http_tts_list', {})
  })

  it('saves the parsed list as `req` and reports skipped entries', async () => {
    vi.mocked(invokeEnvelope).mockResolvedValue({ saved: 2, skipped: 1 })
    const drafts = [
      { name: '引擎甲', url: 'https://example.test/a' },
      { name: '引擎乙', url: 'https://example.test/b' },
      { name: '', url: '' },
    ]

    await expect(saveHttpTtsList(drafts)).resolves.toEqual({ saved: 2, skipped: 1 })
    expect(invokeEnvelope).toHaveBeenCalledWith('save_http_tts_list', { req: drafts })
  })

  it('deletes by id only, never sending the name as well', async () => {
    // 后端是「id 命中或 name 命中」都删。两个一起传会顺手删掉同名兄弟引擎,
    // 所以有 id 时只能传 id —— 这是本文件最重要的一条契约。
    vi.mocked(invokeEnvelope).mockResolvedValue({ deleted: 1 })

    await deleteHttpTts({ id: 7, name: '同名引擎' })

    expect(invokeEnvelope).toHaveBeenCalledTimes(1)
    expect(invokeEnvelope).toHaveBeenCalledWith('delete_http_tts', { req: { id: 7 } })
    expect(Object.keys(envelopeArgs().req)).toEqual(['id'])
  })

  it('falls back to the name when the id is missing or zero', async () => {
    vi.mocked(invokeEnvelope).mockResolvedValue({ deleted: 1 })

    await deleteHttpTts({ name: ' 引擎甲 ' })
    expect(envelopeArgs().req).toEqual({ name: '引擎甲' })

    vi.mocked(invokeEnvelope).mockClear()
    await deleteHttpTts({ id: 0, name: '引擎甲' })
    expect(envelopeArgs().req).toEqual({ name: '引擎甲' })
  })

  it('rejects when neither id nor name is usable', async () => {
    await expect(deleteHttpTts({ id: null, name: '   ' })).rejects.toThrow('缺少要删除的朗读引擎标识')
    expect(invokeEnvelope).not.toHaveBeenCalled()
  })

  it('reads a remote engine file through the backend (no CORS in WebView)', async () => {
    const remote = ['[{"name":"引擎甲","url":"https://example.test/a"}]']
    vi.mocked(invokeEnvelope).mockResolvedValue(remote)

    await expect(readRemoteHttpTtsFile('https://example.test/tts.json')).resolves.toEqual(remote)
    expect(invokeEnvelope).toHaveBeenCalledWith('read_remote_http_tts_file', {
      req: { url: 'https://example.test/tts.json' },
    })
  })

  it('synthesizes one segment and tags the blob from the wav magic', async () => {
    vi.mocked(invokeRaw).mockResolvedValue(bytes('RIFF', 0, 0, 0, 0, 'WAVE', 0, 0, 0, 0))

    const blob = await synthesizeHttpTts({ ttsId: 12, text: '第一段正文', speed: 1.5 })

    expect(invokeRaw).toHaveBeenCalledWith('synthesize_http_tts', {
      req: { ttsId: 12, text: '第一段正文', speed: 1.5 },
    })
    // IPC 二进制没有 HTTP 头，引擎的 contentType 多是空串：MIME 只能靠嗅探。
    expect(blob.type).toBe('audio/wav')
    expect(blob.size).toBeGreaterThan(0)
  })

  it('accepts Uint8Array payloads and honours the configured contentType as fallback', async () => {
    vi.mocked(invokeRaw).mockResolvedValue(new Uint8Array([1, 2, 3, 4]))

    const blob = await synthesizeHttpTts({
      ttsId: 'engine-1',
      text: '正文',
      contentType: 'Audio/Ogg; charset=utf-8',
    })

    expect(blob.type).toBe('audio/ogg')
  })

  it('defaults to audio/mpeg when neither magic nor contentType is usable', async () => {
    vi.mocked(invokeRaw).mockResolvedValue(bytes(1, 2, 3))

    const blob = await synthesizeHttpTts({ ttsId: 3, text: '正文', contentType: 'text/html' })

    expect(blob.type).toBe('audio/mpeg')
  })

  it('rejects an empty audio body instead of playing silence', async () => {
    vi.mocked(invokeRaw).mockResolvedValue(new ArrayBuffer(0))

    await expect(synthesizeHttpTts({ ttsId: 1, text: '正文' })).rejects.toThrow('空音频')
  })

  it('turns a Tauri string rejection into an Error with the readable message', async () => {
    vi.mocked(invokeRaw).mockRejectedValue('引擎请求失败：接口返回了 HTML 错误页')

    await expect(synthesizeHttpTts({ ttsId: 1, text: '正文' })).rejects.toThrow('接口返回了 HTML 错误页')
  })

  it('strips the AppError category prefix but keeps the real message', async () => {
    // 后端 AppError::BadRequest 的 Display 是 "bad request: {消息}"，
    // 这层英文前缀不该出现在用户可见的 toast 里。
    vi.mocked(invokeRaw).mockRejectedValue('bad request: 引擎请求失败：接口返回了 HTML 错误页')

    await expect(synthesizeHttpTts({ ttsId: 1, text: '正文' }))
      .rejects.toThrow('引擎请求失败：接口返回了 HTML 错误页')

    vi.mocked(invokeRaw).mockRejectedValue('not found: 未找到朗读引擎')
    await expect(synthesizeHttpTts({ ttsId: 1, text: '正文' })).rejects.toThrow('未找到朗读引擎')
  })

  it('does not strip a prefix that carries no message after it', async () => {
    // 只剩前缀时保留原文，避免错误信息变成空串、最后只显示兜底文案。
    vi.mocked(invokeRaw).mockRejectedValue('bad request: ')

    await expect(synthesizeHttpTts({ ttsId: 1, text: '正文' })).rejects.toThrow('bad request:')
  })
})

describe('sniffAudioMime', () => {
  it('recognizes the formats Legado engines actually return', () => {
    expect(sniffAudioMime(bytes('RIFF', 0, 0, 0, 0, 'WAVE'))).toBe('audio/wav')
    expect(sniffAudioMime(bytes('ID3', 0, 0, 0))).toBe('audio/mpeg')
    expect(sniffAudioMime(bytes(0xff, 0xfb, 0x90, 0x64))).toBe('audio/mpeg')
    expect(sniffAudioMime(bytes('OggS', 0, 0, 0))).toBe('audio/ogg')
    expect(sniffAudioMime(bytes('fLaC', 0, 0, 0))).toBe('audio/flac')
    expect(sniffAudioMime(bytes(0, 0, 0, 0x18, 'ftypM4A '))).toBe('audio/mp4')
    expect(sniffAudioMime(bytes('FORM', 0, 0, 0, 0, 'AIFF'))).toBe('audio/aiff')
  })

  it('returns an empty string for unknown bytes so the caller can fall back', () => {
    expect(sniffAudioMime(bytes('not-an-audio'))).toBe('')
    expect(sniffAudioMime(new ArrayBuffer(0))).toBe('')
  })
})

describe('normalizeAudioContentType', () => {
  it('keeps only plain audio mime types', () => {
    expect(normalizeAudioContentType('audio/mpeg')).toBe('audio/mpeg')
    expect(normalizeAudioContentType(' Audio/WAV ; charset=utf-8')).toBe('audio/wav')
    expect(normalizeAudioContentType('text/html')).toBe('')
    expect(normalizeAudioContentType(undefined)).toBe('')
  })
})
