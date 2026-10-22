import { describe, expect, it } from 'vitest'
import { hasSpeakableContent } from './speakableText'

describe('hasSpeakableContent', () => {
  it('rejects texts that only contain punctuation or quotes', () => {
    // 这些正是会拿到「服务端不返回音频」的输入：`……` 单独成段、双音色切分后只剩引号。
    expect(hasSpeakableContent('……')).toBe(false)
    expect(hasSpeakableContent('“”')).toBe(false)
    expect(hasSpeakableContent('，。')).toBe(false)
    expect(hasSpeakableContent('——')).toBe(false)
    // 全角空格、半角空白、换行都不算内容。
    expect(hasSpeakableContent('\u3000')).toBe(false)
    expect(hasSpeakableContent(' \t\n')).toBe(false)
    // 西文标点同样不算。
    expect(hasSpeakableContent('...')).toBe(false)
    expect(hasSpeakableContent('--- !?')).toBe(false)
  })

  it('accepts any text with a letter or number', () => {
    expect(hasSpeakableContent('嗯')).toBe(true)
    expect(hasSpeakableContent('123')).toBe(true)
    expect(hasSpeakableContent('abc')).toBe(true)
    expect(hasSpeakableContent('他说：“你好。”')).toBe(true)
    // 全角数字 / 字母混在标点里也算。
    expect(hasSpeakableContent('“３”')).toBe(true)
  })
})
