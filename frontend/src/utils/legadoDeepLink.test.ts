import { describe, expect, it } from 'vitest'
import { parseLegadoDeepLink } from './legadoDeepLink'

describe('parseLegadoDeepLink', () => {
  it('解析书源一键导入', () => {
    const parsed = parseLegadoDeepLink(
      'legado://import/bookSource?src=https://example.test/sources/a.json',
    )
    expect(parsed).toEqual({
      supported: true,
      kind: 'bookSource',
      src: 'https://example.test/sources/a.json',
    })
  })

  it('识别订阅源、净化规则、朗读引擎', () => {
    const cases: Array<[string, string]> = [
      ['rssSource', 'rssSource'],
      ['replaceRule', 'replaceRule'],
      ['httpTTS', 'httpTTS'],
      // 站点里写法不完全统一，别名也要认
      ['httpTTS', 'http-tts'],
      ['httpTTS', 'tts'],
    ]
    for (const [expected, raw] of cases) {
      const parsed = parseLegadoDeepLink(
        `legado://import/${raw}?src=https://example.test/x.json`,
      )
      expect(parsed.supported).toBe(true)
      if (parsed.supported) expect(parsed.kind).toBe(expected)
    }
  })

  it('src 被 URL 编码时能解出来', () => {
    const parsed = parseLegadoDeepLink(
      'legado://import/bookSource?src=https%3A%2F%2Fexample.test%2Fb.json',
    )
    expect(parsed.supported).toBe(true)
    if (parsed.supported) expect(parsed.src).toBe('https://example.test/b.json')
  })

  it('src 被二次编码时也能解出来', () => {
    const parsed = parseLegadoDeepLink(
      'legado://import/bookSource?src=https%253A%252F%252Fexample.test%252Fc.json',
    )
    if (parsed.supported) expect(parsed.src).toBe('https://example.test/c.json')
  })

  it('主题/排版等桌面版没有的类型报 unsupported 而不是崩', () => {
    const parsed = parseLegadoDeepLink(
      'legado://import/theme?src=https://example.test/theme.json',
    )
    expect(parsed.supported).toBe(false)
    if (!parsed.supported) {
      expect(parsed.kindName).toBe('theme')
      expect(parsed.src).toBe('https://example.test/theme.json')
    }
  })

  it('不是 legado 链接时抛错', () => {
    expect(() => parseLegadoDeepLink('https://example.test/x')).toThrow('legado://')
  })

  it('缺少 src 时抛错', () => {
    expect(() => parseLegadoDeepLink('legado://import/bookSource')).toThrow('src')
  })

  it('src 不是 http(s) 时抛错', () => {
    expect(() =>
      parseLegadoDeepLink('legado://import/bookSource?src=file:///etc/passwd'),
    ).toThrow('http')
  })

  it('缺少导入类型时抛错', () => {
    expect(() => parseLegadoDeepLink('legado://import/')).toThrow('导入类型')
  })

  it('非 import 动作抛错', () => {
    expect(() => parseLegadoDeepLink('legado://open/bookSource?src=https://e.test/a')).toThrow(
      '不支持',
    )
  })
})
