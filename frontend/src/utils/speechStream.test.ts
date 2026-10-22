import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SpeechStream, type SpeechChunk } from './speechStream'

/** 假 audio：记录 play/pause/切块，能被测试手动触发 onended / 进度。 */
class FakeAudio {
  src = ''
  paused = true
  ended = false
  duration = 10
  currentTime = 0
  playCalls = 0
  pauseCalls = 0
  onended: (() => void) | null = null
  onerror: (() => void) | null = null
  ontimeupdate: (() => void) | null = null
  onloadedmetadata: (() => void) | null = null

  play() {
    this.playCalls += 1
    this.paused = false
    return Promise.resolve()
  }

  pause() {
    this.pauseCalls += 1
    this.paused = true
  }

  /** 模拟播放自然结束。 */
  finish() {
    this.ended = true
    this.onended?.()
  }

  /** 模拟播放进度（用于进度回调）。 */
  seekTo(currentTime: number) {
    this.currentTime = currentTime
    this.ontimeupdate?.()
  }
}

interface Deferred<T> {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (error: unknown) => void
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

function chunk(text: string, chapterIndex = 0, type: 'narration' | 'dialogue' = 'narration'): SpeechChunk {
  return { text, type, chapterIndex }
}

const AUDIO_BLOB = new Blob(['audio'])

let audioInstances: FakeAudio[]
let createdUrls: string[]
let revokedUrls: string[]

function setup(options: {
  chunks: SpeechChunk[]
  bufferChars?: number
  maxConcurrent?: number
  synthesize?: (chunk: SpeechChunk, signal: AbortSignal) => Promise<Blob>
}) {
  audioInstances = []
  createdUrls = []
  revokedUrls = []
  const handlers = {
    supply: vi.fn(),
    synthesize: vi.fn(),
    onChunkStart: vi.fn(),
    onChunkProgress: vi.fn(),
    onChapterChange: vi.fn(),
    onIdle: vi.fn(),
    onError: vi.fn(),
    onChunkError: vi.fn(),
  }
  handlers.synthesize.mockImplementation(options.synthesize || (() => Promise.resolve(AUDIO_BLOB)))
  // 默认每批 2 块、供完返回 []（贴近真实 supply 的「按需小批量」用法）。
  let cursor = 0
  handlers.supply.mockImplementation(async () => {
    const batch = options.chunks.slice(cursor, cursor + 2)
    cursor += batch.length
    return batch
  })
  const stream = new SpeechStream(handlers, {
    bufferChars: options.bufferChars ?? 500,
    maxConcurrent: options.maxConcurrent ?? 2,
    createAudio: () => {
      const audio = new FakeAudio()
      audioInstances.push(audio)
      return audio as unknown as HTMLAudioElement
    },
  })
  return { stream, handlers }
}

function playingAudio(): FakeAudio | undefined {
  return audioInstances.find((audio) => audio.playCalls > 0 && !audio.paused)
}

function flush() {
  return new Promise((resolve) => setTimeout(resolve, 0))
}

describe('SpeechStream', () => {
  beforeEach(() => {
    vi.stubGlobal('URL', {
      createObjectURL: vi.fn(() => {
        const url = `blob:test-${createdUrls.length + 1}`
        createdUrls.push(url)
        return url
      }),
      revokeObjectURL: vi.fn((url: string) => {
        revokedUrls.push(url)
      }),
    })
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('starts playing as soon as the first chunk is synthesized', async () => {
    const blobs = new Map([
      ['第一块', deferred<Blob>()],
      ['第二块', deferred<Blob>()],
    ])
    const { stream, handlers } = setup({
      chunks: [chunk('第一块'), chunk('第二块')],
      synthesize: (item) => blobs.get(item.text)?.promise ?? Promise.resolve(AUDIO_BLOB),
    })

    stream.start()
    await vi.waitFor(() => expect(handlers.synthesize).toHaveBeenCalledTimes(2))

    blobs.get('第一块')?.resolve(AUDIO_BLOB)
    await vi.waitFor(() => expect(handlers.onChunkStart).toHaveBeenCalledTimes(1))
    // 第二块还没合成完，第一块已经在播 —— 首播不等缓冲填满。
    expect(handlers.onChunkStart.mock.calls[0][0].text).toBe('第一块')
    expect(stream.isPlaying).toBe(true)
    expect(playingAudio()).toBeDefined()
    expect(handlers.onChunkStart).toHaveBeenCalledTimes(1)
  })

  it('keeps synthesizing ahead while playing and never drops to a zero buffer', async () => {
    const sixChars = (index: number) => chunk(`块${index}12345`) // 每块 6 字
    const { stream, handlers } = setup({
      chunks: [0, 1, 2, 3, 4, 5].map(sixChars),
      bufferChars: 10,
    })

    stream.start()
    await vi.waitFor(() => expect(stream.isPlaying).toBe(true))
    // 缓冲目标 10 字 / 每块 6 字：只该预合成 3 块（12 字），不会把整章都请求掉。
    await vi.waitFor(() => expect(handlers.synthesize).toHaveBeenCalledTimes(3))
    await flush()
    expect(handlers.synthesize).toHaveBeenCalledTimes(3)
    expect(handlers.supply.mock.calls.length).toBeGreaterThanOrEqual(2)
    expect(stream.bufferedChars).toBeGreaterThanOrEqual(10)
    expect(stream.inFlightCount + stream.bufferedChars).toBeGreaterThan(0)
  })

  it('plays out-of-order synthesis results in chunk order', async () => {
    const chunks = [chunk('甲'), chunk('乙'), chunk('丙')]
    const blobs = new Map(chunks.map((item) => [item.text, deferred<Blob>()]))
    const { stream, handlers } = setup({
      chunks,
      maxConcurrent: 3,
      synthesize: (item) => blobs.get(item.text)?.promise ?? Promise.resolve(AUDIO_BLOB),
    })
    handlers.supply.mockImplementationOnce(async () => chunks)
    handlers.supply.mockImplementation(async () => [])

    stream.start()
    await vi.waitFor(() => expect(handlers.synthesize).toHaveBeenCalledTimes(3))
    // 后两块先回来：不能抢播。
    blobs.get('乙')?.resolve(AUDIO_BLOB)
    blobs.get('丙')?.resolve(AUDIO_BLOB)
    await flush()
    expect(handlers.onChunkStart).not.toHaveBeenCalled()

    blobs.get('甲')?.resolve(AUDIO_BLOB)
    await vi.waitFor(() => expect(handlers.onChunkStart).toHaveBeenCalledTimes(1))
    expect(handlers.onChunkStart.mock.calls[0][0].text).toBe('甲')

    playingAudio()?.finish()
    await vi.waitFor(() => expect(handlers.onChunkStart).toHaveBeenCalledTimes(2))
    expect(handlers.onChunkStart.mock.calls[1][0].text).toBe('乙')

    playingAudio()?.finish()
    await vi.waitFor(() => expect(handlers.onChunkStart).toHaveBeenCalledTimes(3))
    expect(handlers.onChunkStart.mock.calls[2][0].text).toBe('丙')
  })

  it('calls onIdle once when supply is exhausted and the buffer empties', async () => {
    const { stream, handlers } = setup({ chunks: [chunk('唯一一块')] })

    stream.start()
    await vi.waitFor(() => expect(stream.isPlaying).toBe(true))
    playingAudio()?.finish()

    await vi.waitFor(() => expect(handlers.onIdle).toHaveBeenCalledTimes(1))
    // supply 返回空之后不能再被调用。
    expect(handlers.supply).toHaveBeenCalledTimes(2)
    await flush()
    expect(handlers.supply).toHaveBeenCalledTimes(2)
    expect(handlers.onError).not.toHaveBeenCalled()
  })

  it('pauses and resumes the current audio', async () => {
    const { stream } = setup({ chunks: [chunk('正在播放')] })
    stream.start()
    await vi.waitFor(() => expect(stream.isPlaying).toBe(true))
    const audio = playingAudio() as FakeAudio
    expect(audio.paused).toBe(false)

    stream.pause()
    expect(audio.paused).toBe(true)

    stream.resume()
    await vi.waitFor(() => expect(audio.paused).toBe(false))
    expect(audio.playCalls).toBe(2)
  })

  it('stops playback, aborts in-flight synthesis and releases object URLs', async () => {
    const pending = deferred<Blob>()
    const { stream, handlers } = setup({
      chunks: [chunk('第一块'), chunk('第二块')],
      synthesize: (item) => (item.text === '第一块' ? Promise.resolve(AUDIO_BLOB) : pending.promise),
    })

    stream.start()
    await vi.waitFor(() => expect(stream.isPlaying).toBe(true))
    const audio = playingAudio() as FakeAudio
    const inflightSignal = handlers.synthesize.mock.calls[1]?.[1] as AbortSignal

    stream.stop()
    expect(stream.isPlaying).toBe(false)
    expect(audio.paused).toBe(true)
    expect(audio.src).toBe('')
    expect(inflightSignal.aborted).toBe(true)
    expect(revokedUrls.length).toBeGreaterThan(0)

    // 停止之后迟到的合成结果不能再触发播放或 idle。
    pending.resolve(AUDIO_BLOB)
    await flush()
    expect(handlers.onChunkStart).toHaveBeenCalledTimes(1)
    expect(handlers.onIdle).not.toHaveBeenCalled()
  })

  it('waits for onChapterChange before playing the first chunk of the next chapter', async () => {
    const events: string[] = []
    const { stream, handlers } = setup({
      chunks: [chunk('第一章', 0), chunk('第二章', 1)],
    })
    handlers.onChunkStart.mockImplementation((item: SpeechChunk) => {
      events.push(`start:${item.text}`)
    })
    handlers.onChapterChange.mockImplementation(async (chapterIndex: number) => {
      events.push(`chapter:${chapterIndex}`)
    })
    handlers.supply.mockImplementationOnce(async () => [chunk('第一章', 0), chunk('第二章', 1)])
    handlers.supply.mockImplementation(async () => [])

    stream.start()
    await vi.waitFor(() => expect(events).toEqual(['start:第一章']))
    playingAudio()?.finish()

    await vi.waitFor(() => expect(events).toEqual(['start:第一章', 'chapter:1', 'start:第二章']))
    expect(handlers.onChapterChange).toHaveBeenCalledTimes(1)
  })

  it('retries a failed chunk once and plays it when the retry succeeds', async () => {
    const failure = new Error('临时网络抖动')
    let attempts = 0
    const { stream, handlers } = setup({
      chunks: [chunk('重试成功的块')],
      synthesize: () => {
        attempts += 1
        return attempts === 1 ? Promise.reject(failure) : Promise.resolve(AUDIO_BLOB)
      },
    })

    stream.start()
    // 首次失败后 300ms 重试；重试成功则照常播放，不提示、不中断。
    await vi.waitFor(() => expect(handlers.onChunkStart).toHaveBeenCalledTimes(1), { timeout: 2000 })
    expect(attempts).toBe(2)
    expect(handlers.onChunkError).not.toHaveBeenCalled()
    expect(handlers.onError).not.toHaveBeenCalled()
  })

  it('skips a chunk that fails twice and keeps playing the next one', async () => {
    const failure = new Error('该片段没有可朗读的内容')
    const { stream, handlers } = setup({
      chunks: [chunk('坏块'), chunk('好块')],
      synthesize: (item) => (item.text === '坏块' ? Promise.reject(failure) : Promise.resolve(AUDIO_BLOB)),
    })

    stream.start()

    // 坏块重试一次仍失败 → 跳过；后续块照常播放，不终止整段朗读。
    await vi.waitFor(() => expect(handlers.onChunkStart).toHaveBeenCalledTimes(1), { timeout: 2000 })
    expect(handlers.onChunkStart.mock.calls[0][0].text).toBe('好块')
    expect(handlers.onChunkError).toHaveBeenCalledTimes(1)
    expect((handlers.onChunkError.mock.calls[0][0] as Error).message).toBe('该片段没有可朗读的内容')
    expect(handlers.onError).not.toHaveBeenCalled()
  })

  it('reports the same skipped-chunk error only once per session', async () => {
    const failure = new Error('同一类失败')
    const { stream, handlers } = setup({
      chunks: [chunk('坏块一'), chunk('坏块二'), chunk('坏块三')],
      maxConcurrent: 1,
      synthesize: () => Promise.reject(failure),
    })

    stream.start()

    await vi.waitFor(() => expect(handlers.onIdle).toHaveBeenCalledTimes(1), { timeout: 3000 })
    // 三块都失败（重试后跳过），但同类提示只回调一次；连续失败数未到阈值所以不终止。
    expect(handlers.onChunkError).toHaveBeenCalledTimes(1)
    expect(handlers.onError).not.toHaveBeenCalled()
  })

  it('terminates the stream after five consecutive failed chunks', async () => {
    const failure = new Error('通道持续失败')
    const { stream, handlers } = setup({
      chunks: [chunk('块一'), chunk('块二'), chunk('块三'), chunk('块四'), chunk('块五'), chunk('块六')],
      maxConcurrent: 1,
      synthesize: () => Promise.reject(failure),
    })

    stream.start()

    await vi.waitFor(() => expect(handlers.onError).toHaveBeenCalledTimes(1), { timeout: 5000 })
    expect((handlers.onError.mock.calls[0][0] as Error).message).toBe('通道持续失败')
    expect(stream.isPlaying).toBe(false)
    // 前四块跳过时已提示过一次，第五块达到阈值直接终止。
    expect(handlers.onChunkError).toHaveBeenCalledTimes(1)
  })

  it('treats an empty audio blob like a failed chunk instead of killing the stream', async () => {
    let attempts = 0
    const { stream, handlers } = setup({
      chunks: [chunk('空音频')],
      synthesize: () => {
        attempts += 1
        return Promise.resolve(attempts === 1 ? new Blob([]) : AUDIO_BLOB)
      },
    })

    stream.start()

    // 空音频先重试一次：第二次有内容就照常播放，不弹致命错误。
    await vi.waitFor(() => expect(handlers.onChunkStart).toHaveBeenCalledTimes(1), { timeout: 2000 })
    expect(attempts).toBe(2)
    expect(handlers.onError).not.toHaveBeenCalled()
  })
})
