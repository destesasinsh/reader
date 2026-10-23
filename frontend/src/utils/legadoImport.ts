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
import { getBookSources, readRemoteSourceFile, saveBookSources } from '../api/source'
import { getRssSources, readRemoteRssSourceFile, saveRssSources } from '../api/rss'
import { saveReplaceRules, getReplaceRules } from '../api/replaceRule'
import { readRemoteHttpTtsFile, saveHttpTtsList, getHttpTtsList } from '../api/httpTts'
import { useAppStore } from '../stores/app'
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

/**
 * 导入前的确认。
 *
 * 为什么要确认：`legado://` 是注册到系统的协议，**任何网页**都能放一条链接把用户
 * 引到这里；而保存是按 URL upsert 的，同 URL 的已有条目会被静默覆盖。不给用户看
 * 一眼「要导入什么、多少条、可能覆盖多少条」就落盘，等于把一次点击变成不可撤销的
 * 数据变更。
 */
async function confirmImport(
  kindName: string,
  incoming: number,
  overwriting: number,
): Promise<boolean> {
  const appStore = useAppStore()
  const detail = overwriting > 0
    ? `\n其中 ${overwriting} 条与现有条目同地址，继续会覆盖它们的配置。`
    : ''
  return appStore.confirmDialog(
    `即将导入 ${incoming} 条${kindName}。${detail}\n来源：外部链接（legado://）`,
    { title: '确认导入' },
  )
}

async function importBookSources(src: string): Promise<LegadoImportResult> {
  const texts = await readRemoteSourceFile(src)
  const sources = flattenJsonTexts(texts) as BookSource[]
  if (!sources.length) throw new Error('链接里的书源文件为空或格式不正确')

  const existing = new Set((await getBookSources()).map((source) => source.bookSourceUrl))
  const overwriting = sources.filter((source) => existing.has(source.bookSourceUrl)).length
  if (!(await confirmImport('书源', sources.length, overwriting))) {
    throw new Error('已取消导入')
  }

  const saved = await saveBookSources(sources)
  // 后端是"尽力写入"：它返回实际落盘条数，而不是我们送进去的条数。
  const count = saved?.count ?? sources.length
  return { message: `已导入 ${count} 个书源`, type: count < sources.length ? 'warning' : 'success' }
}

async function importRssSources(src: string): Promise<LegadoImportResult> {
  const texts = await readRemoteRssSourceFile(src)
  const sources = flattenJsonTexts(texts) as RssSource[]
  if (!sources.length) throw new Error('链接里的订阅源文件为空或格式不正确')

  const existing = new Set((await getRssSources()).map((source) => source.sourceUrl))
  const overwriting = sources.filter((source) => existing.has(source.sourceUrl)).length
  if (!(await confirmImport('订阅源', sources.length, overwriting))) {
    throw new Error('已取消导入')
  }

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
  // 规则按 name upsert，同名会被覆盖；这里也算一下会覆盖多少条。
  const existing = new Set((await getReplaceRules()).map((rule) => rule.name))
  const overwriting = result.rules.filter((rule) => existing.has(rule.name)).length
  if (!(await confirmImport('净化规则', result.rules.length, overwriting))) {
    throw new Error('已取消导入')
  }
  await saveReplaceRules(result.rules)
  return { message: built.text, type: built.type === 'success' ? 'success' : 'warning' }
}

async function importHttpTts(src: string): Promise<LegadoImportResult> {
  const texts = await readRemoteHttpTtsFile(src)
  // `flattenJsonTexts` 已经摊平成数组，不再有 `{ data }` 信封要拆。
  const engines = flattenJsonTexts(texts) as Parameters<typeof saveHttpTtsList>[0]
  if (!Array.isArray(engines) || !engines.length) {
    throw new Error('链接里的朗读引擎文件为空或格式不正确')
  }
  const existing = new Set((await getHttpTtsList()).map((engine) => engine.name))
  const overwriting = engines.filter(
    (engine) => Boolean(engine.name) && existing.has(engine.name as string),
  ).length
  if (!(await confirmImport('朗读引擎', engines.length, overwriting))) {
    throw new Error('已取消导入')
  }
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
