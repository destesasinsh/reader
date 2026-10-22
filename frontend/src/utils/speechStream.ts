// 朗读缓冲流：始终保有 N 个字符的已合成音频，边播边补，块间无缝切换。
//
// 为什么需要它：旧的串行链路是「请求一块 → 等网络 → 播 → onEnd → 再请求」，每块
// （几十到几百字）之间都有一次网络空档。把「合成」和「播放」解耦后，网络等待被藏
// 在播放后面：当前块在播时，下一块已经在另一个 <audio> 上准备好，onended 立即切。
//
// 这里刻意不依赖 Vue / DOM 事件循环以外的东西：createAudio 可注入，synthesize /
// supply 由调用方提供，因而可以用假 audio + 可控延迟的单测覆盖时序。

export type SpeechChunkType = 'narration' | 'dialogue'

export interface SpeechChunk {
  text: string
  type: SpeechChunkType
  /** 该块属于哪一章；播放跨章时用于通知 UI 切换。 */
  chapterIndex: number
}

export interface SpeechStreamHandlers {
  /** 需要更多文本时调用；返回 [] 表示没有更多（书末）。 */
  supply: () => Promise<SpeechChunk[]>
  /** 按块合成音频；provider 适配层实现（http / openai）。 */
  synthesize: (chunk: SpeechChunk, signal: AbortSignal) => Promise<Blob>
  onChunkStart?: (chunk: SpeechChunk) => void
  onChunkProgress?: (chunk: SpeechChunk, progress: number) => void
  /** 播放即将进入下一章时调用（用于切章 UI）；返回 Promise 时等它完成再播。 */
  onChapterChange?: (chapterIndex: number) => void | Promise<void>
  /** 缓冲空且没有更多文本。 */
  onIdle?: () => void
  /** 致命错误：连续多块失败（通常意味着真的网络/通道故障），流会停止。 */
  onError?: (error: Error) => void
  /**
   * 单块重试后仍失败、已跳过继续播放时调用。
   * 同一条错误消息在一次会话内最多回调一次（避免每块都弹提示刷屏）。
   */
  onChunkError?: (error: Error, chunk: SpeechChunk) => void
}

export interface SpeechStreamOptions {
  /** 目标缓冲字符数（默认 500）。 */
  bufferChars: number
  /** 同时合成请求数，默认 2。 */
  maxConcurrent?: number
  /** 便于测试注入；默认 () => new Audio()。 */
  createAudio?: () => HTMLAudioElement
}

const DEFAULT_BUFFER_CHARS = 500
const DEFAULT_MAX_CONCURRENT = 2
/** 单块合成失败后的重试次数：只重试一次，间隔 300ms（瞬时抖动通常一次就恢复）。 */
const CHUNK_RETRY_ATTEMPTS = 1
const CHUNK_RETRY_DELAY_MS = 300
/** 连续这么多块（重试后仍失败）才认为通道真的不可用，终止整段朗读。 */
const MAX_CONSECUTIVE_CHUNK_FAILURES = 5
const EMPTY_AUDIO_MESSAGE = '朗读引擎返回了空音频'
const PLAYBACK_ERROR_MESSAGE = '朗读音频播放失败'

interface IndexedChunk {
  chunk: SpeechChunk
  index: number
}

interface InFlightEntry extends IndexedChunk {
  controller: AbortController
  /** 已经失败过几次（0 = 首次请求）。用于决定重试还是跳过。 */
  attempt: number
}

interface ReadyEntry extends IndexedChunk {
  blob: Blob
}

function toError(error: unknown): Error {
  if (error instanceof Error) return error
  const message = typeof error === 'string' ? error.trim() : ''
  return new Error(message || '朗读失败')
}

export class SpeechStream {
  private readonly handlers: SpeechStreamHandlers
  private readonly bufferChars: number
  private readonly maxConcurrent: number
  private readonly createAudio: () => HTMLAudioElement
  /** 双 audio 乒乓：一个在播时另一个预挂下一块，省掉每次 new Audio 的启动开销。 */
  private readonly audios: HTMLAudioElement[]

  private stopped = false
  private started = false
  private paused = false
  private idleNotified = false

  private supplyInFlight = false
  private supplyExhausted = false
  private nextChunkIndex = 0
  /** 已供给、还没排进合成队列的块（只占文本内存）。 */
  private dispatchQueue: IndexedChunk[] = []
  private inflight = new Map<number, InFlightEntry>()
  private ready = new Map<number, ReadyEntry>()
  /** 重试后仍失败、被跳过的块。播放推进时要越过这些 index，否则会永远等不到。 */
  private skipped = new Set<number>()
  private retryTimers = new Set<ReturnType<typeof setTimeout>>()
  private nextPlayIndex = 0

  /** 连续失败块数：任一块成功即清零；达到阈值才终止整段朗读。 */
  private consecutiveChunkFailures = 0
  /** 已提示过的单块错误消息（每会话一次，避免同一故障刷屏）。 */
  private reportedChunkErrors = new Set<string>()

  private playingAudio: HTMLAudioElement | null = null
  private playingEntry: ReadyEntry | null = null
  private playingUrl = ''
  /** 已预挂到空闲 audio、等待接班的下一块。 */
  private prepared: { entry: ReadyEntry; audio: HTMLAudioElement; url: string } | null = null
  private lastChapterIndex: number | null = null
  private starting = false

  constructor(handlers: SpeechStreamHandlers, options: SpeechStreamOptions) {
    this.handlers = handlers
    this.bufferChars = Math.max(1, options.bufferChars || DEFAULT_BUFFER_CHARS)
    this.maxConcurrent = Math.max(1, options.maxConcurrent || DEFAULT_MAX_CONCURRENT)
    this.createAudio = options.createAudio || (() => new Audio())
    this.audios = [this.createAudio(), this.createAudio()]
  }

  /** 已就绪未播 + 在途（不含正在播的那块，也不含尚未排入合成的文本队列）。 */
  get bufferedChars(): number {
    let total = 0
    for (const entry of this.inflight.values()) total += entry.chunk.text.length
    for (const entry of this.ready.values()) total += entry.chunk.text.length
    return total
  }

  get inFlightCount(): number {
    return this.inflight.size
  }

  get isPlaying(): boolean {
    return this.playingAudio !== null && this.playingEntry !== null
  }

  start(): void {
    if (this.started || this.stopped) return
    this.started = true
    this.pump()
  }

  pause(): void {
    if (this.stopped || this.paused) return
    this.paused = true
    if (this.playingAudio) {
      this.playingAudio.pause()
    }
  }

  resume(): void {
    if (this.stopped || !this.paused) return
    this.paused = false
    if (this.playingAudio) {
      // 浏览器可能拒绝恢复（自动播放策略），此时当作播放失败处理，别静默卡住。
      this.playingAudio.play().catch((error: Error) => this.fail(toError(error)))
    }
    this.pump()
  }

  stop(): void {
    if (this.stopped) return
    this.stopped = true
    // 中止在途合成：网络请求无法真正取消时（Tauri IPC），AbortSignal 至少让
    // 回调在返回后立刻丢弃，不会在停止后突然开始播音。
    for (const entry of this.inflight.values()) {
      entry.controller.abort()
    }
    this.inflight.clear()
    for (const timer of this.retryTimers) clearTimeout(timer)
    this.retryTimers.clear()
    this.dispatchQueue = []
    this.ready.clear()
    this.skipped.clear()
    this.supplyExhausted = true
    if (this.prepared) {
      this.pauseAudio(this.prepared.audio)
      this.releaseUrl(this.prepared.url)
      this.prepared = null
    }
    if (this.playingAudio) {
      this.pauseAudio(this.playingAudio)
      this.releaseUrl(this.playingUrl)
    }
    this.playingAudio = null
    this.playingEntry = null
    this.playingUrl = ''
  }

  private releaseUrl(url: string) {
    if (!url) return
    try {
      URL.revokeObjectURL(url)
    } catch {
      // 测试环境/极端情况下 revoke 可能不可用，不能因此中断停止流程。
    }
  }

  private pauseAudio(audio: HTMLAudioElement) {
    try {
      audio.pause()
    } catch {
      // 忽略：部分平台在 src 已清空时 pause 会抛。
    }
    audio.onended = null
    audio.onerror = null
    audio.ontimeupdate = null
    audio.onloadedmetadata = null
    audio.src = ''
  }

  private pump() {
    if (this.stopped || !this.started) return
    // 1) 需要更多文本：队列空了且缓冲低于目标时才向 supply 要。
    //    单飞由 supplyInFlight 保证；返回 [] 后不再调用。
    if (
      !this.supplyExhausted
      && !this.supplyInFlight
      && this.dispatchQueue.length === 0
      && this.bufferedChars < this.bufferChars
    ) {
      void this.supplyMore()
    }
    // 2) 合成：有空位就派发。用「就绪+在途」控制额度，避免 supply 一次给出整章时
    //    立刻把整章都合成出来（音频 blob 占内存，且对流畅度没有额外收益）。
    this.dispatch()
    // 3) 播放：当前没在播就取下一块。
    if (!this.paused) this.playNext()
    this.maybeNotifyIdle()
  }

  private async supplyMore() {
    if (this.stopped || this.supplyInFlight || this.supplyExhausted) return
    this.supplyInFlight = true
    try {
      const chunks = await this.handlers.supply()
      if (this.stopped) return
      const usable = (chunks || []).filter((chunk) => Boolean(chunk?.text?.trim()))
      if (!usable.length) {
        this.supplyExhausted = true
        return
      }
      for (const chunk of usable) {
        this.dispatchQueue.push({ chunk, index: this.nextChunkIndex })
        this.nextChunkIndex += 1
      }
    } catch (error) {
      this.fail(toError(error))
    } finally {
      this.supplyInFlight = false
      if (!this.stopped) this.pump()
    }
  }

  private dispatch() {
    while (
      !this.stopped
      && this.inflight.size < this.maxConcurrent
      && this.dispatchQueue.length > 0
      && this.readyChars() + this.inflightChars() < this.bufferChars
    ) {
      const item = this.dispatchQueue.shift() as IndexedChunk
      this.dispatchItem({ ...item, attempt: 0 })
    }
  }

  /** 派发（或重试）一次合成请求。重试也走这里，保证 attempt 传递一致。 */
  private dispatchItem(item: IndexedChunk & { attempt: number }) {
    const controller = new AbortController()
    this.inflight.set(item.index, { ...item, controller })
    void this.handlers.synthesize(item.chunk, controller.signal)
      .then((blob) => {
        if (!blob || blob.size <= 0) {
          // 空音频和「合成报错」等价处理：可能只是服务端对该片段无内容可读，
          // 同样先重试一次，再跳过，而不是直接终止整段朗读。
          this.handleChunkFailure(item, controller, new Error(EMPTY_AUDIO_MESSAGE))
          return
        }
        this.handleSynthesized(item, blob)
      })
      .catch((error) => this.handleChunkFailure(item, controller, toError(error)))
  }

  private readyChars() {
    let total = 0
    for (const entry of this.ready.values()) total += entry.chunk.text.length
    return total
  }

  private inflightChars() {
    let total = 0
    for (const entry of this.inflight.values()) total += entry.chunk.text.length
    return total
  }

  private handleSynthesized(item: IndexedChunk, blob: Blob) {
    if (this.stopped || !this.inflight.has(item.index)) return
    this.inflight.delete(item.index)
    // 有块成功就说明通道恢复，连续失败计数清零。
    this.consecutiveChunkFailures = 0
    this.ready.set(item.index, { ...item, blob })
    // 在播时下一块若此刻才合成完，立刻挂到空闲 audio 上，onended 时才能无缝切。
    this.prepareNext()
    this.pump()
  }

  /**
   * 单块合成失败：先重试一次（300ms 后），仍失败则跳过继续。
   *
   * 为什么不能一失败就终止：在线朗读对纯标点/空内容片段本来就可能不返回音频，
   * 一个坏块不应该让整本书停下；只有连续多块失败才视为真正的通道故障。
   */
  private handleChunkFailure(item: IndexedChunk & { attempt: number }, controller: AbortController, error: Error) {
    if (this.stopped || controller.signal.aborted) return
    if (!this.inflight.has(item.index)) return
    this.inflight.delete(item.index)

    if (item.attempt < CHUNK_RETRY_ATTEMPTS) {
      const timer = setTimeout(() => {
        this.retryTimers.delete(timer)
        if (this.stopped) return
        // 重试前再确认这块没被停止/跳过（停止会清空队列并 abort）。
        if (this.skipped.has(item.index) || this.ready.has(item.index)) return
        this.dispatchItem({ ...item, attempt: item.attempt + 1 })
        this.pump()
      }, CHUNK_RETRY_DELAY_MS)
      this.retryTimers.add(timer)
      // 重试期间允许先把后续块派发出去（不占用合成额度）。
      this.pump()
      return
    }

    this.consecutiveChunkFailures += 1
    if (this.consecutiveChunkFailures >= MAX_CONSECUTIVE_CHUNK_FAILURES) {
      // 连续多块都失败：这通常不是单个片段的问题，而是通道/网络真的不可用。
      this.fail(error)
      return
    }

    // 跳过本块：记录 index 让播放推进越过它；同一错误每会话只提示一次。
    this.skipped.add(item.index)
    const message = error.message || '朗读失败'
    if (!this.reportedChunkErrors.has(message)) {
      this.reportedChunkErrors.add(message)
      this.handlers.onChunkError?.(error, item.chunk)
    }
    this.pump()
  }

  private pickIdleAudio(): HTMLAudioElement {
    // 乒乓：优先用没在播、也没预挂的那只。
    for (const audio of this.audios) {
      if (audio !== this.playingAudio && audio !== this.prepared?.audio) return audio
    }
    return this.audios.find((audio) => audio !== this.playingAudio) || this.audios[0]
  }

  private playNext() {
    if (this.stopped || this.paused || this.playingAudio || this.starting) return
    // 失败被跳过的块永远不会出现在 ready 里：先把播放位置推过它们，否则会死等。
    while (!this.ready.has(this.nextPlayIndex) && this.skipped.has(this.nextPlayIndex)) {
      this.nextPlayIndex += 1
    }
    const entry = this.ready.get(this.nextPlayIndex)
    if (!entry) return
    this.starting = true
    void this.startEntry(entry)
      .catch((error: Error) => this.fail(toError(error)))
      .finally(() => {
        this.starting = false
        if (!this.stopped) this.pump()
      })
  }

  private async startEntry(entry: ReadyEntry) {
    // 跨章：先把 UI 切到目标章节（加载正文/DOM），完成后再播这一块。
    if (this.lastChapterIndex !== null && entry.chunk.chapterIndex !== this.lastChapterIndex) {
      if (this.handlers.onChapterChange) {
        await this.handlers.onChapterChange(entry.chunk.chapterIndex)
        if (this.stopped || this.paused) return
      }
    }
    this.lastChapterIndex = entry.chunk.chapterIndex
    this.ready.delete(entry.index)
    this.nextPlayIndex = entry.index + 1

    let audio: HTMLAudioElement
    let url: string
    if (this.prepared && this.prepared.entry.index === entry.index) {
      // 已经预挂好，直接接班 —— 这正是「无缝」的关键路径。
      audio = this.prepared.audio
      url = this.prepared.url
      this.prepared = null
    } else {
      audio = this.pickIdleAudio()
      url = this.createObjectUrl(entry.blob)
      this.attachAudio(audio)
      audio.src = url
    }

    this.playingAudio = audio
    this.playingEntry = entry
    this.playingUrl = url
    this.handlers.onChunkStart?.(entry.chunk)
    await audio.play()
    if (this.stopped) return
    this.prepareNext()
    this.pump()
  }

  private createObjectUrl(blob: Blob): string {
    return URL.createObjectURL(blob)
  }

  private prepareNext() {
    if (this.stopped || this.paused || this.prepared) return
    const entry = this.ready.get(this.nextPlayIndex)
    if (!entry) return
    const audio = this.pickIdleAudio()
    const url = this.createObjectUrl(entry.blob)
    this.attachAudio(audio)
    audio.src = url
    this.prepared = { entry, audio, url }
  }

  private attachAudio(audio: HTMLAudioElement) {
    audio.onended = () => this.handleAudioEnded(audio)
    audio.onerror = () => {
      if (audio === this.playingAudio) this.fail(new Error(PLAYBACK_ERROR_MESSAGE))
    }
    audio.ontimeupdate = () => this.handleAudioProgress(audio)
    audio.onloadedmetadata = () => this.handleAudioProgress(audio)
  }

  private handleAudioProgress(audio: HTMLAudioElement) {
    if (audio !== this.playingAudio || !this.playingEntry) return
    const duration = audio.duration
    if (!Number.isFinite(duration) || duration <= 0) return
    const progress = Math.max(0, Math.min(1, audio.currentTime / duration))
    this.handlers.onChunkProgress?.(this.playingEntry.chunk, progress)
  }

  private handleAudioEnded(audio: HTMLAudioElement) {
    if (audio !== this.playingAudio || !this.playingEntry) return
    this.pauseAudio(audio)
    this.releaseUrl(this.playingUrl)
    this.playingAudio = null
    this.playingEntry = null
    this.playingUrl = ''
    this.pump()
  }

  private maybeNotifyIdle() {
    if (this.stopped || this.idleNotified || !this.started) return
    if (!this.supplyExhausted || this.supplyInFlight) return
    // 还有块在等重试时不能算 idle，否则会提前触发「继续下一段/下一章」。
    if (this.retryTimers.size) return
    if (this.dispatchQueue.length || this.inflight.size || this.ready.size) return
    if (this.playingAudio || this.starting) return
    if (this.paused) return
    this.idleNotified = true
    this.handlers.onIdle?.()
  }

  private fail(error: Error) {
    if (this.stopped) return
    this.handlers.onError?.(error)
    this.stop()
  }
}
