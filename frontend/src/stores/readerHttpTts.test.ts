import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useAppStore } from './app'
import { useReaderStore } from './reader'
import { getBookContent, getBookInfo, getChapterList, saveBookProgress } from '../api/bookshelf'
import { syncLegadoBookProgress, uploadLegadoBookProgress } from '../api/webdav'
import { requestOpenAISpeechAudio } from '../utils/openaiSpeech'
import { synthesizeHttpTts } from '../api/httpTts'

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

// 只 mock 会发 IPC 的函数；sniff/normalize 这些纯函数保留真实实现，
// 否则「配置归一化」的测试测的就不是产品代码了。
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

/** 只实现 store 用到的那几个属性的 HTMLAudioElement 替身。 */
class MockAudio {
  src: string
  paused = true
  ended = false
  duration = 10
  currentTime = 0
  onplay: (() => void) | null = null
  onpause: (() => void) | null = null
  onloadedmetadata: (() => void) | null = null
  ontimeupdate: (() => void) | null = null
  onended: (() => void) | null = null
  onerror: (() => void) | null = null

  constructor(src: string) {
    this.src = src
  }

  async play() {
    this.paused = false
    this.onplay?.()
    return undefined
  }

  pause() {
    this.paused = true
    this.onpause?.()
  }
}

let audioInstances: MockAudio[] = []
const revokeObjectURL = vi.fn()

function stubStorage(seed: Record<string, string> = {}) {
  const storage = new Map<string, string>(Object.entries(seed))
  vi.stubGlobal('localStorage', {
    getItem: vi.fn((key: string) => storage.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => storage.set(key, value)),
    removeItem: vi.fn((key: string) => storage.delete(key)),
    clear: vi.fn(() => storage.clear()),
  })
}

function stubBrowserGlobals() {
  audioInstances = []
  revokeObjectURL.mockReset()
  vi.stubGlobal('Audio', class extends MockAudio {
    constructor(src: string) {
      super(src)
      audioInstances.push(this)
    }
  })
  vi.stubGlobal('URL', {
    createObjectURL: vi.fn(() => 'blob:http-tts-audio'),
    revokeObjectURL,
  })
  // 加载动画/定时器都走 window.setTimeout；这里只需要 setTimeout/clearTimeout 两个成员。
  vi.stubGlobal('window', {
    setTimeout: (handler: () => void, ms?: number) => setTimeout(handler, ms),
    clearTimeout: (id: number) => clearTimeout(id),
  })
}

describe('reader http tts config', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    stubBrowserGlobals()
    vi.mocked(synthesizeHttpTts).mockReset()
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('keeps old two-provider configs working (no httpTtsId key)', () => {
    stubStorage({
      'reader-speechConfig': JSON.stringify({
        provider: 'openai',
        openaiBaseUrl: 'https://api.example.test/v1',
        openaiVoice: 'vivian',
      }),
    })

    const store = useReaderStore()

    expect(store.speechConfig.provider).toBe('openai')
    expect(store.speechConfig.httpTtsId).toBeNull()
    expect(store.speechConfig.httpTtsContentType).toBe('')
    expect(store.httpSpeechConfigured).toBe(false)
  })

  it('normalizes illegal values instead of trusting whatever is in localStorage', () => {
    stubStorage({
      'reader-speechConfig': JSON.stringify({
        provider: 'not-a-provider',
        httpTtsId: { nested: true },
        httpTtsContentType: 'text/html',
      }),
    })

    const store = useReaderStore()

    // 对象展开只补缺失 key，补不住「key 存在但值非法」：老配置/手改配置必须落回安全值。
    expect(store.speechConfig.provider).toBe('system')
    expect(store.speechConfig.httpTtsId).toBeNull()
    expect(store.speechConfig.httpTtsContentType).toBe('')
  })

  it('restores a valid http provider selection', () => {
    stubStorage({
      'reader-speechConfig': JSON.stringify({
        provider: 'http',
        httpTtsId: 42,
        httpTtsContentType: 'audio/wav',
      }),
    })

    const store = useReaderStore()

    expect(store.speechConfig.provider).toBe('http')
    expect(store.speechConfig.httpTtsId).toBe(42)
    expect(store.speechConfig.httpTtsContentType).toBe('audio/wav')
    expect(store.httpSpeechConfigured).toBe(true)
    expect(store.speechProviderLabel).toBe('在线朗读引擎')
  })

  it('persists the selected engine id and contentType', () => {
    stubStorage()
    const store = useReaderStore()

    store.setHttpTtsId(101, 'audio/wav')
    expect(store.speechConfig.httpTtsId).toBe(101)
    expect(store.speechConfig.httpTtsContentType).toBe('audio/wav')
    expect(localStorage.setItem).toHaveBeenLastCalledWith(
      'reader-speechConfig',
      expect.stringContaining('"httpTtsId":101'),
    )

    store.setHttpTtsId(null)
    expect(store.speechConfig.httpTtsId).toBeNull()
    // 引擎清空后 MIME 兜底值也必须清掉，否则会用上一个引擎的类型去解释新音频。
    expect(store.speechConfig.httpTtsContentType).toBe('')
  })
})

describe('reader http tts playback', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    stubStorage()
    stubBrowserGlobals()
    vi.mocked(getBookContent).mockReset()
    vi.mocked(getBookInfo).mockReset()
    vi.mocked(getChapterList).mockReset()
    vi.mocked(saveBookProgress).mockReset()
    vi.mocked(syncLegadoBookProgress).mockReset()
    vi.mocked(uploadLegadoBookProgress).mockReset()
    vi.mocked(requestOpenAISpeechAudio).mockReset()
    vi.mocked(synthesizeHttpTts).mockReset()
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  function prepareStore() {
    const store = useReaderStore()
    store.setSpeechProvider('http')
    store.setHttpTtsId(7, 'audio/wav')
    return store
  }

  it('synthesizes the whole chapter once and plays the returned blob', async () => {
    const store = prepareStore()
    const onStart = vi.fn()
    const onEnd = vi.fn()
    vi.mocked(synthesizeHttpTts).mockResolvedValue(new Blob(['audio'], { type: 'audio/wav' }))

    store.setSpeechRate(1.5)
    store.startTTS('第一句。第二句。第三句。', { onStart, onEnd })

    await vi.waitFor(() => expect(store.isSpeaking).toBe(true))
    // 整段一次请求：前端切句会让 onEnd 多次触发，打断自动朗读的章节推进。
    expect(synthesizeHttpTts).toHaveBeenCalledTimes(1)
    expect(synthesizeHttpTts).toHaveBeenCalledWith({
      ttsId: 7,
      text: '第一句。第二句。第三句。',
      speed: 1.5,
      contentType: 'audio/wav',
    })
    expect(onStart).toHaveBeenCalledTimes(1)

    const audio = audioInstances[0]
    audio.currentTime = 3
    audio.ontimeupdate?.()
    expect(store.speechProgress).toBeCloseTo(0.3)

    audio.onended?.()
    expect(onEnd).toHaveBeenCalledTimes(1)
    expect(store.isSpeaking).toBe(false)
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:http-tts-audio')
  })

  it('pauses and resumes through the same HTMLAudioElement path as openai', async () => {
    const store = prepareStore()
    vi.mocked(synthesizeHttpTts).mockResolvedValue(new Blob(['audio']))
    store.startTTS('正文')
    await vi.waitFor(() => expect(store.isSpeaking).toBe(true))
    const audio = audioInstances[0]

    store.pauseTTS()
    expect(audio.paused).toBe(true)
    expect(store.isPaused).toBe(true)

    store.pauseTTS()
    await vi.waitFor(() => expect(store.isPaused).toBe(false))
    expect(audio.paused).toBe(false)
    expect(store.isSpeaking).toBe(true)
  })

  it('interrupts the previous paragraph audio when auto reading advances', async () => {
    const store = prepareStore()
    vi.mocked(synthesizeHttpTts).mockResolvedValue(new Blob(['audio']))

    store.startTTS('第一段正文')
    await vi.waitFor(() => expect(store.isSpeaking).toBe(true))
    const firstAudio = audioInstances[0]

    // 自动朗读以段落推进：上一段的 audio 必须先被停掉，否则两段会叠着播。
    store.startTTS('第二段正文')

    await vi.waitFor(() => expect(audioInstances).toHaveLength(2))
    expect(firstAudio.paused).toBe(true)
    expect(revokeObjectURL).toHaveBeenCalled()
    expect(store.isSpeaking).toBe(true)
  })

  it('stops playback and releases the object URL', async () => {    const store = prepareStore()
    vi.mocked(synthesizeHttpTts).mockResolvedValue(new Blob(['audio']))
    store.startTTS('正文')
    await vi.waitFor(() => expect(store.isSpeaking).toBe(true))

    store.stopTTS()

    expect(store.isSpeaking).toBe(false)
    expect(store.isPaused).toBe(false)
    expect(store.speechProgress).toBe(0)
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:http-tts-audio')
  })

  it('reports a readable error instead of failing silently when no engine is selected', () => {
    const store = useReaderStore()
    const appStore = useAppStore()
    store.setSpeechProvider('http')
    const onError = vi.fn()

    store.startTTS('正文', { onError })

    expect(onError).toHaveBeenCalledWith(expect.any(Error))
    expect(appStore.toasts.at(-1)?.message).toContain('在线朗读引擎')
    expect(synthesizeHttpTts).not.toHaveBeenCalled()
  })

  it('surfaces backend synthesis failures as a toast and onError', async () => {
    const store = prepareStore()
    const appStore = useAppStore()
    const onError = vi.fn()
    vi.mocked(synthesizeHttpTts).mockRejectedValue(new Error('引擎请求失败：接口返回了 HTML 错误页'))

    store.startTTS('正文', { onError })

    await vi.waitFor(() => expect(onError).toHaveBeenCalledTimes(1))
    expect((onError.mock.calls[0][0] as Error).message).toContain('接口返回了 HTML 错误页')
    expect(appStore.toasts.at(-1)?.message).toContain('接口返回了 HTML 错误页')
    expect(store.isSpeechLoading).toBe(false)
    expect(store.isSpeaking).toBe(false)
  })

  it('drops a late response that arrives after the user stopped', async () => {
    const store = prepareStore()
    let resolveSynth: ((blob: Blob) => void) | undefined
    vi.mocked(synthesizeHttpTts).mockImplementation(() => new Promise<Blob>((resolve) => {
      resolveSynth = resolve
    }))
    const onStart = vi.fn()

    store.startTTS('正文', { onStart })
    await vi.waitFor(() => expect(store.isSpeechLoading).toBe(true))

    store.stopTTS()
    resolveSynth?.(new Blob(['audio']))
    await vi.waitFor(() => expect(store.isSpeechLoading).toBe(false))

    // session 校验必须拦住迟到的音频：stop 之后不能再突然开口。
    expect(audioInstances).toHaveLength(0)
    expect(onStart).not.toHaveBeenCalled()
    expect(store.isSpeaking).toBe(false)
  })

  it('labels the third provider for the TTS panel', () => {
    const store = useReaderStore()

    expect(store.speechProviderLabel).toBe('系统语音')
    store.setSpeechProvider('openai')
    expect(store.speechProviderLabel).toBe('API 语音')
    store.setSpeechProvider('http')
    expect(store.speechProviderLabel).toBe('在线朗读引擎')
  })
})
