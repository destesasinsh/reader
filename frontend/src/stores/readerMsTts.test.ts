import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useAppStore } from './app'
import {
  DEFAULT_MS_FORMAT,
  DEFAULT_MS_VOICE,
  MS_PITCH_MAX,
  MS_PITCH_MIN,
  MS_STYLE_DEGREE_MAX,
  MS_STYLE_DEGREE_MIN,
  MS_VOLUME_MAX,
  MS_VOLUME_MIN,
  useReaderStore,
} from './reader'
import { synthesizeMsTts } from '../api/msTts'

vi.mock('../api/bookshelf', () => ({
  getChapterList: vi.fn(),
  getBookInfo: vi.fn(),
  getBookContent: vi.fn(),
  saveBookProgress: vi.fn(),
  setBookSource: vi.fn(),
}))

vi.mock('../api/bookmark', () => ({
  getBookmarks: vi.fn(),
  saveBookmark: vi.fn(),
  deleteBookmark: vi.fn(),
  deleteBookmarks: vi.fn(),
}))

vi.mock('../api/replaceRule', () => ({
  getReplaceRules: vi.fn(),
  applyReplaceRulesInSandbox: vi.fn(),
}))

vi.mock('../api/webdav', () => ({
  syncLegadoBookProgress: vi.fn(),
  uploadLegadoBookProgress: vi.fn(),
}))

vi.mock('../utils/recentBooks', () => ({
  saveRecentReadBook: vi.fn(),
}))

vi.mock('../utils/openaiSpeech', () => ({
  DEFAULT_OPENAI_BASE_URL: 'https://api.openai.com/v1',
  inferSpeechApiFormat: vi.fn(() => 'openai'),
  getSpeechApiFormatOption: vi.fn(() => ({ supportedFormats: ['mp3', 'wav', 'opus', 'flac', 'pcm'] })),
  requestOpenAISpeechAudio: vi.fn(),
}))

vi.mock('../api/httpTts', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/httpTts')>()
  return {
    ...actual,
    getHttpTtsList: vi.fn(),
    saveHttpTtsList: vi.fn(),
    deleteHttpTts: vi.fn(),
    readRemoteHttpTtsFile: vi.fn(),
    synthesizeHttpTts: vi.fn(),
  }
})

vi.mock('../api/edgeTts', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/edgeTts')>()
  return {
    ...actual,
    listEdgeTtsVoices: vi.fn(),
    synthesizeEdgeTts: vi.fn(),
  }
})

vi.mock('../api/msTts', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/msTts')>()
  return {
    ...actual,
    listMsTtsVoices: vi.fn(),
    synthesizeMsTts: vi.fn(),
  }
})

function stubStorage(seed: Record<string, string> = {}) {
  const storage = new Map<string, string>(Object.entries(seed))
  vi.stubGlobal('localStorage', {
    getItem: vi.fn((key: string) => storage.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => storage.set(key, value)),
    removeItem: vi.fn((key: string) => storage.delete(key)),
    clear: vi.fn(() => storage.clear()),
  })
}

function stubWindow() {
  vi.stubGlobal('window', {
    setTimeout: (handler: () => void, ms?: number) => setTimeout(handler, ms),
    clearTimeout: (id: number) => clearTimeout(id),
  })
}

class MockAudio {
  src: string
  paused = true
  playCalls = 0
  onplay: (() => void) | null = null
  onended: (() => void) | null = null
  onerror: (() => void) | null = null

  constructor(src: string) {
    this.src = src
  }

  play() {
    this.playCalls += 1
    this.paused = false
    this.onplay?.()
    return Promise.resolve()
  }

  pause() {
    this.paused = true
  }
}

let previewAudios: MockAudio[] = []
let revokedUrls: string[] = []

function stubPreviewAudioGlobals() {
  previewAudios = []
  revokedUrls = []
  vi.stubGlobal('Audio', class extends MockAudio {
    constructor(src: string) {
      super(src)
      previewAudios.push(this)
    }
  })
  vi.stubGlobal('URL', {
    createObjectURL: vi.fn(() => 'blob:ms-preview'),
    revokeObjectURL: vi.fn((url: string) => {
      revokedUrls.push(url)
    }),
  })
}

describe('reader ms tts config', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    stubWindow()
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('defaults to the Yunjian voice, no style, 1.0 degree and 96k quality', () => {
    stubStorage()
    const appStore = useAppStore()
    appStore.setOnlineStatus(true)
    const store = useReaderStore()

    expect(store.speechConfig.msVoice).toBe(DEFAULT_MS_VOICE)
    expect(store.speechConfig.msVoice).toBe('zh-CN-YunjianNeural')
    expect(store.speechConfig.msDialogueVoice).toBe('')
    expect(store.speechConfig.msStyle).toBe('')
    expect(store.speechConfig.msDialogueStyle).toBe('')
    expect(store.speechConfig.msStyleDegree).toBe(1)
    expect(store.speechConfig.msPitch).toBe(0)
    expect(store.speechConfig.msVolume).toBe(0)
    expect(store.speechConfig.msFormat).toBe(DEFAULT_MS_FORMAT)
    expect(store.speechConfig.msFormat).toBe('96k')

    store.setSpeechProvider('msdemo')
    expect(store.speechProviderLabel).toBe('微软朗读')
    expect(store.msSpeechConfigured).toBe(true)
  })

  it('normalizes illegal saved ms values', () => {
    stubStorage({
      'reader-speechConfig': JSON.stringify({
        provider: 'msdemo',
        msVoice: 42,
        msStyle: null,
        msStyleDegree: 5,
        msPitch: 999,
        msVolume: -999,
        msFormat: 'wav',
      }),
    })
    const store = useReaderStore()

    expect(store.speechConfig.provider).toBe('msdemo')
    expect(store.speechConfig.msVoice).toBe(DEFAULT_MS_VOICE)
    expect(store.speechConfig.msStyle).toBe('')
    // 风格强度夹到 0.5–2.0（保留一位小数）；音调/音量按各自范围夹紧。
    expect(store.speechConfig.msStyleDegree).toBe(MS_STYLE_DEGREE_MAX)
    expect(store.speechConfig.msPitch).toBe(MS_PITCH_MAX)
    expect(store.speechConfig.msVolume).toBe(MS_VOLUME_MIN)
    // 只接受 48k/96k/192k，非法值回默认 96k。
    expect(store.speechConfig.msFormat).toBe(DEFAULT_MS_FORMAT)
  })

  it('keeps a saved 192k quality and clamps a too-small degree', () => {
    stubStorage({
      'reader-speechConfig': JSON.stringify({ msFormat: '192k', msStyleDegree: 0.1 }),
    })
    const store = useReaderStore()

    expect(store.speechConfig.msFormat).toBe('192k')
    expect(store.speechConfig.msStyleDegree).toBe(MS_STYLE_DEGREE_MIN)
  })

  it('persists and clamps the ms setters', () => {
    stubStorage()
    const store = useReaderStore()

    store.setMsVoice('zh-CN-XiaoxiaoNeural')
    store.setMsDialogueVoice('zh-CN-YunyangNeural')
    store.setMsStyle('newscast')
    store.setMsDialogueStyle('sports')
    store.setMsStyleDegree(1.55)
    store.setMsFormat('192k')

    expect(store.speechConfig.msVoice).toBe('zh-CN-XiaoxiaoNeural')
    expect(store.speechConfig.msStyle).toBe('newscast')
    expect(store.speechConfig.msDialogueStyle).toBe('sports')
    // 一位小数。
    expect(store.speechConfig.msStyleDegree).toBe(1.6)
    expect(store.speechConfig.msFormat).toBe('192k')
    expect(localStorage.setItem).toHaveBeenLastCalledWith(
      'reader-speechConfig',
      expect.stringContaining('"msFormat":"192k"'),
    )

    store.setMsStyleDegree(9)
    store.setMsPitch(-999)
    store.setMsVolume(999)
    store.setMsFormat('384k' as never)
    expect(store.speechConfig.msStyleDegree).toBe(MS_STYLE_DEGREE_MAX)
    expect(store.speechConfig.msPitch).toBe(MS_PITCH_MIN)
    expect(store.speechConfig.msVolume).toBe(MS_VOLUME_MAX)
    expect(store.speechConfig.msFormat).toBe(DEFAULT_MS_FORMAT)
  })
})

describe('reader ms tts synthesis', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    stubStorage()
    stubWindow()
    vi.mocked(synthesizeMsTts).mockReset()
    vi.mocked(synthesizeMsTts).mockResolvedValue(new Blob(['audio'], { type: 'audio/mpeg' }))
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  function prepareStore() {
    const appStore = useAppStore()
    appStore.setOnlineStatus(true)
    const store = useReaderStore()
    store.setSpeechProvider('msdemo')
    return store
  }

  it('sends voice, signed percents and the default 96k quality', async () => {
    const store = prepareStore()

    await store.synthesizeSpeechChunkBlob('旁白正文', 'narration')

    expect(vi.mocked(synthesizeMsTts).mock.calls[0][0]).toEqual({
      text: '旁白正文',
      voice: DEFAULT_MS_VOICE,
      style: undefined,
      styleDegree: undefined,
      rate: '+0%',
      pitch: '+0%',
      volume: '+0%',
      format: '96k',
    })
  })

  it('formats rate/pitch/volume and passes style/degree/quality', async () => {
    const store = prepareStore()
    store.setMsStyle('newscast')
    store.setMsStyleDegree(1.5)
    store.setMsPitch(-10)
    store.setMsVolume(20)
    store.setMsFormat('192k')
    store.setSpeechRate(1.4)

    await store.synthesizeSpeechChunkBlob('风格文本', 'narration')

    expect(vi.mocked(synthesizeMsTts).mock.calls[0][0]).toMatchObject({
      style: 'newscast',
      styleDegree: 1.5,
      rate: '+40%',
      pitch: '-10%',
      volume: '+20%',
      format: '192k',
    })
  })

  it('uses the dialogue voice/style only when dual voice is on, with fallbacks', async () => {
    const store = prepareStore()
    store.setMsDialogueVoice('zh-CN-XiaoxiaoNeural')
    store.setMsDialogueStyle('sports')

    // 双音色关闭：dialogue 块也按旁白处理。
    await store.synthesizeSpeechChunkBlob('“对白”', 'dialogue')
    expect(vi.mocked(synthesizeMsTts).mock.calls[0][0]).toMatchObject({ voice: DEFAULT_MS_VOICE })

    store.setDualVoiceEnabled(true)
    await store.synthesizeSpeechChunkBlob('“对白”', 'dialogue')
    expect(vi.mocked(synthesizeMsTts).mock.calls[1][0]).toMatchObject({
      voice: 'zh-CN-XiaoxiaoNeural',
      style: 'sports',
    })

    // 对话风格清空 → 跟随旁白风格（此时旁白也没风格 → undefined）。
    store.setMsStyle('newscast')
    store.setMsDialogueStyle('')
    await store.synthesizeSpeechChunkBlob('“对白”', 'dialogue')
    expect(vi.mocked(synthesizeMsTts).mock.calls[2][0]).toMatchObject({
      voice: 'zh-CN-XiaoxiaoNeural',
      style: 'newscast',
    })
  })

  it('rejects an already-aborted signal without calling the backend', async () => {
    const store = prepareStore()
    const controller = new AbortController()
    controller.abort()

    await expect(store.synthesizeSpeechChunkBlob('正文', 'narration', controller.signal))
      .rejects.toThrow()
    expect(synthesizeMsTts).not.toHaveBeenCalled()
  })
})

describe('reader ms tts preview', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    stubStorage()
    stubWindow()
    stubPreviewAudioGlobals()
    vi.mocked(synthesizeMsTts).mockReset()
    vi.mocked(synthesizeMsTts).mockResolvedValue(new Blob(['audio'], { type: 'audio/mpeg' }))
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('previews through ms voice/style and does not touch reading state', async () => {
    const appStore = useAppStore()
    appStore.setOnlineStatus(true)
    const store = useReaderStore()
    store.setSpeechProvider('msdemo')
    store.setMsStyle('newscast')
    store.setMsStyleDegree(1.5)

    await store.previewSpeech('试听文本')

    expect(store.isSpeechPreviewing).toBe(true)
    expect(store.isSpeechPreviewLoading).toBe(false)
    // 试听不污染书内朗读状态。
    expect(store.isSpeaking).toBe(false)
    expect(store.speechProgress).toBe(0)
    expect(vi.mocked(synthesizeMsTts).mock.calls[0][0]).toMatchObject({
      text: '试听文本',
      voice: DEFAULT_MS_VOICE,
      style: 'newscast',
      styleDegree: 1.5,
      format: '96k',
    })
    expect(previewAudios).toHaveLength(1)

    store.stopSpeechPreview()
    expect(store.isSpeechPreviewing).toBe(false)
    expect(previewAudios[0].paused).toBe(true)
    expect(revokedUrls).toContain('blob:ms-preview')
  })

  it('splits dual-voice preview text and plays each segment with its own voice', async () => {
    const appStore = useAppStore()
    appStore.setOnlineStatus(true)
    const store = useReaderStore()
    store.setSpeechProvider('msdemo')
    store.setDualVoiceEnabled(true)
    store.setMsDialogueVoice('zh-CN-XiaoxiaoNeural')

    // 旁白+对话+旁白：切分成 3 段后逐段串行合成/播放。
    await store.previewSpeech('他说：“你好。”然后走了。')

    await vi.waitFor(() => expect(vi.mocked(synthesizeMsTts)).toHaveBeenCalledTimes(1))
    expect(vi.mocked(synthesizeMsTts).mock.calls[0][0]).toMatchObject({ text: '他说：', voice: DEFAULT_MS_VOICE })
    await vi.waitFor(() => expect(previewAudios).toHaveLength(1))

    previewAudios[0].onended?.()
    await vi.waitFor(() => expect(vi.mocked(synthesizeMsTts)).toHaveBeenCalledTimes(2))
    expect(vi.mocked(synthesizeMsTts).mock.calls[1][0]).toMatchObject({
      text: '“你好。”',
      voice: 'zh-CN-XiaoxiaoNeural',
    })
    await vi.waitFor(() => expect(previewAudios).toHaveLength(2))

    previewAudios[1].onended?.()
    await vi.waitFor(() => expect(vi.mocked(synthesizeMsTts)).toHaveBeenCalledTimes(3))
    expect(vi.mocked(synthesizeMsTts).mock.calls[2][0]).toMatchObject({ text: '然后走了。', voice: DEFAULT_MS_VOICE })
    await vi.waitFor(() => expect(previewAudios).toHaveLength(3))

    previewAudios[2].onended?.()
    await vi.waitFor(() => expect(store.isSpeechPreviewing).toBe(false))
    // 每段播完都回收自己的 objectURL。
    expect(revokedUrls.length).toBeGreaterThanOrEqual(3)
    expect(store.isSpeaking).toBe(false)
    expect(store.speechProgress).toBe(0)
  })

  it('keeps a single synthesis when dual voice is off', async () => {
    const appStore = useAppStore()
    appStore.setOnlineStatus(true)
    const store = useReaderStore()
    store.setSpeechProvider('msdemo')

    await store.previewSpeech('他说：“你好。”然后走了。')

    await vi.waitFor(() => expect(vi.mocked(synthesizeMsTts)).toHaveBeenCalledTimes(1))
    // 单音色：整段文本一次合成，不切分。
    expect(vi.mocked(synthesizeMsTts).mock.calls[0][0]).toMatchObject({
      text: '他说：“你好。”然后走了。',
      voice: DEFAULT_MS_VOICE,
    })
  })

  it('previews a single dialogue-only text with the dialogue voice', async () => {
    const appStore = useAppStore()
    appStore.setOnlineStatus(true)
    const store = useReaderStore()
    store.setSpeechProvider('msdemo')
    store.setDualVoiceEnabled(true)
    store.setMsDialogueVoice('zh-CN-XiaoxiaoNeural')

    // 双音色 + 唯一片段是对白：仍要按对话音色合成，而不是退回旁白。
    await store.previewSpeech('“你好。”')

    await vi.waitFor(() => expect(vi.mocked(synthesizeMsTts)).toHaveBeenCalledTimes(1))
    expect(vi.mocked(synthesizeMsTts).mock.calls[0][0]).toMatchObject({
      text: '“你好。”',
      voice: 'zh-CN-XiaoxiaoNeural',
    })
  })

  it('drops a late dual-voice segment when preview is stopped mid-chain', async () => {
    const appStore = useAppStore()
    appStore.setOnlineStatus(true)
    const store = useReaderStore()
    store.setSpeechProvider('msdemo')
    store.setDualVoiceEnabled(true)
    store.setMsDialogueVoice('zh-CN-XiaoxiaoNeural')
    vi.mocked(synthesizeMsTts)
      .mockImplementationOnce(() => Promise.resolve(new Blob(['first'], { type: 'audio/mpeg' })))
      .mockImplementationOnce(() => new Promise<Blob>(() => { /* 第二段刻意挂起 */ }))

    await store.previewSpeech('他说：“你好。”')
    await vi.waitFor(() => expect(previewAudios).toHaveLength(1))

    // 第一段播完 → 触发第二段合成（挂起中），此时停止试听。
    previewAudios[0].onended?.()
    await vi.waitFor(() => expect(vi.mocked(synthesizeMsTts)).toHaveBeenCalledTimes(2))
    store.stopSpeechPreview()
    await new Promise((resolve) => setTimeout(resolve, 20))

    // 停止后不能再有第二段播放，状态也要复位。
    expect(previewAudios).toHaveLength(1)
    expect(store.isSpeechPreviewing).toBe(false)
    expect(store.isSpeechPreviewLoading).toBe(false)
  })
})
