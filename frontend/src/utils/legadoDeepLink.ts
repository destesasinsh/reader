/**
 * `legado://` 深链解析。
 *
 * 书源站点的「一键导入」按钮指向 `legado://import/<kind>?src=<url>`，这正是手机版
 * Legado 注册的协议。桌面版此前没接这一层，点了自然没反应；现在安装包会注册该
 * 协议，系统把链接作为启动参数交给本应用，这里负责把它翻译成"要导入什么"。
 *
 * 抽成纯函数的原因：协议解析的边界情况（大小写、URL 编码、非法 src、未知 kind）
 * 都能单测覆盖，不需要真的去点网页按钮。
 */

/** 桌面版能真正消化的导入类型。 */
export type LegadoImportKind = 'bookSource' | 'rssSource' | 'replaceRule' | 'httpTTS'

export type ParsedLegadoDeepLink =
  | { supported: true; kind: LegadoImportKind; src: string }
  /** 协议合法，但这个类型桌面版没有对应功能（主题、排版等）。 */
  | { supported: false; kindName: string; src: string | null }

/** Legado 的 kind 写法在不同站点不完全统一，做一次归一化。 */
const KIND_ALIASES: Record<string, LegadoImportKind> = {
  booksource: 'bookSource',
  rsssource: 'rssSource',
  replacerule: 'replaceRule',
  httptts: 'httpTTS',
  'http-tts': 'httpTTS',
  tts: 'httpTTS',
}

/**
 * 解析一条深链。格式不对时抛错（调用方直接 toast 出来），
 * 格式对但能力不支持时返回 `supported: false`。
 */
export function parseLegadoDeepLink(raw: string): ParsedLegadoDeepLink {
  const text = (raw || '').trim()
  if (!text) throw new Error('导入链接为空')

  const lower = text.toLowerCase()
  if (!lower.startsWith('legado://')) {
    throw new Error('不是 legado:// 链接')
  }

  // 不借助 `new URL`：自定义协议在部分环境里会被解析成 host='import' 之类，
  // 各浏览器/WebView 行为不一致；这里按固定结构手工切开更稳。
  const withoutScheme = text.slice('legado://'.length)
  const [pathPart, queryPart = ''] = splitOnce(withoutScheme, '?')
  const segments = pathPart.replace(/^\/+/, '').split('/').filter(Boolean)

  const action = (segments[0] || '').toLowerCase()
  const kindName = segments[1] || ''
  if (action !== 'import') {
    throw new Error(`不支持的链接类型：${action || '(空)'}`)
  }
  if (!kindName) {
    throw new Error('链接里没有指定导入类型')
  }

  const params = new URLSearchParams(queryPart)
  const rawSrc = params.get('src') || params.get('url') || ''
  // 有些站点把 src 做了两次编码，解到不能再解为止（最多两次，避免死循环）。
  const src = decodeRepeatedly(rawSrc)
  if (!src) {
    throw new Error('链接里没有 src 参数')
  }

  const normalizedKind = KIND_ALIASES[kindName.toLowerCase()]
  if (!normalizedKind) {
    return { supported: false, kindName, src }
  }
  if (!/^https?:\/\//i.test(src)) {
    throw new Error(`导入地址必须是 http(s)：${src}`)
  }
  return { supported: true, kind: normalizedKind, src }
}

function splitOnce(text: string, separator: string): [string, string] {
  const index = text.indexOf(separator)
  if (index < 0) return [text, '']
  return [text.slice(0, index), text.slice(index + separator.length)]
}

function decodeRepeatedly(value: string): string {
  let current = value.trim()
  for (let attempt = 0; attempt < 2; attempt += 1) {
    if (!/%[0-9a-f]{2}/i.test(current)) break
    try {
      const decoded = decodeURIComponent(current)
      if (decoded === current) break
      current = decoded
    } catch {
      // 非法百分号序列：保持原样，交给后面的 http(s) 校验去拒绝。
      break
    }
  }
  return current
}
