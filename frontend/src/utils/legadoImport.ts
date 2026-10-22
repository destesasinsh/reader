/**
 * `legado://` 一键导入的执行端。
 *
 * 解析在 `legadoDeepLink.ts`，这里只负责"按类型调到已有的导入通道"——书源、RSS、
 * 净化规则、朗读引擎各自的导入逻辑此前都已存在（应用内导入），深链只是多了一个
 * 触发入口，不应该再复制一套解析/保存代码。
 *
 * 返回值是给 toast 用的文案（含跳过条目数），失败时直接抛错由调用方展示。
 */

import { invokeEnvelope } from '../api/invoke'
import { readRemoteSourceFile, saveBookSources } from '../api/source'
import { readRemoteRssSourceFile, saveRssSources } from '../api/rss'
import { saveReplaceRules } from '../api/replaceRule'
import { readRemoteHttpTtsFile, saveHttpTtsList } from '../api/httpTts'
import {
  buildReplaceRuleImportMessage,
  normalizeLegadoReplaceRules,
} from './replaceRuleImport'
import {
  parseLegadoDeepLink,
  type LegadoImportKind,
  type ParsedLegadoDeepLink,
} from './legadoDeepLink'
import type { BookSource, RssSource } from '../types'

export interface LegadoImportResult {
  message: string
  /** 有跳过条目时用 warning，让用户注意到"不是全都进来了"。 */
  type: 'success' | 'warning'
}

/** 把 `Vec<String>`（每项是一段可 JSON.parse 的文本）摊平成对象数组。 */
function flattenJsonTexts(texts: string[]): unknown[] {
  return texts.flatMap((item) => {
    try {
      const value: unknown = JSON.parse(item)
      return Array.isArray(value) ? value : [value]
    } catch {
      return []
    }
  })
}

async function importBookSources(src: string): Promise<LegadoImportResult> {
  const texts = await readRemoteSourceFile(src)
  const sources = flattenJsonTexts(texts) as BookSource[]
  if (!sources.length) throw new Error('链接里的书源文件为空或格式不正确')
  await saveBookSources(sources)
  return { message: `已导入 ${sources.length} 个书源`, type: 'success' }
}

async function importRssSources(src: string): Promise<LegadoImportResult> {
  const texts = await readRemoteRssSourceFile(src)
  const sources = flattenJsonTexts(texts) as RssSource[]
  if (!sources.length) throw new Error('链接里的订阅源文件为空或格式不正确')
  await saveRssSources(sources)
  return { message: `已导入 ${sources.length} 个订阅源`, type: 'success' }
}

async function importReplaceRules(src: string): Promise<LegadoImportResult> {
  // 净化规则只在前端解析（后端没有对应实体），远程文本走通用命令拿，避免跨域。
  const text = await invokeEnvelope<string>('read_remote_text_file', { req: { url: src } })
  const result = normalizeLegadoReplaceRules(JSON.parse(text) as unknown)
  const built = buildReplaceRuleImportMessage(result)
  if (!result.rules.length) {
    // 全被跳过时不写库，直接按错误抛出，让用户看到具体原因而不是"导入 0 条"。
    throw new Error(built.text)
  }
  await saveReplaceRules(result.rules)
  return { message: built.text, type: built.type === 'success' ? 'success' : 'warning' }
}

async function importHttpTts(src: string): Promise<LegadoImportResult> {
  const texts = await readRemoteHttpTtsFile(src)
  const drafts = flattenJsonTexts(texts) as Parameters<typeof saveHttpTtsList>[0]
  const engines = Array.isArray(drafts) ? drafts : drafts.data
  if (!engines.length) throw new Error('链接里的朗读引擎文件为空或格式不正确')
  const saved = await saveHttpTtsList(engines)
  const skipped = saved?.skipped ?? 0
  return {
    message: skipped
      ? `已导入 ${saved?.saved ?? 0} 个朗读引擎，跳过 ${skipped} 条无效条目`
      : `已导入 ${saved?.saved ?? 0} 个朗读引擎`,
    type: skipped ? 'warning' : 'success',
  }
}

const IMPORTERS: Record<LegadoImportKind, (src: string) => Promise<LegadoImportResult>> = {
  bookSource: importBookSources,
  rssSource: importRssSources,
  replaceRule: importReplaceRules,
  httpTTS: importHttpTts,
}

/** 已经解析好的深链 → 执行导入。 */
export async function runLegadoImport(parsed: ParsedLegadoDeepLink): Promise<LegadoImportResult> {
  if (!parsed.supported) {
    throw new Error(
      `暂不支持导入「${parsed.kindName}」：桌面版目前能导入书源、订阅源、净化规则和朗读引擎`,
    )
  }
  return IMPORTERS[parsed.kind](parsed.src)
}

/** 深链原文 → 解析并执行。 */
export async function importFromLegadoDeepLink(rawUrl: string): Promise<LegadoImportResult> {
  return runLegadoImport(parseLegadoDeepLink(rawUrl))
}
