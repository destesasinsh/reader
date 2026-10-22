import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { BUFFER_CHARS_MAX, BUFFER_CHARS_MIN, DEFAULT_BUFFER_CHARS, useReaderStore } from './reader'
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

// 纯函数（normalize/sniff）保留真实实现，只替换会发 IPC 的合成函数。
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

/** 只实现 store 用到的那部分 HTMLAudioElement。 */
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

class MockUtterance {
  text: string
  lang = ''
  rate = 1
  pitch = 1
  voice: { name: string; lang: string } | null = null
  onstart: (() => void) | null = null
  onend: (() => void) | null = null
  onerror: ((event: { error: string }) => void) | null = null
  onboundary: ((event: { charIndex: number }) => void) | null = null

  constructor(text: string) {
    this.text = text
  }
}

interface SynthStub {
  speaking: boolean
  pending: boolean
  paused: boolean
  voices: Array<{ name: string; lang: string }>
  spoken: MockUtterance[]
  getVoices: () => Array<{ name: string; lang: string }>
  speak: (utterance: MockUtterance) => void
  cancel: () => void
  pause: () => void
  resume: () => void
}

function createSynthStub(): SynthStub {
  const stub: SynthStub = {
    speaking: false,
    pending: false,
    paused: false,
    voices: [],
    spoken: [],
    getVoices: () => stub.voices,
    speak: (utterance) => {
      stub.spoken.push(utterance)
    },
    cancel: () => undefined,
    pause: () => undefined,
    resume: () => undefined,
  }
  return stub
}

const synthStub = createSynthStub()

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

function stubAudioGlobals(withSpeechSynth = false) {
  audioInstances = []
  revokeObjectURL.mockReset()
  vi.stubGlobal('Audio', class extends MockAudio {
    constructor(src: string) {
      super(src)
      audioInstances.push(this)
    }
  })
  vi.stubGlobal('URL', {
    createObjectURL: vi.fn(() => 'blob:dual-voice-audio'),
    revokeObjectURL,
  })
  vi.stubGlobal('window', {
    setTimeout: (handler: () => void, ms?: number) => setTimeout(handler, ms),
    clearTimeout: (id: number) => clearTimeout(id),
    ...(withSpeechSynth ? { speechSynthesis: synthStub } : {}),
  })
  vi.stubGlobal('SpeechSynthesisUtterance', MockUtterance)
}

function resetStoreMocks() {
  vi.mocked(getBookContent).mockReset()
  vi.mocked(getBookInfo).mockReset()
  vi.mocked(getChapterList).mockReset()
  vi.mocked(saveBookProgress).mockReset()
  vi.mocked(syncLegadoBookProgress).mockReset()
  vi.mocked(uploadLegadoBookProgress).mockReset()
  vi.mocked(requestOpenAISpeechAudio).mockReset()
  vi.mocked(synthesizeHttpTts).mockReset()
}

describe('reader speech buffer config', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    stubAudioGlobals()
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('defaults dual voice to off and buffer to 500 characters', () => {
    stubStorage()
    const store = useReaderStore()

    expect(store.speechConfig.dualVoiceEnabled).toBe(false)
    expect(store.speechConfig.dialogueVoiceName).toBe('')
    expect(store.speechConfig.dialogueOpenaiVoice).toBe('')
    expect(store.speechConfig.dialogueHttpTtsId).toBeNull()
    expect(store.speechConfig.bufferChars).toBe(DEFAULT_BUFFER_CHARS)
    expect(DEFAULT_BUFFER_CHARS).toBe(500)
  })

  it('normalizes illegal dual-voice and buffer values from localStorage', () => {
    stubStorage({
      'reader-speechConfig': JSON.stringify({
        dualVoiceEnabled: 'yes',
        dialogueVoiceName: 42,
        dialogueHttpTtsId: { bad: true },
        dialogueHttpTtsContentType: 'text/html',
        bufferChars: -10,
      }),
    })
    const store = useReaderStore()

    // 布尔只认 true；字符串 "yes" 不能把功能打开。
    expect(store.speechConfig.dualVoiceEnabled).toBe(false)
    expect(store.speechConfig.dialogueVoiceName).toBe('')
    expect(store.speechConfig.dialogueHttpTtsId).toBeNull()
    expect(store.speechConfig.dialogueHttpTtsContentType).toBe('')
    // 缓冲字符数夹到 [200, 2000]。
    expect(store.speechConfig.bufferChars).toBe(BUFFER_CHARS_MIN)
  })

  it('clamps an oversized buffer and falls back on a non-numeric one', () => {
    stubStorage({
      'reader-speechConfig': JSON.stringify({ bufferChars: 99999, dualVoiceEnabled: true }),
    })
    const first = useReaderStore()
    expect(first.speechConfig.bufferChars).toBe(BUFFER_CHARS_MAX)
    expect(first.speechConfig.dualVoiceEnabled).toBe(true)

    setActivePinia(createPinia())
    stubStorage({ 'reader-speechConfig': JSON.stringify({ bufferChars: 'abc' }) })
    const second = useReaderStore()
    expect(second.speechConfig.bufferChars).toBe(DEFAULT_BUFFER_CHARS)
  })

  it('persists setters and keeps the dialogue choices after turning dual voice off', () => {
    stubStorage()
    const store = useReaderStore()

    store.setDialogueVoiceName('Voice-B')
    store.setDialogueOpenaiVoice('nova')
    store.setDialogueHttpTtsId(8, 'audio/wav')
    store.setBufferChars(800)
    store.setDualVoiceEnabled(true)
    store.setDualVoiceEnabled(false)

    // 关闭开关只是停用，不能清空已选值 —— 否则下次打开又要重选一遍。
    expect(store.speechConfig.dualVoiceEnabled).toBe(false)
    expect(store.speechConfig.dialogueVoiceName).toBe('Voice-B')
    expect(store.speechConfig.dialogueOpenaiVoice).toBe('nova')
    expect(store.speechConfig.dialogueHttpTtsId).toBe(8)
    expect(store.speechConfig.dialogueHttpTtsContentType).toBe('audio/wav')
    expect(store.speechConfig.bufferChars).toBe(800)
    expect(localStorage.setItem).toHaveBeenLastCalledWith(
      'reader-speechConfig',
      expect.stringContaining('"dialogueHttpTtsId":8'),
    )

    store.setBufferChars(100000)
    expect(store.speechConfig.bufferChars).toBe(BUFFER_CHARS_MAX)
  })
})

describe('reader speech chunk synthesis', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    stubStorage()
    stubAudioGlobals()
    resetStoreMocks()
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('uses the narration/dialogue engine for each http chunk', async () => {
    const store = useReaderStore()
    store.setSpeechProvider('http')
    store.setHttpTtsId(7, 'audio/wav')
    store.setDialogueHttpTtsId(8, 'audio/wav')
    store.setDualVoiceEnabled(true)
    store.setSpeechRate(1.5)
    vi.mocked(synthesizeHttpTts).mockResolvedValue(new Blob(['audio']))

    await store.synthesizeSpeechChunkBlob('旁白文本', 'narration')
    expect(vi.mocked(synthesizeHttpTts).mock.calls[0][0]).toMatchObject({
      ttsId: 7,
      text: '旁白文本',
      speed: 1.5,
      contentType: 'audio/wav',
    })

    await store.synthesizeSpeechChunkBlob('“对白”', 'dialogue')
    expect(vi.mocked(synthesizeHttpTts).mock.calls[1][0]).toMatchObject({
      ttsId: 8,
      text: '“对白”',
      contentType: 'audio/wav',
    })
  })

  it('falls back to the narration engine when no dialogue engine is configured', async () => {
    const store = useReaderStore()
    store.setSpeechProvider('http')
    store.setHttpTtsId(7, 'audio/wav')
    store.setDualVoiceEnabled(true)
    vi.mocked(synthesizeHttpTts).mockResolvedValue(new Blob(['audio']))

    await store.synthesizeSpeechChunkBlob('“对白”', 'dialogue')

    expect(vi.mocked(synthesizeHttpTts).mock.calls[0][0]).toMatchObject({ ttsId: 7 })
  })

  it('reports a readable error when the http engine is not selected', async () => {
    const store = useReaderStore()
    store.setSpeechProvider('http')

    // 提示由调用方（缓冲流 onError）统一弹，这里只保证错误信息可读。
    await expect(store.synthesizeSpeechChunkBlob('正文', 'narration')).rejects.toThrow('在线朗读引擎')
    expect(synthesizeHttpTts).not.toHaveBeenCalled()
  })

  it('uses the narration/dialogue voice for each openai chunk', async () => {
    const store = useReaderStore()
    store.setSpeechProvider('openai')
    store.setOpenAISpeechVoice('vivian')
    store.setDialogueOpenaiVoice('nova')
    store.setDualVoiceEnabled(true)
    vi.mocked(requestOpenAISpeechAudio).mockResolvedValue(new Blob(['audio']))

    await store.synthesizeSpeechChunkBlob('旁白', 'narration')
    expect(vi.mocked(requestOpenAISpeechAudio).mock.calls[0][0].voice).toBe('vivian')

    await store.synthesizeSpeechChunkBlob('“对白”', 'dialogue')
    expect(vi.mocked(requestOpenAISpeechAudio).mock.calls[1][0].voice).toBe('nova')

    // 对话音色留空时跟随旁白音色。
    store.setDialogueOpenaiVoice('')
    await store.synthesizeSpeechChunkBlob('“对白”', 'dialogue')
    expect(vi.mocked(requestOpenAISpeechAudio).mock.calls[2][0].voice).toBe('vivian')
  })

  it('rejects an already-aborted signal without calling the backend', async () => {
    const store = useReaderStore()
    store.setSpeechProvider('http')
    store.setHttpTtsId(7)
    const controller = new AbortController()
    controller.abort()

    await expect(store.synthesizeSpeechChunkBlob('正文', 'narration', controller.signal))
      .rejects.toThrow()
    expect(synthesizeHttpTts).not.toHaveBeenCalled()
  })
})

describe('reader speech stream controls', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    stubStorage()
    stubAudioGlobals()
    resetStoreMocks()
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('routes pause/resume/stop to the attached stream controller', () => {
    const store = useReaderStore()
    const controls = { pause: vi.fn(), resume: vi.fn(), stop: vi.fn() }
    store.attachSpeechStreamControls(controls)

    store.pauseTTS()
    expect(controls.pause).toHaveBeenCalledTimes(1)
    expect(store.isPaused).toBe(true)

    store.pauseTTS()
    expect(controls.resume).toHaveBeenCalledTimes(1)
    expect(store.isPaused).toBe(false)

    store.stopTTS()
    expect(controls.stop).toHaveBeenCalledTimes(1)
    // 句柄在 stop 时摘掉，后续 stop 不会再打到已经停掉的流上。
    store.stopTTS()
    expect(controls.stop).toHaveBeenCalledTimes(1)
  })

  it('stops an active stream before starting a new utterance', async () => {
    const store = useReaderStore()
    const controls = { pause: vi.fn(), resume: vi.fn(), stop: vi.fn() }
    store.attachSpeechStreamControls(controls)
    store.setSpeechProvider('http')
    store.setHttpTtsId(7)
    vi.mocked(synthesizeHttpTts).mockResolvedValue(new Blob(['audio']))

    store.startTTS('新的正文')

    expect(controls.stop).toHaveBeenCalledTimes(1)
    await vi.waitFor(() => expect(synthesizeHttpTts).toHaveBeenCalledTimes(1))
  })
})

describe('reader dual voice over system speech', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    stubStorage()
    stubAudioGlobals(true)
    synthStub.voices = [
      { name: 'Voice-A', lang: 'zh-CN' },
      { name: 'Voice-B', lang: 'zh-CN' },
    ]
    synthStub.spoken = []
    synthStub.speaking = false
    synthStub.pending = false
    synthStub.paused = false
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('queues one utterance per segment with the matching voice', () => {
    const store = useReaderStore()
    store.setSpeechProvider('system')
    store.setVoiceName('Voice-A')
    store.setDialogueVoiceName('Voice-B')
    store.setDualVoiceEnabled(true)
    const onStart = vi.fn()
    const onEnd = vi.fn()

    store.startTTS('他说：“你好。”然后走了。', { onStart, onEnd })

    // 连续 speak() 交给浏览器队列串行：这里不应该有任何递归/定时器。
    expect(synthStub.spoken).toHaveLength(3)
    expect(synthStub.spoken.map((utterance) => utterance.text)).toEqual([
      '他说：',
      '“你好。”',
      '然后走了。',
    ])
    expect(synthStub.spoken.map((utterance) => utterance.voice?.name)).toEqual([
      'Voice-A',
      'Voice-B',
      'Voice-A',
    ])

    synthStub.spoken[0].onstart?.()
    expect(store.isSpeaking).toBe(true)
    expect(onStart).toHaveBeenCalledTimes(1)

    synthStub.spoken[0].onend?.()
    synthStub.spoken[1].onend?.()
    // 中间片段结束不能触发 onEnd，否则自动朗读会在段中翻章。
    expect(onEnd).not.toHaveBeenCalled()

    synthStub.spoken[2].onend?.()
    expect(onEnd).toHaveBeenCalledTimes(1)
    expect(store.isSpeaking).toBe(false)
    expect(store.speechProgress).toBe(1)
  })

  it('does not report cancellation as a playback error', () => {
    const store = useReaderStore()
    store.setSpeechProvider('system')
    store.setDualVoiceEnabled(true)
    const onError = vi.fn()

    store.startTTS('他说：“你好。”', { onError })
    synthStub.spoken[0].onerror?.({ error: 'interrupted' })

    expect(onError).not.toHaveBeenCalled()
  })
})
