/**
 * Legado(阅读3.0，Java) 净化规则 → JavaScript 正则。
 *
 * 为什么需要这一层：Legado 的 `pattern` 是 **Java 正则**，而这里用 `new RegExp` 执行。
 * 两者语法大同小异，但实测导入的 16 条规则里有 **11 条直接构造失败** —— 全部是
 * Java 专有写法，`new RegExp` 会抛异常；旧实现用 try/catch 把它吞成一行
 * `console.error`，于是用户看到的只是"规则导入了但完全没效果"。
 *
 * 已处理的差异（都是实测数据里真实出现的）：
 * - **内联旗标** `(?i)` / `(?m)` / `(?s)` / `(?x)`：Java 允许写在任意位置；JS 不支持
 *   内联旗标，只能提成整体 flags。绝大多数规则把它写在开头，但也有写在中间的。
 * - **`\h`**：Java 的水平空白，JS 没有。展开成等价的字符类。
 * - **占有量词** `++` / `*+` / `?+` / `{n,m}+`：JS 不支持，退化成贪婪匹配。
 * - **`\R`**：Java 的任意换行，展开成 `(?:\r\n|[\r\n])`。
 *
 * 刻意**不**处理的东西（没有可靠等价物，宁可让编译器报错、由 UI 标成「无法执行」，
 * 也不静默改变语义）：`\p{...}`（需要 u 旗标，而 u 会让 `\-` 之类旧写法变语法错误）、
 * 原子组 `(?>...)`、`\G`。
 */

/** Java 专有、但可以安全展开成等价字符类的写法。 */
const HORIZONTAL_SPACE = '[ \\t\\u00a0\\u1680\\u2000-\\u200a\\u202f\\u205f\\u3000]'

/** 内联旗标组 `(?i)`、`(?ms)`。带冒号的 `(?i:...)` 是作用域内旗标，不动它。 */
const INLINE_FLAGS = /\(\?([imsx]+)\)/g

/** 占有量词 `++`、`*+`、`?+`、`{2,3}+`。 */
const POSSESSIVE_QUANTIFIER = /([+*?]|\{\d+(?:,\d*)?\})\+/g

/**
 * `\w` 的 Unicode 版本。
 *
 * 这是**必须**对齐的语义差异，不是可选的优化：Java 的 `\w` 默认是 ASCII，而中文规则
 * 几乎都按"`\w` 包含汉字"来写。实测规则 `#07 标点——` 里有一条
 * `[^\n\wꁘ“”【】（）…]{4,}`，本意是"匹配 4 个以上的标点/符号"；若 `\w` 只认 ASCII，
 * 汉字就落在被排除集合里，于是**整段中文正文都会命中**，被替换成「——」。
 * 展开成显式字符类后，行为与规则作者的意图一致（只吃标点，不动汉字）。
 */
const UNICODE_WORD = [
  '0-9A-Za-z_',
  '\\u00C0-\\u024F', // 拉丁扩展
  '\\u0370-\\u03FF', // 希腊
  '\\u0400-\\u04FF', // 西里尔
  '\\u3040-\\u30FF', // 平假名/片假名
  '\\u3400-\\u4DBF', // CJK 扩展 A
  '\\u4E00-\\u9FFF', // CJK 基本区
  '\\uF900-\\uFAFF', // CJK 兼容表意
  '\\uFF10-\\uFF19', // 全角数字
  '\\uFF21-\\uFF3A', // 全角大写
  '\\uFF41-\\uFF5A', // 全角小写
].join('')

/**
 * `\w` 的 Unicode 补集（BMP 范围内）。
 *
 * 为什么 `\W` 也必须展开：规则里常见 `(?<=\n)\W+$`（"整行都是符号"），若 `\W` 仍是
 * JS 默认的 ASCII 语义，汉字会被算作 `\W`，于是整段中文正文都满足条件被删掉 —— 实测
 * `#15 净化词语` 正是这样把正文清空的。
 */
const UNICODE_NON_WORD = [
  '\\u0000-\\u002F',
  '\\u003A-\\u0040',
  '\\u005B-\\u005E',
  '\\u0060',
  '\\u007B-\\u00BF',
  '\\u0250-\\u036F',
  '\\u0500-\\u303F',
  '\\u3100-\\u33FF',
  '\\u4DC0-\\u4DFF',
  '\\uA000-\\uF8FF',
  '\\uFB00-\\uFF0F',
  '\\uFF1A-\\uFF20',
  '\\uFF3B-\\uFF40',
  '\\uFF5B-\\uFFFF',
].join('')

/** 把 `\w` / `\W` 展开成 Unicode 字符类；字符类内部只展开成范围（不能嵌套方括号）。 */
function expandUnicodeWord(pattern: string): string {
  let output = ''
  let inClass = false
  for (let index = 0; index < pattern.length; index += 1) {
    const char = pattern[index]
    if (char === '\\') {
      const next = pattern[index + 1]
      if (next === 'w' || next === 'W') {
        const body = next === 'w' ? UNICODE_WORD : UNICODE_NON_WORD
        output += inClass ? body : `[${body}]`
        index += 1
        continue
      }
      // 其它转义原样保留（`\\w` 这种连续反斜杠也会走到这里，保持原意）。
      output += char + (next ?? '')
      index += 1
      continue
    }
    if (char === '[' && !inClass) inClass = true
    else if (char === ']' && inClass) inClass = false
    output += char
  }
  return output
}

export type CompiledReplaceRule =
  | { kind: 'regex'; regex: RegExp; replacement: string }
  | { kind: 'literal'; needle: string; replacement: string }
  /**
   * `@js:` 脚本替换：不能在前端执行（会拿到 DOM 与 IPC），要交给后端 rquickjs 沙盒。
   * 这里只做 Java→JS 正则转换，把可送后端的三元组准备好。
   */
  | { kind: 'script'; source: string; flags: string; code: string }
  | { kind: 'error'; error: string }

/**
 * 把一条规则编译成可执行形态。
 *
 * 基础 flags 保持 `gm`：老实现就是 `new RegExp(pattern, 'gm')`，换成别的会让用户
 * 现有规则（尤其依赖 `^`/`$` 按行锚定的）行为突变。这里只在此基础上追加规则自己
 * 声明的 `i` / `s`。
 */
export function compileReplaceRule(
  pattern: string,
  isRegex: boolean,
  replacement: string,
): CompiledReplaceRule {
  // `@js:` 是 Legado 的脚本替换：脚本以 `result` 为输入、返回值为替换结果。
  // 桌面版在 Rust 沙盒里执行（见 src/parser/replace_rule_js.rs），这里只负责把
  // 正则部分降级成 JS 语法，脚本原文保持不动。
  if (/^@js:/i.test(replacement.trim())) {
    const { source, flags } = convertJavaRegexToJs(pattern)
    return {
      kind: 'script',
      source,
      flags,
      code: replacement.trim().slice('@js:'.length),
    }
  }

  if (!isRegex) {
    // 字面量替换：`String.replaceAll(str, str)` 仍会解释 `$&`/`$1`，而"字面量"语义
    // 要求原样插入，所以把 `$` 转义掉。
    // 注意必须用**函数形式**：`replaceAll('$', '$$')` 里的 `$$` 自身会被当成转义，
    // 等于什么都没做（实测结果仍是 `$100`）。
    return {
      kind: 'literal',
      needle: pattern,
      replacement: replacement.replaceAll('$', () => '$$'),
    }
  }

  const { source, flags } = convertJavaRegexToJs(pattern)
  try {
    return {
      kind: 'regex',
      regex: new RegExp(source, flags),
      replacement: toJsReplacement(replacement),
    }
  } catch (error) {
    return {
      kind: 'error',
      error: (error as Error).message.replace(/^Invalid regular expression:\s*/, ''),
    }
  }
}

/** 拆出 Java 正则里的内联旗标并做语法降级。导出以便单测直接断言转换结果。 */
export function convertJavaRegexToJs(pattern: string): { source: string; flags: string } {
  let flags = 'gm'
  let source = pattern.replace(INLINE_FLAGS, (_match, letters: string) => {
    for (const letter of letters) {
      // x(verbose) 在 JS 没有等价物：只能丢弃，否则整条规则直接不可用。
      if ('is'.includes(letter) && !flags.includes(letter)) flags += letter
    }
    return ''
  })

  source = source.replaceAll('\\h', HORIZONTAL_SPACE)
  source = source.replace(POSSESSIVE_QUANTIFIER, '$1')
  source = source.replaceAll('\\R', '(?:\\r\\n|[\\r\\n])')
  source = expandUnicodeWord(source)

  return { source, flags }
}

/**
 * Java 的 `$0` 是「整个匹配」，JS 里对应 `$&`；不转的话 `$0` 会作为字面量插进正文。
 *
 * 替换串里要产出字面量 `$&`，必须写成 `$$&`（`$$` 转义成 `$`）。
 */
export function toJsReplacement(replacement: string): string {
  return replacement.replace(/\$0(?![0-9])/g, '$$&')
}

/** 送后端沙盒的一步：字符串替换在沙盒里也走 `String.replace`，所以统一用正则表达。 */
export interface SandboxRuleStep {
  pattern: string
  flags: string
  replacement: string
  isScript: boolean
}

export type SandboxStepResult = { ok: true; step: SandboxRuleStep } | { ok: false; error: string }

/** 把字面量串转成等价正则源，让后端可以统一用 `RegExp` 处理。 */
function escapeRegExpSource(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/**
 * 把一条规则编译成"可以整批发给 Rust 沙盒"的形式。
 *
 * 之所以不直接复用 `compileReplaceRule` 的结果：后端需要的是 `RegExp` 能直接吃的
 * pattern/flags，而前端 `literal` 分支返回的是原始 needle（`replaceAll` 用）。这里把
 * 字面量转义成等价正则，保证两条路径对同一条规则的处理结果一致。
 */
export function toSandboxRuleStep(
  pattern: string,
  isRegex: boolean,
  replacement: string,
): SandboxStepResult {
  if (/^@js:/i.test(replacement.trim())) {
    const { source, flags } = convertJavaRegexToJs(pattern)
    try {
      new RegExp(source, flags)
    } catch (error) {
      return { ok: false, error: (error as Error).message }
    }
    return {
      ok: true,
      step: {
        pattern: source,
        flags,
        replacement: replacement.trim().slice('@js:'.length),
        isScript: true,
      },
    }
  }

  if (!isRegex) {
    return {
      ok: true,
      step: {
        pattern: escapeRegExpSource(pattern),
        flags: 'g',
        // 同 `compileReplaceRule`：必须用函数形式转义，否则 `$$` 会被再解释一次。
        replacement: replacement.replaceAll('$', () => '$$'),
        isScript: false,
      },
    }
  }

  const { source, flags } = convertJavaRegexToJs(pattern)
  try {
    new RegExp(source, flags)
  } catch (error) {
    return { ok: false, error: (error as Error).message }
  }
  return {
    ok: true,
    step: { pattern: source, flags, replacement: toJsReplacement(replacement), isScript: false },
  }
}
