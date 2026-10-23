<template>
  <div class="rule-manager" :style="{ background: theme.popup, color: theme.fontColor }">
    <div class="rule-header">
      <h3>内容净化规则</h3>
      <div class="header-actions">
        <button class="add-btn" @click="openEditModal()">+ 新增规则</button>
        <button class="add-btn" :disabled="importing" @click="openImportPanel()">导入</button>
        <button class="close-btn" @click="handleClose">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="m15 18-6-6 6-6" />
          </svg>
        </button>
      </div>
    </div>

    <div class="rule-list">
      <div v-if="loading" class="loading">加载中...</div>
      <div v-else-if="!store.replaceRules.length" class="empty">
        暂无自定义规则
        <p>规则按“排序”从小到大依次执行</p>
      </div>
      <div
        v-else
        v-for="rule in sortedRules"
        :key="rule.id"
        class="rule-item"
        :class="{ disabled: !rule.isEnabled }"
      >
        <div class="rule-info">
          <div class="rule-name-row">
            <span class="rule-name">{{ rule.name }}</span>
            <span class="rule-badge" v-if="rule.isRegex">正则</span>
            <!-- Legado 规则是 Java 正则, 少数写法在 JS 里没有等价物。标出来,
                 免得用户以为「导入了却没效果」是阅读器的问题。 -->
            <span
              v-if="ruleIssues.get(rule.name)"
              class="rule-badge warn"
              :title="ruleIssues.get(rule.name)"
            >无法执行</span>
          </div>
          <div class="rule-scope">{{ describeScope(rule.scope) }}</div>
          <div class="rule-pattern">匹配: {{ rule.pattern }}</div>
          <div class="rule-replace">替换为: {{ rule.replacement || '(空)' }}</div>
        </div>
        <div class="rule-ops">
          <label class="switch">
            <input type="checkbox" v-model="rule.isEnabled" @change="toggleRule(rule)">
            <span class="slider"></span>
          </label>
          <button class="op-btn" @click="openEditModal(rule)">编辑</button>
          <button class="op-btn delete" @click="handleDelete(rule)">删除</button>
        </div>
      </div>
    </div>

    <!-- Edit Modal (Simplified for now as a sub-panel or use a real modal) -->
    <Transition name="fade">
      <div v-if="editingRule" class="edit-overlay" :style="{ background: theme.popup }">
        <div class="edit-header">
          <h4>{{ editingRule.id ? '编辑规则' : '新增规则' }}</h4>
        </div>
        <div class="edit-body">
          <div class="form-item">
            <label>名称</label>
            <input v-model="editingRule.name" placeholder="起个名字" />
          </div>
          <div class="form-item">
            <label>匹配项 (Pattern)</label>
            <textarea v-model="editingRule.pattern" placeholder="文本或正则表达式"></textarea>
          </div>
          <div class="form-item">
            <label>替换为 (Replacement)</label>
            <input v-model="editingRule.replacement" placeholder="留空则为删除" />
          </div>
          <div class="form-item">
            <label>生效范围</label>
            <div class="scope-group">
              <button type="button" class="scope-btn" :class="{ active: getScopeMode(editingRule) === 'global' }" @click="setScopeMode('global')">全部书籍</button>
              <!-- 「当前书籍 / 当前书源」需要阅读上下文。从主设置打开时没有打开中的书,
                   这两个按钮点下去只会静默无效, 因此直接不显示。 -->
              <template v-if="!props.standalone">
                <button type="button" class="scope-btn" :class="{ active: getScopeMode(editingRule) === 'book' }" @click="setScopeMode('book')">当前书籍</button>
                <button type="button" class="scope-btn" :class="{ active: getScopeMode(editingRule) === 'source' }" @click="setScopeMode('source')">当前书源</button>
              </template>
            </div>
            <div class="scope-preview">{{ describeScope(editingRule.scope) }}</div>
          </div>
          <div class="form-row">
            <label><input type="checkbox" v-model="editingRule.isRegex"> 正则模式</label>
            <label>排序: <input class="order-input" type="number" v-model="editingRule.order"></label>
          </div>
        </div>
        <div class="edit-footer">
          <button @click="editingRule = null">取消</button>
          <button class="primary" @click="handleSave">保存</button>
        </div>
      </div>
    </Transition>

    <!-- 导入面板：复用编辑面板的 .edit-overlay 层级与配色。
         入口只有两个：直链（前端 fetch）与本地文件（前端 JSON.parse）。 -->
    <Transition name="fade">
      <div v-if="importPanelOpen" class="edit-overlay" :style="{ background: theme.popup }">
        <div class="edit-header">
          <h4>导入净化规则</h4>
        </div>
        <div class="edit-body">
          <div class="form-item">
            <label>规则 JSON 链接</label>
            <input
              v-model="importUrl"
              type="url"
              placeholder="https://example.test/replace-rules.json"
              :disabled="importing"
              @keyup.enter="importFromUrl"
            >
          </div>
          <div class="import-hint">
            支持 Legado 净化规则的 JSON 数组格式。<code>replacement</code> 以
            <code>@js:</code> 开头的脚本规则会在 Rust 沙盒里执行；
            <code>pattern</code> 本身是脚本（用脚本生成匹配式）的规则没有等价能力，
            导入时会跳过并统计。
          </div>
          <div class="import-actions">
            <button class="op-btn" :disabled="importing" @click="importFromUrl">
              {{ importing ? '导入中…' : '从网络导入' }}
            </button>
            <button class="op-btn" :disabled="importing" @click="localFileInputRef?.click()">
              选择本地文件
            </button>
            <input
              ref="localFileInputRef"
              type="file"
              accept=".json,application/json"
              hidden
              @change="handleLocalFile"
            >
          </div>
          <div class="import-hint">
            本地文件直接在前端解析。导入按规则名合并，同名规则会被覆盖。
          </div>
        </div>
        <div class="edit-footer">
          <button :disabled="importing" @click="closeImportPanel">取消</button>
        </div>
      </div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useReaderStore } from '../../stores/reader'
import { useAppStore } from '../../stores/app'
import { saveReplaceRule, saveReplaceRules, deleteReplaceRule } from '../../api/replaceRule'
import {
  buildReplaceRuleImportMessage,
  normalizeLegadoReplaceRules,
} from '../../utils/replaceRuleImport'
import { compileReplaceRule } from '../../utils/replaceRuleRegex'
import type { ReplaceRule } from '../../types'

const store = useReaderStore()
const appStore = useAppStore()

/**
 * 独立模式：从主设置抽屉打开时用。
 *
 * 此时组件不在阅读页的面板栈里，`store.backPanel()` 无从回退，所以关闭动作改成
 * 向上抛事件；主题仍复用 `chromeTheme`（它只依赖全局主题与背景图配置，与是否
 * 正在阅读无关），因此观感与阅读页里一致。
 */
const props = withDefaults(defineProps<{ standalone?: boolean }>(), { standalone: false })
const emit = defineEmits<{ close: [] }>()

function handleClose() {
  if (props.standalone) {
    emit('close')
    return
  }
  store.backPanel()
}

const theme = computed(() => store.chromeTheme)
const loading = ref(false)
const editingRule = ref<ReplaceRule | null>(null)

/* ─── 导入面板 ─── */
const importPanelOpen = ref(false)
const importUrl = ref('')
const importing = ref(false)
const localFileInputRef = ref<HTMLInputElement | null>(null)

const sortedRules = computed(() => {
  return [...store.replaceRules].sort((a, b) => a.order - b.order)
})

/**
 * 规则名 → 无法执行的原因。
 *
 * 两个来源缺一不可：
 *  - 静态编译：规则本身写错（正则不支持、脚本语法错），一进列表就该标出来；
 *  - 运行时失败：`store.failedReplaceRules` 记录本次阅读里真正执行失败的规则
 *    （超时、脚本抛错、内存超限）。只做静态检查的话，「点了没反应」无从排查。
 */
const ruleIssues = computed(() => {
  const issues = new Map<string, string>(Object.entries(store.failedReplaceRules))
  for (const rule of store.replaceRules) {
    const compiled = compileReplaceRule(rule.pattern, rule.isRegex, rule.replacement)
    if (compiled.kind === 'error') issues.set(rule.name, compiled.error)
  }
  return issues
})

function describeScope(scope?: string) {
  if (!scope || scope === '*') return '范围: 全部书籍'
  if (scope.startsWith('source:')) {
    return `范围: 当前书源 (${scope.slice('source:'.length)})`
  }
  if (scope.startsWith('book:')) {
    return `范围: 当前书籍 (${scope.slice('book:'.length)})`
  }
  return `范围: ${scope}`
}

function getScopeMode(rule: ReplaceRule) {
  const scope = rule.scope || '*'
  if (scope.startsWith('source:')) return 'source'
  if (scope.startsWith('book:')) return 'book'
  return 'global'
}

onMounted(async () => {
  loading.value = true
  await store.fetchReplaceRules()
  loading.value = false
})

function openEditModal(rule?: ReplaceRule) {
  importPanelOpen.value = false
  if (rule) {
    editingRule.value = { ...rule }
  } else {
    editingRule.value = {
      id: 0,
      name: '',
      pattern: '',
      replacement: '',
      scope: '*',
      isEnabled: true,
      isRegex: false,
      order: store.replaceRules.length + 1
    }
  }
}

function setScopeMode(mode: 'global' | 'book' | 'source') {
  if (!editingRule.value) return
  if (mode === 'global') {
    editingRule.value.scope = '*'
    return
  }
  if (!store.book) return
  editingRule.value.scope = mode === 'source'
    ? `source:${store.book.origin}`
    : `book:${store.book.bookUrl}`
}

async function handleSave() {
  if (!editingRule.value) return
  if (!editingRule.value.name || !editingRule.value.pattern) {
    alert('请填写名称和匹配项')
    return
  }
  try {
    await saveReplaceRule(editingRule.value)
    await store.fetchReplaceRules()
    editingRule.value = null
  } catch (e: any) {
    alert(e.message)
  }
}

async function toggleRule(rule: ReplaceRule) {
  try {
    await saveReplaceRule(rule)
    await store.fetchReplaceRules()
  } catch (e: any) {
    alert(e.message)
    // saveReplaceRule 可能已在请求发出前就改了本地对象; 用服务端数据恢复真实状态,
    // 不能简单取反(旧实现失败时反而把用户刚点的开关翻回去)。
    const restored = await store.fetchReplaceRules()
    // 恢复也失败时本地开关已不可信, 必须明确告知 —— 否则界面停在一个
    // 既非"用户点的值"也非"服务端值"的状态上而无人察觉。
    if (!restored) alert('替换规则状态同步失败，请重新打开本面板确认')
  }
}

async function handleDelete(rule: ReplaceRule) {
  const ok = await appStore.confirmDialog(`确定删除规则 "${rule.name}" 吗？`, { title: '删除规则', danger: true })
  if (!ok) return
  try {
    await deleteReplaceRule(rule)
    await store.fetchReplaceRules()
  } catch (e: any) {
    alert(e.message)
  }
}

/* ─── 导入 ─── */

function openImportPanel() {
  // 两个面板都绝对定位铺满本面板, 同时开会互相盖住, 打开一个就关掉另一个。
  editingRule.value = null
  importUrl.value = ''
  importPanelOpen.value = true
}

function closeImportPanel() {
  if (importing.value) return
  importPanelOpen.value = false
}

/** 直链导入用前端 fetch，而不是后端 readRemoteSourceFile。
 *
 *  为什么不能走后端: 那条命令会把响应反序列化成 BookSource，净化规则的
 *  pattern/replacement 字段名对不上，会被整段丢掉，拿回来只剩空规则。
 *  为什么 WebView 里直连可行: 该站返回 `access-control-allow-origin: *`，
 *  且 tauri.conf.json 的 csp 是 null，不拦 fetch。
 *  跨域被拒时 fetch 只抛 `TypeError: Failed to fetch`，对用户毫无信息量，
 *  所以这里统一换成可读提示。 */
async function fetchRulesJson(url: string): Promise<unknown> {
  let response: Response
  try {
    response = await fetch(url, { cache: 'no-store' })
  } catch {
    throw new Error('无法从该地址获取规则：请检查网络连接，或该站点不允许跨域访问')
  }
  if (!response.ok) {
    throw new Error(`获取规则失败：服务器返回 ${response.status}`)
  }
  const text = await response.text()
  try {
    return JSON.parse(text)
  } catch {
    throw new Error('该地址返回的不是合法 JSON（可能被重定向到了网页）')
  }
}

/** 归一化 + 保存 + 提示，直链与本地文件两个入口共用。
 *  全部被跳过时只提示、不写入 —— 保存空数组会让人误以为导入成功了。 */
async function applyImportedRules(raw: unknown) {
  const result = normalizeLegadoReplaceRules(raw)
  const feedback = buildReplaceRuleImportMessage(result)
  if (!result.rules.length) {
    appStore.showToast(feedback.text, 'error')
    return
  }
  await saveReplaceRules(result.rules)
  const refreshed = await store.fetchReplaceRules()
  importPanelOpen.value = false
  // 保存成功但列表没拉回来时不能说"导入成功": 面板上看到的仍是旧列表,
  // 用户会以为规则没进去。明确提示需要重开面板确认。
  if (!refreshed) {
    appStore.showToast(`${feedback.text}，但规则列表刷新失败，请重新打开本面板确认`, 'warning')
    return
  }
  appStore.showToast(feedback.text, feedback.type)
}

async function importFromUrl() {
  if (importing.value) return
  const url = importUrl.value.trim()
  if (!url) {
    appStore.showToast('请输入规则 JSON 链接', 'warning')
    return
  }
  importing.value = true
  try {
    await applyImportedRules(await fetchRulesJson(url))
  } catch (error) {
    appStore.showToast((error as Error).message || '导入失败', 'error')
  } finally {
    importing.value = false
  }
}

async function handleLocalFile(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  importing.value = true
  try {
    let raw: unknown
    try {
      raw = JSON.parse(await file.text())
    } catch {
      throw new Error('文件内容不是合法 JSON')
    }
    await applyImportedRules(raw)
  } catch (error) {
    appStore.showToast((error as Error).message || '导入失败', 'error')
  } finally {
    importing.value = false
    // 清空 value: 否则再次选同一个文件不会触发 change
    input.value = ''
  }
}
</script>

<style scoped>
.rule-manager {
  display: flex;
  flex-direction: column;
  height: 100%;
  position: relative;
}

.rule-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 20px;
  border-bottom: 1px solid rgba(0,0,0,0.06);
  flex-shrink: 0;
}

.rule-header h3 { margin: 0; font-size: 16px; }

.header-actions { display: flex; gap: 12px; align-items: center; }

.add-btn {
  /* 阅读页: 淡主色底 + currentColor 文字(跟随阅读主题 fontColor)。
   * 不要混入 --color-text —— 那是应用主题文字色, 与阅读主题无关。 */
  background: color-mix(in srgb, var(--color-primary) 14%, transparent);
  color: currentColor;
  border: 1px solid color-mix(in srgb, var(--color-primary) 34%, transparent);
  padding: 6px 12px;
  border-radius: 6px;
  font-size: 13px;
  cursor: pointer;
}

.add-btn:hover {
  background: color-mix(in srgb, var(--color-primary) 20%, transparent);
}

.add-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.close-btn {
  width: 32px;
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 8px;
  color: inherit;
  opacity: 0.6;
  background: transparent;
  border: none;
  cursor: pointer;
}

.rule-list {
  flex: 1;
  overflow-y: auto;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  -webkit-overflow-scrolling: touch;
  overscroll-behavior: contain;
}

.loading, .empty {
  padding: 40px;
  text-align: center;
  opacity: 0.5;
  font-size: 14px;
}

.rule-item {
  padding: 12px;
  border-radius: 12px;
  background: rgba(0,0,0,0.03);
  border: 1px solid rgba(0,0,0,0.05);
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  transition: opacity 0.2s;
}

.rule-item.disabled { opacity: 0.5; }

.rule-info { flex: 1; min-width: 0; }

.rule-name-row { display: flex; align-items: center; gap: 8px; margin-bottom: 4px; }
.rule-name { font-weight: 600; font-size: 14px; }
/* 文字必须用 currentColor(跟随阅读主题 fontColor), 不能混 --color-primary。
 * 底色已由实心主色改为 16% 淡底, 文字若仍是主色就成了"同色压同色":
 * 实算默认主题 2.71:1、天蓝 2.39:1, 暗灰主题(#888 底)仅 1.10:1 —— 几乎不可见,
 * 比改动前的"实心底 + 白字"3.19:1 还差。currentColor 在各主题下为 6.77~10.73:1
 * (暗灰 3.04:1, 等于该主题纯文字的可读上限)。
 * 与上面 .add-btn 同一条规则: 阅读页文字不混 --color-text。 */
.rule-badge { font-size: 10px; background: color-mix(in srgb, var(--color-primary) 16%, transparent); color: currentColor; padding: 1px 4px; border-radius: 3px; }
/* 无法执行: 用告警色而不是主题色, 避免和「正则」标签混在一起看不见。 */
.rule-badge.warn {
  background: color-mix(in srgb, #d9534f 18%, transparent);
  color: #c9302c;
  cursor: help;
}
.rule-scope { font-size: 12px; color: var(--color-primary); opacity: 0.9; margin-bottom: 4px; }

.rule-pattern, .rule-replace {
  font-size: 12px;
  opacity: 0.6;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  margin-top: 2px;
}

.rule-ops { display: flex; flex-direction: column; align-items: flex-end; gap: 8px; }

.op-btn {
  font-size: 12px;
  background: transparent;
  border: 1px solid var(--color-border);
  padding: 2px 8px;
  border-radius: 4px;
  cursor: pointer;
  color: inherit;
}

.op-btn.delete { color: #ef4444; border-color: rgba(239, 68, 68, 0.2); }

/* Switch style —— 阅读页: 底用就地 color-mix(不能复用 soft 令牌, 原因见
 * .add-btn), 圆点用 currentColor 跟随阅读主题; 原来橙底白圆点在浅色阅读主题下
 * 圆点与底几乎同亮。 */
.switch { position: relative; display: inline-block; width: 34px; height: 18px; }
.switch input { opacity: 0; width: 0; height: 0; }
.slider { position: absolute; cursor: pointer; top: 0; left: 0; right: 0; bottom: 0; background-color: color-mix(in srgb, currentColor 18%, transparent); transition: .4s; border-radius: 18px; }
.slider:before { position: absolute; content: ""; height: 14px; width: 14px; left: 2px; bottom: 2px; background-color: currentColor; opacity: 0.55; transition: .4s; border-radius: 50%; }
input:checked + .slider { background-color: color-mix(in srgb, var(--color-primary) 26%, transparent); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--color-primary) 34%, transparent); }
input:checked + .slider:before { transform: translateX(16px); opacity: 1; }

/* Edit Overlay */
.edit-overlay {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  z-index: 10;
  display: flex;
  flex-direction: column;
  padding: 20px;
  overflow-y: auto;
  -webkit-overflow-scrolling: touch;
}

.edit-header h4 { margin: 0 0 16px 0; }

.edit-body { flex: 1; display: flex; flex-direction: column; gap: 16px; }

.form-item { display: flex; flex-direction: column; gap: 6px; }
.form-item label { font-size: 12px; opacity: 0.7; }
.form-item input, .form-item textarea {
  background: color-mix(in srgb, currentColor 7%, transparent);
  border: 1px solid color-mix(in srgb, currentColor 18%, transparent);
  padding: 8px 12px;
  border-radius: 8px;
  color: inherit;
  outline: none;
  font-size: 14px;
}
.form-item textarea { height: 80px; resize: none; }

.scope-group {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.scope-btn {
  border: 1px solid rgba(0,0,0,0.1);
  background: transparent;
  color: inherit;
  padding: 8px 12px;
  border-radius: 999px;
  cursor: pointer;
  font-size: 12px;
}

.scope-btn.active {
  background: color-mix(in srgb, var(--color-primary) 14%, transparent);
  color: currentColor;
  border-color: color-mix(in srgb, var(--color-primary) 34%, transparent);
}

.scope-preview {
  font-size: 12px;
  opacity: 0.65;
}

.form-row { display: flex; align-items: center; gap: 20px; font-size: 13px; }

/* 导入面板: 复用 .edit-overlay 的层级/配色, 只补它自己的排版。
 * .import-actions 里的按钮放大到和编辑面板按钮一致的点击面积。 */
.import-hint {
  font-size: 12px;
  opacity: 0.6;
  line-height: 1.65;
}

.import-hint code {
  font-size: 11px;
  padding: 1px 3px;
  border-radius: 3px;
  background: color-mix(in srgb, currentColor 10%, transparent);
}

.import-actions {
  display: flex;
  gap: 12px;
  flex-wrap: wrap;
}

.import-actions .op-btn {
  padding: 8px 14px;
  font-size: 13px;
  border-radius: 8px;
}

.import-actions .op-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.order-input {
  width: 64px;
  min-height: 32px;
  padding: 5px 8px;
  border: 1px solid color-mix(in srgb, currentColor 20%, transparent);
  border-radius: 8px;
  outline: none;
  background: color-mix(in srgb, currentColor 8%, transparent);
  color: inherit;
  color-scheme: inherit;
  font-variant-numeric: tabular-nums;
}

.order-input:focus {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 3px rgba(201, 127, 58, 0.16);
}

.edit-footer { display: flex; justify-content: flex-end; gap: 12px; margin-top: 20px; }
.edit-footer button { padding: 8px 20px; border-radius: 8px; border: 1px solid var(--color-border); background: transparent; color: inherit; cursor: pointer; }
.edit-footer button.primary { background: color-mix(in srgb, var(--color-primary) 14%, transparent); color: currentColor; border: 1px solid color-mix(in srgb, var(--color-primary) 34%, transparent); }
.edit-footer button.primary:hover { background: color-mix(in srgb, var(--color-primary) 20%, transparent); border-color: var(--color-primary); }
</style>
