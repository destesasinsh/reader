import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invokeEnvelope, invokeRaw } from './invoke'
import { listEdgeTtsVoices, synthesizeEdgeTts, type EdgeTtsVoice } from './edgeTts'

vi.mock('./invoke', () => ({
  invokeEnvelope: vi.fn(),
  invokeRaw: vi.fn(),
}))

/** 拼一段最小字节：mp3 的魔数（ID3 或帧同步）足以让嗅探识别。 */
function bytes(...values: Array<number | string>): ArrayBuffer {
  const data = values.flatMap((value) => (
    typeof value === 'string'
      ? Array.from(value, (char) => char.charCodeAt(0))
      : [value]
  ))
  return new Uint8Array(data).buffer
}

const VOICES: EdgeTtsVoice[] = [
  { shortName: 'zh-CN-YunxiNeural', locale: 'zh-CN', gender: 'Male', friendlyName: '云希' },
  { shortName: 'en-US-AriaNeural', locale: 'en-US', gender: 'Female', friendlyName: 'Aria' },
]

describe('edgeTts api', () => {
  beforeEach(() => {
    vi.mocked(invokeEnvelope).mockReset()
    vi.mocked(invokeRaw).mockReset()
  })

  it('lists voices through the envelope command', async () => {
    vi.mocked(invokeEnvelope).mockResolvedValue(VOICES)

    await expect(listEdgeTtsVoices()).resolves.toEqual(VOICES)
    expect(invokeEnvelope).toHaveBeenCalledWith('edge_tts_list_voices', {})
  })

  it('sends text/voice/rate and tags the returned mp3 as audio/mpeg', async () => {
    // ID3 标签是 mp3 最常见的开头（edge-tts 返回的就是带标签的 mp3）。
    vi.mocked(invokeRaw).mockResolvedValue(bytes('ID3', 0, 0, 0, 0, 0))

    const blob = await synthesizeEdgeTts({
      text: '测试文本',
      voice: 'zh-CN-YunxiNeural',
      rate: '-20%',
    })

    expect(invokeRaw).toHaveBeenCalledWith('edge_tts_synthesize', {
      req: { text: '测试文本', voice: 'zh-CN-YunxiNeural', rate: '-20%' },
    })
    // IPC 二进制没有 HTTP 头，MIME 靠魔数嗅探。
    expect(blob.type).toBe('audio/mpeg')
    expect(blob.size).toBeGreaterThan(0)
  })

  it('passes pitch, volume and format through when provided', async () => {
    vi.mocked(invokeRaw).mockResolvedValue(bytes('ID3', 0, 0, 0))

    await synthesizeEdgeTts({
      text: '正文',
      voice: 'zh-CN-XiaoxiaoNeural',
      rate: '+15%',
      pitch: '-20Hz',
      volume: '-30%',
      format: '96k',
    })

    expect(vi.mocked(invokeRaw).mock.calls[0][1]).toEqual({
      req: {
        text: '正文',
        voice: 'zh-CN-XiaoxiaoNeural',
        rate: '+15%',
        pitch: '-20Hz',
        volume: '-30%',
        format: '96k',
      },
    })
  })

  it('omits rate/pitch/volume/format when the caller does not set them', async () => {
    vi.mocked(invokeRaw).mockResolvedValue(bytes(0xff, 0xfb, 0x90, 0x64))

    const blob = await synthesizeEdgeTts({ text: '正文', voice: 'zh-CN-YunxiNeural' })

    expect(vi.mocked(invokeRaw).mock.calls[0][1]).toEqual({
      req: { text: '正文', voice: 'zh-CN-YunxiNeural' },
    })
    // 裸 MPEG 帧同步也要认成 mp3。
    expect(blob.type).toBe('audio/mpeg')
  })

  it('falls back to audio/mpeg for bytes it cannot sniff', async () => {
    vi.mocked(invokeRaw).mockResolvedValue(bytes(1, 2, 3, 4))

    const blob = await synthesizeEdgeTts({ text: '正文', voice: 'zh-CN-YunxiNeural' })
    expect(blob.type).toBe('audio/mpeg')
  })

  it('strips the AppError prefix from backend errors', async () => {
    vi.mocked(invokeRaw).mockRejectedValue('bad request: Edge 朗读服务暂时不可用，请稍后再试')

    await expect(synthesizeEdgeTts({ text: '正文', voice: 'zh-CN-YunxiNeural' }))
      .rejects.toThrow('Edge 朗读服务暂时不可用，请稍后再试')
  })

  it('wraps a non-string rejection with a readable fallback', async () => {
    vi.mocked(invokeRaw).mockRejectedValue({ message: 'weird' })

    await expect(synthesizeEdgeTts({ text: '正文', voice: 'zh-CN-YunxiNeural' }))
      .rejects.toThrow('Edge 朗读请求失败')
  })

  it('rejects an empty audio body instead of playing silence', async () => {
    vi.mocked(invokeRaw).mockResolvedValue(new ArrayBuffer(0))

    await expect(synthesizeEdgeTts({ text: '正文', voice: 'zh-CN-YunxiNeural' }))
      .rejects.toThrow('空音频')
  })
})
