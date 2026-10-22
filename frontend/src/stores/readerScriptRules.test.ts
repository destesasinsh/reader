import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import type { Book } from '../types'
import { applyReplaceRulesInSandbox } from '../api/replaceRule'
import { useAppStore } from './app'
import { useReaderStore } from './reader'

vi.mock('../api/bookshelf', async (importOriginal) => ({
  ...await importOriginal<typeof import('../api/bookshelf')>(),
  getBookContent: vi.fn(),
  getBookInfo: vi.fn(),
  getChapterList: vi.fn(),
  saveBookProgress: vi.fn(),
}))

vi.mock('../api/replaceRule', async (importOriginal) => ({
  ...await importOriginal<typeof import('../api/replaceRule')>(),
  getReplaceRules: vi.fn().mockResolvedValue([]),
  applyReplaceRulesInSandbox: vi.fn(),
}))

const scriptRule = {
  id: 1,
  name: '全角转半角',
  // 只依赖 result 的脚本，与真实导入的 @js: 规则同形态
  pattern: '[０-９]',
  replacement: '@js:R = result; R == "１" ? "1" : R',
  isEnabled: true,
  isRegex: true,
  order: 0,
  scope: '*',
}

function setupReader(rules: typeof scriptRule[]) {
  const reader = useReaderStore()
  reader.book = {
    name: '脚本规则测试',
    author: '测试作者',
    origin: 'https://script.fixture.test',
    bookUrl: 'https://script.fixture.test/book',
  } as Book
  reader.content = '第１２３章'
  reader.replaceRules = rules
  return reader
}

describe('@js: 脚本规则走沙盒', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    const storage = new Map<string, string>()
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key),
    })
    useAppStore().setOnlineStatus(false)
    vi.mocked(applyReplaceRulesInSandbox).mockReset()
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('没有脚本规则时不发起任何 IPC（保持同步快路径）', async () => {
    const reader = setupReader([
      { ...scriptRule, name: '普通规则', replacement: '' },
    ])
    // 同步路径照常执行普通正则规则（这里是把全角数字删掉）
    expect(reader.displayContent).toBe('第章')
    await Promise.resolve()
    expect(applyReplaceRulesInSandbox).not.toHaveBeenCalled()
  })

  it('有脚本规则时把转换后的规则整批发给后端，并用返回值替换正文', async () => {
    vi.mocked(applyReplaceRulesInSandbox).mockResolvedValue('第123章')
    const reader = setupReader([scriptRule])

    // 沙盒结果到达前先显示同步结果（脚本规则暂不生效）
    expect(reader.displayContent).toBe('第１２３章')

    await vi.waitFor(() => expect(reader.displayContent).toBe('第123章'))
    expect(applyReplaceRulesInSandbox).toHaveBeenCalledTimes(1)
    const payload = vi.mocked(applyReplaceRulesInSandbox).mock.calls[0][0]
    expect(payload.text).toBe('第１２３章')
    expect(payload.rules[0]).toMatchObject({
      isScript: true,
      replacement: 'R = result; R == "１" ? "1" : R',
    })
  })

  it('脚本执行失败时不改变正文，并把原因挂到规则名上', async () => {
    vi.mocked(applyReplaceRulesInSandbox).mockRejectedValue(new Error('脚本死循环'))
    const reader = setupReader([scriptRule])

    await vi.waitFor(() => expect(reader.failedReplaceRules[scriptRule.name]).toBe('脚本死循环'))
    expect(reader.displayContent).toBe('第１２３章')
  })
})
