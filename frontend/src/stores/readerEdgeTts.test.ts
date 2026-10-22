import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useAppStore } from './app'
import {
  DEFAULT_EDGE_FORMAT,
  DEFAULT_EDGE_VOICE,
  EDGE_PITCH_MAX,
  EDGE_PITCH_MIN,
  EDGE_VOLUME_MAX,
  EDGE_VOLUME_MIN,
  formatEdgePitch,
  formatEdgeRate,
  formatEdgeVolume,
  useReaderStore,
} from './reader'
import { synthesizeEdgeTts } from '../api/edgeTts'

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

/** 试听专用的假 audio：只记录播放/暂停与 src，能手动触发 onended。 */
class MockPreviewAudio {
  src: string
  paused = true
  playCalls = 0
  pauseCalls = 0
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
    this.pauseCalls += 1
    this.paused = true
  }
}

let previewAudios: MockPreviewAudio[] = []
let revokedUrls: string[] = []

function stubPreviewAudioGlobals() {
  previewAudios = []
  revokedUrls = []
  vi.stubGlobal('Audio', class extends MockPreviewAudio {
    constructor(src: string) {
      super(src)
      previewAudios.push(this)
    }
  })
  vi.stubGlobal('URL', {
    createObjectURL: vi.fn(() => 'blob:preview-audio'),
    revokeObjectURL: vi.fn((url: string) => {
      revokedUrls.push(url)
    }),
  })
}

interface SystemSpeechStub {
  speaking: boolean
  pending: boolean
  paused: boolean
  spoken: MockUtterance[]
  getVoices: () => Array<{ name: string; lang: string }>
  speak: (utterance: MockUtterance) => void
  cancel: () => void
  pause: () => void
  resume: () => void
}

class MockUtterance {
  text: string
  lang = ''
  rate = 1
  pitch = 1
  voice: { name: string; lang: string } | null = null
  onstart: (() => void) | null = null
  onend: (() => void) | null = null
  onerror: ((event: { error: string }) => void) | null = null

  constructor(text: string) {
    this.text = text
  }
}

function createSystemSpeechStub(voices: Array<{ name: string; lang: string }> = [{ name: 'Voice-A', lang: 'zh-CN' }]): SystemSpeechStub {
  const stub: SystemSpeechStub = {
    speaking: false,
    pending: false,
    paused: false,
    spoken: [],
    getVoices: () => voices,
    speak: (utterance) => {
      stub.spoken.push(utterance)
    },
    cancel: () => undefined,
    pause: () => undefined,
    resume: () => undefined,
  }
  return stub
}

function stubSystemSpeech(voices?: Array<{ name: string; lang: string }>) {
  const stub = createSystemSpeechStub(voices)
  vi.stubGlobal('window', {
    setTimeout: (handler: () => void, ms?: number) => setTimeout(handler, ms),
    clearTimeout: (id: number) => clearTimeout(id),
    speechSynthesis: stub,
  })
  vi.stubGlobal('SpeechSynthesisUtterance', MockUtterance)
  return stub
}

function flush() {
  return new Promise((resolve) => setTimeout(resolve, 0))
}

describe('reader edge tts config', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    stubWindow()
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('defaults to the Yunxi voice and reports the provider label', () => {
    stubStorage()
    const appStore = useAppStore()
    appStore.setOnlineStatus(true)
    const store = useReaderStore()

    expect(store.speechConfig.edgeVoice).toBe(DEFAULT_EDGE_VOICE)
    expect(store.speechConfig.edgeVoice).toBe('zh-CN-YunxiNeural')
    expect(store.speechConfig.edgeDialogueVoice).toBe('')
    // 音调/音量默认都不偏移（后端拿到 +0Hz / +0%）；音质默认标准 48k。
    expect(store.speechConfig.edgePitch).toBe(0)
    expect(store.speechConfig.edgeVolume).toBe(0)
    expect(store.speechConfig.edgeFormat).toBe(DEFAULT_EDGE_FORMAT)
    expect(store.speechConfig.edgeFormat).toBe('48k')
    store.setSpeechProvider('edge')
    expect(store.speechProviderLabel).toBe('Edge 朗读')
    expect(store.edgeSpeechConfigured).toBe(true)
  })

  it('requires a voice and an online connection to be configured', () => {
    stubStorage()
    const appStore = useAppStore()
    appStore.setOnlineStatus(true)
    const store = useReaderStore()

    expect(store.edgeSpeechConfigured).toBe(true)
    // Edge 是纯在线服务：离线时直接算未配置，避免点了朗读才报网络错。
    appStore.setOnlineStatus(false)
    expect(store.edgeSpeechConfigured).toBe(false)
  })

  it('normalizes illegal saved edge values', () => {
    stubStorage({
      'reader-speechConfig': JSON.stringify({
        provider: 'edge',
        edgeVoice: 42,
        edgeDialogueVoice: null,
        edgePitch: 500,
        edgeVolume: -999,
      }),
    })
    const store = useReaderStore()

    expect(store.speechConfig.provider).toBe('edge')
    expect(store.speechConfig.edgeVoice).toBe(DEFAULT_EDGE_VOICE)
    expect(store.speechConfig.edgeDialogueVoice).toBe('')
    // 越界夹回 [-100, 100]。
    expect(store.speechConfig.edgePitch).toBe(EDGE_PITCH_MAX)
    expect(store.speechConfig.edgeVolume).toBe(EDGE_VOLUME_MIN)
  })

  it('falls back to zero for non-numeric pitch/volume in storage', () => {
    stubStorage({
      'reader-speechConfig': JSON.stringify({
        edgePitch: 'high',
        edgeVolume: Number.NaN,
        edgeFormat: 'wav',
      }),
    })
    const store = useReaderStore()

    expect(store.speechConfig.edgePitch).toBe(0)
    expect(store.speechConfig.edgeVolume).toBe(0)
    // 只接受 48k / 96k，其它（含历史脏值）回默认。
    expect(store.speechConfig.edgeFormat).toBe('48k')
  })

  it('keeps a saved 96k quality and normalizes unknown formats', () => {
    stubStorage({ 'reader-speechConfig': JSON.stringify({ edgeFormat: '96k' }) })
    const high = useReaderStore()
    expect(high.speechConfig.edgeFormat).toBe('96k')

    setActivePinia(createPinia())
    stubStorage({ 'reader-speechConfig': JSON.stringify({ edgeFormat: 96 }) })
    const invalid = useReaderStore()
    expect(invalid.speechConfig.edgeFormat).toBe(DEFAULT_EDGE_FORMAT)
  })

  it('keeps a saved dialogue voice and trims blank narration voices to the default', () => {
    stubStorage({
      'reader-speechConfig': JSON.stringify({
        edgeVoice: '   ',
        edgeDialogueVoice: 'zh-CN-XiaoxiaoNeural',
      }),
    })
    const store = useReaderStore()

    expect(store.speechConfig.edgeVoice).toBe(DEFAULT_EDGE_VOICE)
    expect(store.speechConfig.edgeDialogueVoice).toBe('zh-CN-XiaoxiaoNeural')
  })

  it('persists the edge voice setters', () => {
    stubStorage()
    const store = useReaderStore()

    store.setEdgeVoice('zh-CN-XiaoxiaoNeural')
    store.setEdgeDialogueVoice('zh-CN-YunyangNeural')

    expect(store.speechConfig.edgeVoice).toBe('zh-CN-XiaoxiaoNeural')
    expect(store.speechConfig.edgeDialogueVoice).toBe('zh-CN-YunyangNeural')
    expect(localStorage.setItem).toHaveBeenLastCalledWith(
      'reader-speechConfig',
      expect.stringContaining('"edgeDialogueVoice":"zh-CN-YunyangNeural"'),
    )
  })

  it('persists the edge quality and falls back on an unknown value', () => {
    stubStorage()
    const store = useReaderStore()

    store.setEdgeFormat('96k')
    expect(store.speechConfig.edgeFormat).toBe('96k')
    expect(localStorage.setItem).toHaveBeenLastCalledWith(
      'reader-speechConfig',
      expect.stringContaining('"edgeFormat":"96k"'),
    )

    // UI 之外的脏值：非 48k/96k 一律回默认，不能把非法 outputFormat 发给后端。
    store.setEdgeFormat('192k' as never)
    expect(store.speechConfig.edgeFormat).toBe(DEFAULT_EDGE_FORMAT)
  })

  it('persists and clamps the edge pitch/volume setters', () => {
    stubStorage()
    const store = useReaderStore()

    store.setEdgePitch(-20)
    store.setEdgeVolume(30)
    expect(store.speechConfig.edgePitch).toBe(-20)
    expect(store.speechConfig.edgeVolume).toBe(30)
    expect(localStorage.setItem).toHaveBeenLastCalledWith(
      'reader-speechConfig',
      expect.stringContaining('"edgeVolume":30'),
    )

    store.setEdgePitch(999)
    store.setEdgeVolume(-999)
    expect(store.speechConfig.edgePitch).toBe(EDGE_PITCH_MAX)
    expect(store.speechConfig.edgeVolume).toBe(EDGE_VOLUME_MIN)

    store.setEdgePitch(-999)
    store.setEdgeVolume(999)
    expect(store.speechConfig.edgePitch).toBe(EDGE_PITCH_MIN)
    expect(store.speechConfig.edgeVolume).toBe(EDGE_VOLUME_MAX)
  })
})

describe('reader edge tts synthesis', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    stubStorage()
    stubWindow()
    vi.mocked(synthesizeEdgeTts).mockReset()
    vi.mocked(synthesizeEdgeTts).mockResolvedValue(new Blob(['audio'], { type: 'audio/mpeg' }))
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  function prepareStore() {
    const appStore = useAppStore()
    appStore.setOnlineStatus(true)
    const store = useReaderStore()
    store.setSpeechProvider('edge')
    return store
  }

  it('synthesizes with the narration voice and zero pitch/volume offsets', async () => {
    const store = prepareStore()

    await store.synthesizeSpeechChunkBlob('旁白正文', 'narration')

    expect(synthesizeEdgeTts).toHaveBeenCalledTimes(1)
    expect(vi.mocked(synthesizeEdgeTts).mock.calls[0][0]).toEqual({
      text: '旁白正文',
      voice: DEFAULT_EDGE_VOICE,
      rate: '+0%',
      pitch: '+0Hz',
      volume: '+0%',
      format: '48k',
    })
  })

  it('passes the configured 96k quality to the API', async () => {
    const store = prepareStore()
    store.setEdgeFormat('96k')

    await store.synthesizeSpeechChunkBlob('高音质', 'narration')

    expect(vi.mocked(synthesizeEdgeTts).mock.calls[0][0]).toMatchObject({ format: '96k' })
  })

  it('formats the configured pitch/volume offsets for the request', async () => {
    const store = prepareStore()
    store.setEdgePitch(-20)
    store.setEdgeVolume(-30)

    await store.synthesizeSpeechChunkBlob('低音小声', 'narration')
    expect(vi.mocked(synthesizeEdgeTts).mock.calls[0][0]).toMatchObject({
      pitch: '-20Hz',
      volume: '-30%',
      rate: '+0%',
    })

    store.setEdgePitch(15)
    store.setEdgeVolume(20)
    await store.synthesizeSpeechChunkBlob('高音大声', 'narration')
    expect(vi.mocked(synthesizeEdgeTts).mock.calls[1][0]).toMatchObject({
      pitch: '+15Hz',
      volume: '+20%',
    })
  })

  it('maps speechRate to Edge rate percentages', async () => {
    const store = prepareStore()

    store.setSpeechRate(0.8)
    await store.synthesizeSpeechChunkBlob('慢速', 'narration')
    expect(vi.mocked(synthesizeEdgeTts).mock.calls[0][0].rate).toBe('-20%')

    store.setSpeechRate(1.15)
    await store.synthesizeSpeechChunkBlob('快速', 'narration')
    expect(vi.mocked(synthesizeEdgeTts).mock.calls[1][0].rate).toBe('+15%')

    store.setSpeechRate(1)
    await store.synthesizeSpeechChunkBlob('原速', 'narration')
    expect(vi.mocked(synthesizeEdgeTts).mock.calls[2][0].rate).toBe('+0%')
  })

  it('uses the dialogue voice for dialogue chunks only when dual voice is on', async () => {
    const store = prepareStore()
    store.setEdgeDialogueVoice('zh-CN-XiaoxiaoNeural')

    // 双音色关闭：调用方不会产生 dialogue 块，即便传了也按旁白处理。
    await store.synthesizeSpeechChunkBlob('“对白”', 'dialogue')
    expect(vi.mocked(synthesizeEdgeTts).mock.calls[0][0].voice).toBe(DEFAULT_EDGE_VOICE)

    store.setDualVoiceEnabled(true)
    await store.synthesizeSpeechChunkBlob('“对白”', 'dialogue')
    expect(vi.mocked(synthesizeEdgeTts).mock.calls[1][0].voice).toBe('zh-CN-XiaoxiaoNeural')

    // 旁白块仍然用旁白音色
    await store.synthesizeSpeechChunkBlob('旁白', 'narration')
    expect(vi.mocked(synthesizeEdgeTts).mock.calls[2][0].voice).toBe(DEFAULT_EDGE_VOICE)
  })

  it('falls back to the narration voice when the dialogue voice is blank', async () => {
    const store = prepareStore()
    store.setDualVoiceEnabled(true)
    store.setEdgeDialogueVoice('   ')

    await store.synthesizeSpeechChunkBlob('“对白”', 'dialogue')

    expect(vi.mocked(synthesizeEdgeTts).mock.calls[0][0].voice).toBe(DEFAULT_EDGE_VOICE)
  })

  it('rejects an already-aborted signal without calling the backend', async () => {
    const store = prepareStore()
    const controller = new AbortController()
    controller.abort()

    await expect(store.synthesizeSpeechChunkBlob('正文', 'narration', controller.signal))
      .rejects.toThrow()
    expect(synthesizeEdgeTts).not.toHaveBeenCalled()
  })

  it('formats edge rates for the whole supported speed range', () => {
    expect(formatEdgeRate(1)).toBe('+0%')
    expect(formatEdgeRate(0.8)).toBe('-20%')
    expect(formatEdgeRate(1.15)).toBe('+15%')
    expect(formatEdgeRate(0.5)).toBe('-50%')
    expect(formatEdgeRate(2)).toBe('+100%')
    // 非法倍速回默认（+0%），而不是拼出 "NaN%"。
    expect(formatEdgeRate(Number.NaN)).toBe('+0%')
  })

  it('formats edge pitch and volume with explicit signs', () => {
    expect(formatEdgePitch(0)).toBe('+0Hz')
    expect(formatEdgePitch(-20)).toBe('-20Hz')
    expect(formatEdgePitch(15)).toBe('+15Hz')
    expect(formatEdgePitch(Number.NaN)).toBe('+0Hz')

    expect(formatEdgeVolume(0)).toBe('+0%')
    expect(formatEdgeVolume(-30)).toBe('-30%')
    expect(formatEdgeVolume(20)).toBe('+20%')
    expect(formatEdgeVolume(Number.NaN)).toBe('+0%')
  })
})

describe('reader speech preview', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    stubStorage()
    stubWindow()
    stubPreviewAudioGlobals()
    vi.mocked(synthesizeEdgeTts).mockReset()
    vi.mocked(synthesizeEdgeTts).mockResolvedValue(new Blob(['audio'], { type: 'audio/mpeg' }))
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  function prepareEdgeStore() {
    const appStore = useAppStore()
    appStore.setOnlineStatus(true)
    const store = useReaderStore()
    store.setSpeechProvider('edge')
    store.setEdgeVoice('zh-CN-XiaoxiaoNeural')
    return store
  }

  it('previews through the current voice/params without touching reading state', async () => {
    const store = prepareEdgeStore()
    store.setEdgePitch(-20)
    store.setEdgeVolume(-30)
    const appStore = useAppStore()

    await store.previewSpeech('试听文本')

    expect(store.isSpeechPreviewing).toBe(true)
    expect(store.isSpeechPreviewLoading).toBe(false)
    // 试听绝不污染书内朗读状态。
    expect(store.isSpeaking).toBe(false)
    expect(store.isPaused).toBe(false)
    expect(store.speechProgress).toBe(0)
    expect(appStore.toasts).toHaveLength(0)
    expect(vi.mocked(synthesizeEdgeTts).mock.calls[0][0]).toMatchObject({
      text: '试听文本',
      voice: 'zh-CN-XiaoxiaoNeural',
      pitch: '-20Hz',
      volume: '-30%',
    })
    expect(previewAudios).toHaveLength(1)
    expect(previewAudios[0].playCalls).toBe(1)

    previewAudios[0].onended?.()
    expect(store.isSpeechPreviewing).toBe(false)
    expect(revokedUrls).toContain('blob:preview-audio')
  })

  it('stops preview playback and releases the object URL', async () => {
    const store = prepareEdgeStore()
    await store.previewSpeech('试听文本')
    expect(store.isSpeechPreviewing).toBe(true)

    store.stopSpeechPreview()

    expect(store.isSpeechPreviewing).toBe(false)
    expect(previewAudios[0].paused).toBe(true)
    expect(revokedUrls).toContain('blob:preview-audio')
    // 停止后迟到的 onended 不能改状态。
    previewAudios[0].onended?.()
    expect(store.isSpeechPreviewing).toBe(false)
  })

  it('drops a preview whose synthesis finishes after the user stopped', async () => {
    const store = prepareEdgeStore()
    let resolveSynth: ((blob: Blob) => void) | undefined
    vi.mocked(synthesizeEdgeTts).mockImplementation(() => new Promise<Blob>((resolve) => {
      resolveSynth = resolve
    }))

    const task = store.previewSpeech('试听文本')
    await vi.waitFor(() => expect(store.isSpeechPreviewLoading).toBe(true))

    store.stopSpeechPreview()
    resolveSynth?.(new Blob(['audio']))
    await task
    await flush()

    expect(store.isSpeechPreviewing).toBe(false)
    expect(store.isSpeechPreviewLoading).toBe(false)
    expect(previewAudios).toHaveLength(0)
  })

  it('reports synthesis failures as a readable toast', async () => {
    const store = prepareEdgeStore()
    const appStore = useAppStore()
    vi.mocked(synthesizeEdgeTts).mockRejectedValue(new Error('Edge 朗读服务暂时不可用'))

    await store.previewSpeech('试听文本')

    expect(store.isSpeechPreviewing).toBe(false)
    expect(store.isSpeechPreviewLoading).toBe(false)
    expect(appStore.toasts.at(-1)?.message).toContain('Edge 朗读服务暂时不可用')
  })

  it('speaks dual-voice system preview segment by segment with matching voices', async () => {
    const synthStub = stubSystemSpeech([
      { name: 'Voice-A', lang: 'zh-CN' },
      { name: 'Voice-B', lang: 'zh-CN' },
    ])
    const store = useReaderStore()
    store.setSpeechProvider('system')
    store.setVoiceName('Voice-A')
    store.setDialogueVoiceName('Voice-B')
    store.setDualVoiceEnabled(true)

    await store.previewSpeech('他说：“你好。”')

    // 一次只 speak 一段：第一段播完（onend）才链式 speak 第二段。
    await vi.waitFor(() => expect(synthStub.spoken).toHaveLength(1))
    expect(synthStub.spoken[0].text).toBe('他说：')
    expect(synthStub.spoken[0].voice?.name).toBe('Voice-A')

    synthStub.spoken[0].onstart?.()
    expect(store.isSpeechPreviewing).toBe(true)
    synthStub.spoken[0].onend?.()

    await vi.waitFor(() => expect(synthStub.spoken).toHaveLength(2))
    expect(synthStub.spoken[1].text).toBe('“你好。”')
    expect(synthStub.spoken[1].voice?.name).toBe('Voice-B')

    synthStub.spoken[1].onstart?.()
    synthStub.spoken[1].onend?.()
    expect(store.isSpeechPreviewing).toBe(false)
    expect(store.isSpeaking).toBe(false)
  })

  it('uses the dialogue voice for a single dialogue-only system preview', async () => {
    const synthStub = stubSystemSpeech([
      { name: 'Voice-A', lang: 'zh-CN' },
      { name: 'Voice-B', lang: 'zh-CN' },
    ])
    const store = useReaderStore()
    store.setSpeechProvider('system')
    store.setVoiceName('Voice-A')
    store.setDialogueVoiceName('Voice-B')
    store.setDualVoiceEnabled(true)

    await store.previewSpeech('“你好。”')

    await vi.waitFor(() => expect(synthStub.spoken).toHaveLength(1))
    expect(synthStub.spoken[0].text).toBe('“你好。”')
    expect(synthStub.spoken[0].voice?.name).toBe('Voice-B')

    synthStub.spoken[0].onstart?.()
    synthStub.spoken[0].onend?.()
    expect(store.isSpeechPreviewing).toBe(false)
  })

  it('uses speechSynthesis for the system channel', async () => {
    const synthStub = stubSystemSpeech()
    const store = useReaderStore()
    store.setSpeechProvider('system')
    store.setVoiceName('Voice-A')

    await store.previewSpeech('系统试听文本')

    expect(synthStub.spoken).toHaveLength(1)
    expect(synthStub.spoken[0].text).toBe('系统试听文本')
    expect(synthStub.spoken[0].voice?.name).toBe('Voice-A')
    expect(store.isSpeechPreviewLoading).toBe(true)
    expect(store.isSpeaking).toBe(false)

    synthStub.spoken[0].onstart?.()
    expect(store.isSpeechPreviewing).toBe(true)
    expect(store.isSpeechPreviewLoading).toBe(false)

    synthStub.spoken[0].onend?.()
    expect(store.isSpeechPreviewing).toBe(false)
    expect(store.isSpeaking).toBe(false)
  })
})
