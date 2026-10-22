// 旁白 / 对话切分。
//
// 朗读时「旁白一个音色、对话一个音色」需要先把一段正文按中文引号切开。这里
// 刻意做成纯函数（不碰 DOM / IPC / store）：切分规则很容易写错（未闭合、嵌套、
// 多种引号混用），单独可枚举的输入输出才能把这些边界钉死在测试里。

export type SpeechSegmentType = 'narration' | 'dialogue'

export interface SpeechSegment {
  type: SpeechSegmentType
  text: string
}

/**
 * 引号开符 → 期望的闭符。
 *
 * 只认中文引号：西文 `"..."` 在中文正文里极少成对出现（半角引号还常被用作
 * 长度单位、英寸、代码片段），按它切分会把旁白整段误判成对话。Legado 的对话
 * 正文用的就是这三种成对引号。
 */
const QUOTE_PAIRS: Record<string, string> = {
  '\u201c': '\u201d', // “ ”
  '\u300c': '\u300d', // 「 」
  '\u300e': '\u300f', // 『 』
}

function isEmptyText(text: string): boolean {
  // trim() 同时吃掉半角与全角空格（U+3000）—— 这正是「只 trim 两端」的语义：
  // 文本内部的缩进/空格保留，两端的排版空白不产出空片段。
  return !text.trim()
}

/**
 * 按中文引号把正文切成旁白 / 对话片段。
 *
 * 规则：
 * - 引号（含开闭符本身）及其内容算对话，引号外算旁白；
 * - 未闭合引号：从该引号起到文本末尾都算对话（跨段对话的常见形态）；
 * - 同类引号嵌套按深度配对，第一个闭符不结束外层；
 * - 每个片段 trim 两端，空片段丢弃；连续同类片段不合并（保持与原文对应）。
 *
 * 返回顺序与原文一致；没有引号时返回单个 narration 片段。
 */
export function splitSpeechSegments(text: string): SpeechSegment[] {
  if (isEmptyText(text)) return []

  const segments: SpeechSegment[] = []
  // 当前正在累积的片段类型。切入/切出对话时立刻 flush，保证顺序与原文一致。
  let type: SpeechSegmentType = 'narration'
  let buffer = ''
  // 嵌套引号的期望闭符栈：栈非空即「仍在对话里」。
  const closerStack: string[] = []

  const flush = () => {
    const value = buffer.trim()
    if (value) segments.push({ type, text: value })
    buffer = ''
  }

  for (const char of text) {
    if (type === 'narration') {
      const closer = QUOTE_PAIRS[char]
      if (closer) {
        // 引号前若已有旁白，先收尾；开符本身属于对话片段。
        flush()
        type = 'dialogue'
        closerStack.push(closer)
        buffer = char
      } else {
        buffer += char
      }
      continue
    }

    // ── 对话中 ──
    // 只有「栈顶期望的闭符」才能收一层；其它闭符当普通字符保留，
    // 否则 `“他说「…」”` 会在内层的 」 处把外层也提前关掉。
    if (closerStack.length && char === closerStack[closerStack.length - 1]) {
      closerStack.pop()
      buffer += char
      if (!closerStack.length) {
        flush()
        type = 'narration'
      }
      continue
    }
    const nestedCloser = QUOTE_PAIRS[char]
    if (nestedCloser) {
      // 嵌套引号（同类或异类）一律押栈，等它自己的闭符出现；内容仍是同一段对话。
      closerStack.push(nestedCloser)
    }
    buffer += char
  }

  // 收尾：未闭合引号时 buffer 里就是「引号之后到段尾」的对话；正常结束时
  // 这里是最后一段旁白（或已 flush 过，buffer 为空）。
  flush()
  return segments
}
