import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invokeEnvelope, invokeRaw } from './invoke'
import { listMsTtsVoices, synthesizeMsTts, type MsTtsVoice } from './msTts'

vi.mock('./invoke', () => ({
  invokeEnvelope: vi.fn(),
  invokeRaw: vi.fn(),
}))

function bytes(...values: Array<number | string>): ArrayBuffer {
  const data = values.flatMap((value) => (
    typeof value === 'string'
      ? Array.from(value, (char) => char.charCodeAt(0))
      : [value]
  ))
  return new Uint8Array(data).buffer
}

const VOICES: MsTtsVoice[] = [
  {
    shortName: 'zh-CN-YunjianNeural',
    locale: 'zh-CN',
    gender: 'Male',
    friendlyName: '云健',
    styles: ['newscast', 'sports'],
  },
  { shortName: 'en-US-AriaNeural', locale: 'en-US', gender: 'Female', friendlyName: 'Aria', styles: [] },
]

describe('msTts api', () => {
  beforeEach(() => {
    vi.mocked(invokeEnvelope).mockReset()
    vi.mocked(invokeRaw).mockReset()
  })

  it('lists voices through the envelope command', async () => {
    vi.mocked(invokeEnvelope).mockResolvedValue(VOICES)

    await expect(listMsTtsVoices()).resolves.toEqual(VOICES)
    expect(invokeEnvelope).toHaveBeenCalledWith('ms_tts_list_voices', {})
  })

  it('passes voice/style/params through and tags the returned mp3', async () => {
    vi.mocked(invokeRaw).mockResolvedValue(bytes('ID3', 0, 0, 0))

    const blob = await synthesizeMsTts({
      text: '测试文本',
      voice: 'zh-CN-YunjianNeural',
      style: 'newscast',
      styleDegree: 1.5,
      rate: '+40%',
      pitch: '-10%',
      volume: '+0%',
      format: '192k',
    })

    expect(invokeRaw).toHaveBeenCalledWith('ms_tts_synthesize', {
      req: {
        text: '测试文本',
        voice: 'zh-CN-YunjianNeural',
        style: 'newscast',
        styleDegree: 1.5,
        rate: '+40%',
        pitch: '-10%',
        volume: '+0%',
        format: '192k',
      },
    })
    expect(blob.type).toBe('audio/mpeg')
    expect(blob.size).toBeGreaterThan(0)
  })

  it('does not send style/styleDegree when style is empty', async () => {
    vi.mocked(invokeRaw).mockResolvedValue(bytes(0xff, 0xfb, 0x90, 0x64))

    await synthesizeMsTts({ text: '正文', voice: 'zh-CN-YunjianNeural' })

    expect(vi.mocked(invokeRaw).mock.calls[0][1]).toEqual({
      req: { text: '正文', voice: 'zh-CN-YunjianNeural' },
    })
  })

  it('defaults styleDegree to 1 when a style is used without a degree', async () => {
    vi.mocked(invokeRaw).mockResolvedValue(bytes('ID3', 0, 0, 0))

    await synthesizeMsTts({ text: '正文', voice: 'zh-CN-YunjianNeural', style: 'sports' })

    expect(vi.mocked(invokeRaw).mock.calls[0][1]).toEqual({
      req: {
        text: '正文',
        voice: 'zh-CN-YunjianNeural',
        style: 'sports',
        styleDegree: 1,
      },
    })
  })

  it('strips the AppError prefix from backend errors', async () => {
    vi.mocked(invokeRaw).mockRejectedValue('bad request: 微软朗读服务暂时不可用')

    await expect(synthesizeMsTts({ text: '正文', voice: 'zh-CN-YunjianNeural' }))
      .rejects.toThrow('微软朗读服务暂时不可用')
  })

  it('rejects an empty audio body instead of playing silence', async () => {
    vi.mocked(invokeRaw).mockResolvedValue(new ArrayBuffer(0))

    await expect(synthesizeMsTts({ text: '正文', voice: 'zh-CN-YunjianNeural' }))
      .rejects.toThrow('空音频')
  })
})
