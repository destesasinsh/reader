import { describe, expect, it } from 'vitest'
import {
  compileReplaceRule,
  convertJavaRegexToJs,
  toJsReplacement,
  toSandboxRuleStep,
} from './replaceRuleRegex'

describe('convertJavaRegexToJs', () => {
  it('把开头的内联旗标提成 flags', () => {
    const { source, flags } = convertJavaRegexToJs('(?i)abc')
    expect(source).toBe('abc')
    expect(flags).toBe('gmi')
  })

  it('内联旗标写在中间也要提出来（Java 允许，JS 不支持）', () => {
    // 真实规则里就有 `...|(?m)<>|...` 这种写法
    const { source, flags } = convertJavaRegexToJs('^a$|(?m)<>|b')
    expect(source).toBe('^a$|<>|b')
    // 基础 flags 已是 gm，这里的 (?m) 不会重复追加
    expect(flags).toBe('gm')
  })

  it('同时出现多个内联旗标时合并去重', () => {
    const { flags } = convertJavaRegexToJs('(?m)(?i)(?s)x')
    expect(flags).toContain('m')
    expect(flags).toContain('i')
    expect(flags).toContain('s')
  })

  it('\\h 展开成水平空白字符类', () => {
    const { source } = convertJavaRegexToJs('a\\hb')
    expect(source).not.toContain('\\h')
    expect(source).toContain('\\t')
    // 能匹配普通空格与全角空格
    const re = new RegExp(source)
    expect(re.test('a b')).toBe(true)
    expect(re.test('a\u3000b')).toBe(true)
  })

  it('占有量词退化成贪婪量词（JS 没有占有量词）', () => {
    expect(convertJavaRegexToJs('a++').source).toBe('a+')
    expect(convertJavaRegexToJs('a*+').source).toBe('a*')
    expect(convertJavaRegexToJs('a{2,3}+').source).toBe('a{2,3}')
  })

  it('\\R 展开成任意换行', () => {
    const { source } = convertJavaRegexToJs('a\\Rb')
    const re = new RegExp(source)
    expect(re.test('a\nb')).toBe(true)
    expect(re.test('a\r\nb')).toBe(true)
  })

  it('作用域内旗标 (?i:...) 保持原样', () => {
    const { source } = convertJavaRegexToJs('(?i:abc)')
    expect(source).toBe('(?i:abc)')
  })
})

describe('toJsReplacement', () => {
  it('$0 转成 $&（Java 的整个匹配）', () => {
    expect(toJsReplacement('[$0]')).toBe('[$&]')
  })

  it('不误伤 $1..$9 与 $&', () => {
    expect(toJsReplacement('$1-$2')).toBe('$1-$2')
    expect(toJsReplacement('$&')).toBe('$&')
  })
})

describe('compileReplaceRule', () => {
  it('普通正则规则可直接执行', () => {
    const compiled = compileReplaceRule('广告', false, '')
    expect(compiled.kind).toBe('literal')
    const regex = compileReplaceRule('\\s+', true, ' ')
    expect(regex.kind).toBe('regex')
    if (regex.kind === 'regex') {
      expect('a   b'.replace(regex.regex, regex.replacement)).toBe('a b')
    }
  })

  it('字面量规则的 $ 会被转义，保证原样插入', () => {
    const compiled = compileReplaceRule('价格', false, '$100')
    expect(compiled.kind).toBe('literal')
    if (compiled.kind === 'literal') {
      expect('价格'.replaceAll(compiled.needle, compiled.replacement)).toBe('$100')
    }
  })

  it('@js: 脚本替换编译成 script（交给后端沙盒，不在前端执行）', () => {
    const compiled = compileReplaceRule('(?i)广告', true, '@js:result.toUpperCase()')
    expect(compiled.kind).toBe('script')
    if (compiled.kind === 'script') {
      // 正则部分照样做 Java→JS 降级，脚本原文原样保留
      expect(compiled.source).toBe('广告')
      expect(compiled.flags).toBe('gmi')
      expect(compiled.code).toBe('result.toUpperCase()')
    }
  })

  it('真实 Legado 规则里的 Java 写法现在都能编译', () => {
    // 取自实测数据中先前会抛 "Invalid group" 的几类写法
    const patterns = [
      '(?mi)(?<=^|\\w[^\\n\\w]{0,3})(?!(?<=\\d)--\\d|)，?',
      '^\\s*null\\s*$|^[，、。：；！？…~]+\\n?|(?m)<>',
      '(?i)<(center|font)(?:\\h(?!src\\b)|\\S)+?</',
      '(?m)^[“「【][^\\n]{0,40}$',
    ]
    for (const pattern of patterns) {
      const compiled = compileReplaceRule(pattern, true, '')
      expect(compiled.kind, `${pattern} 应可编译`).toBe('regex')
    }
  })

  it('无法降级的写法如实报错，而不是假装成功', () => {
    const compiled = compileReplaceRule('(?P<name>x)', true, '')
    expect(compiled.kind).toBe('error')
  })
})

describe('toSandboxRuleStep', () => {
  it('正则规则转成后端可直接 new RegExp 的步骤', () => {
    const result = toSandboxRuleStep('(?i)ab', true, '[$0]')
    expect(result.ok).toBe(true)
    if (result.ok) {
      expect(result.step).toEqual({
        pattern: 'ab',
        flags: 'gmi',
        // $0 在 JS 里要变成 $&
        replacement: '[$&]',
        isScript: false,
      })
    }
  })

  it('字面量规则被转义成正则，保证后端与前端结果一致', () => {
    const result = toSandboxRuleStep('a.b', false, '$100')
    expect(result.ok).toBe(true)
    if (result.ok) {
      expect(result.step.pattern).toBe('a\\.b')
      expect(result.step.flags).toBe('g')
      // 字面量的 $ 必须转义，否则会被当成替换占位符
      expect(result.step.replacement).toBe('$$100')
    }
  })

  it('脚本规则的脚本原文去掉 @js: 前缀后传给后端', () => {
    const result = toSandboxRuleStep('[０-９]', true, '@js:R = result; R == "１" ? "1" : R')
    expect(result.ok).toBe(true)
    if (result.ok) {
      expect(result.step.isScript).toBe(true)
      expect(result.step.replacement).toBe('R = result; R == "１" ? "1" : R')
    }
  })

  it('无法降级的正则如实报错', () => {
    const result = toSandboxRuleStep('(?P<x>a)', true, '')
    expect(result.ok).toBe(false)
  })
})

describe('\\w 的 Unicode 展开', () => {
  it('汉字被视为单词字符（否则整段正文会被标点规则吃掉）', () => {
    // 真实规则里的形态：排除「非单词字符」中 4 个以上
    const { source } = convertJavaRegexToJs('^[^\\n\\w]{4,}$')
    const re = new RegExp(source, 'gm')
    expect(re.test('他站在山巅望着远方')).toBe(false)
    expect(re.test('————')).toBe(true)
  })

  it('嵌套在字符类里时只展开成范围，不产生嵌套方括号', () => {
    const { source } = convertJavaRegexToJs('[\\w·]+')
    expect(source.startsWith('[0-9A-Za-z_')).toBe(true)
    expect(source.endsWith('\\u00b7·]+') || source.endsWith('·]+')).toBe(true)
    const re = new RegExp(source)
    expect(re.test('中文·abc')).toBe(true)
  })

  it('字符类外的 \\w 包成独立字符类', () => {
    const { source } = convertJavaRegexToJs('\\w+')
    expect(source.startsWith('[0-9A-Za-z_')).toBe(true)
    expect(new RegExp(source).test('汉字abc')).toBe(true)
  })
})

describe('\\W 的 Unicode 展开', () => {
  it('汉字不算 \W（否则「整行都是符号」的规则会吃掉中文行）', () => {
    const { source } = convertJavaRegexToJs('(?<=\\n)\\W+$')
    const re = new RegExp(source, 'gm')
    expect(re.test('\n他站在山巅')).toBe(false)
    expect(re.test('\n————')).toBe(true)
  })

  it('字符类内部的 \\W 也展开成范围而非嵌套方括号', () => {
    const { source } = convertJavaRegexToJs('[\\W]')
    expect(source).not.toContain('\\W')
    expect(new RegExp(source).test('。')).toBe(true)
    expect(new RegExp(source).test('中')).toBe(false)
  })
})
