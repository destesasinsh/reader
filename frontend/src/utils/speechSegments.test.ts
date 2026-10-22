import { describe, expect, it } from 'vitest'
import { splitSpeechSegments } from './speechSegments'

describe('splitSpeechSegments', () => {
  it('returns a single narration segment when there are no quotes', () => {
    expect(splitSpeechSegments('他推开门，屋里没有人。')).toEqual([
      { type: 'narration', text: '他推开门，屋里没有人。' },
    ])
  })

  it('keeps a single quoted dialogue as one dialogue segment', () => {
    expect(splitSpeechSegments('“你好。”')).toEqual([
      { type: 'dialogue', text: '“你好。”' },
    ])
  })

  it('separates narration, dialogue and trailing narration in order', () => {
    expect(splitSpeechSegments('他说：“你好。”然后走了。')).toEqual([
      { type: 'narration', text: '他说：' },
      { type: 'dialogue', text: '“你好。”' },
      { type: 'narration', text: '然后走了。' },
    ])
  })

  it('treats an unclosed quote as dialogue until the end of the text', () => {
    // 跨段对话在正文里很常见：整段只有一个开引号。
    expect(splitSpeechSegments('他说：“今天不走了')).toEqual([
      { type: 'narration', text: '他说：' },
      { type: 'dialogue', text: '“今天不走了' },
    ])
  })

  it('pairs nested same-kind quotes by depth instead of closing at the first closer', () => {
    expect(splitSpeechSegments('“他说“快走”，然后跑了。”')).toEqual([
      { type: 'dialogue', text: '“他说“快走”，然后跑了。”' },
    ])
  })

  it('keeps nested different-kind quotes inside one dialogue segment', () => {
    expect(splitSpeechSegments('他喊：“先走「左边」那条路”')).toEqual([
      { type: 'narration', text: '他喊：' },
      { type: 'dialogue', text: '“先走「左边」那条路”' },
    ])
  })

  it('recognises the corner-bracket quote forms', () => {
    expect(splitSpeechSegments('「走吧」他挥挥手。')).toEqual([
      { type: 'dialogue', text: '「走吧」' },
      { type: 'narration', text: '他挥挥手。' },
    ])
    expect(splitSpeechSegments('『别出声』')).toEqual([
      { type: 'dialogue', text: '『别出声』' },
    ])
  })

  it('keeps consecutive dialogue segments separate (no merging)', () => {
    expect(splitSpeechSegments('“第一句。”“第二句。”')).toEqual([
      { type: 'dialogue', text: '“第一句。”' },
      { type: 'dialogue', text: '“第二句。”' },
    ])
  })

  it('drops whitespace-only segments between quotes', () => {
    expect(splitSpeechSegments('“甲”   “乙”')).toEqual([
      { type: 'dialogue', text: '“甲”' },
      { type: 'dialogue', text: '“乙”' },
    ])
  })

  it('trims only the ends of each segment', () => {
    // 全角空格在正文中间必须保留：它常被用来做中文排版缩进/停顿。
    const raw = '  旁白开头\u3000\u3000内部停顿  “对\u3000话”  尾巴  '
    expect(splitSpeechSegments(raw)).toEqual([
      { type: 'narration', text: '旁白开头\u3000\u3000内部停顿' },
      { type: 'dialogue', text: '“对\u3000话”' },
      { type: 'narration', text: '尾巴' },
    ])
  })

  it('returns an empty list for blank input', () => {
    expect(splitSpeechSegments('')).toEqual([])
    expect(splitSpeechSegments('   \n\t\u3000')).toEqual([])
  })

  it('treats a stray closing quote as ordinary narration text', () => {
    // 半截数据/OCR 失败时可能只剩闭符：不能因为它把后半段吞成对话。
    expect(splitSpeechSegments('旁白”仍在旁白')).toEqual([
      { type: 'narration', text: '旁白”仍在旁白' },
    ])
  })

  it('handles a dialogue that runs to the end after an inner quote closes', () => {
    expect(splitSpeechSegments('“他说「走」')).toEqual([
      { type: 'dialogue', text: '“他说「走」' },
    ])
  })
})
