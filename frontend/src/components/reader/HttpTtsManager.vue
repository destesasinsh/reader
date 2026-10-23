<template>
  <!-- 必须 Teleport 到 body：这个面板从「阅读设置」抽屉里打开，而 .read-settings
       自己 overflow-y:auto —— position:fixed 的子元素仍会被祖先的 overflow 裁掉，
       在抽屉里只会露出贴着边缘的一条。 -->
  <Teleport to="body">
    <div class="http-tts-overlay" @click.self="close">
      <section
        class="http-tts-panel"
        :style="{ background: theme.popup, color: theme.fontColor }"
      >
        <header class="http-tts-header">
          <div class="http-tts-heading">
            <h3>在线朗读引擎</h3>
            <p class="http-tts-subtitle">
              兼容 Legado 的 httpTTS 规则：地址里的模板由后端展开，导入后就能在朗读里直接选用。
            </p>
          </div>
          <button class="http-tts-close" type="button" aria-label="关闭" @click="close">×</button>
        </header>

        <div class="http-tts-import">
          <div class="http-tts-import-row">
            <input
              v-model.trim="remoteUrl"
              class="http-tts-input"
              type="url"
              placeholder="引擎文件直链，例如 https://example.test/tts.json"
              @keyup.enter="importFromRemote"
            >
            <button
              type="button"
              class="http-tts-btn primary"
              :disabled="importing"
              @click="importFromRemote"
            >{{ importing ? '导入中…' : '从网络导入' }}</button>
          </div>
          <div class="http-tts-import-row">
            <button
              type="button"
              class="http-tts-btn"
              :disabled="importing"
              @click="fileInputRef?.click()"
            >从本地 JSON 导入</button>
            <span class="http-tts-import-note">
              支持 Legado 导出的引擎 JSON 数组；缺少名称或地址的条目会被跳过并计数。
            </span>
            <input
              ref="fileInputRef"
              class="http-tts-hidden-input"
              type="file"
              accept=".json,application/json"
              @change="importFromFile"
            >
          </div>
        </div>

        <div class="http-tts-list">
          <div v-if="loading" class="http-tts-empty">正在读取已导入的引擎…</div>
          <div v-else-if="!engines.length" class="http-tts-empty">还没有导入朗读引擎</div>
          <div
            v-else
            v-for="engine in engines"
            :key="engine.id"
            class="http-tts-item"
          >
            <div class="http-tts-item-main">
              <div class="http-tts-item-name">
                {{ engine.name }}
                <span v-if="engine.contentType" class="http-tts-badge">{{ engine.contentType }}</span>
              </div>
              <div class="http-tts-item-url" :title="engine.url">{{ engine.url }}</div>
            </div>
            <button type="button" class="http-tts-btn danger" @click="removeEngine(engine)">删除</button>
          </div>
        </div>
      </section>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useAppStore } from '../../stores/app'
import { useReaderStore } from '../../stores/reader'
import {
  deleteHttpTts,
  getHttpTtsList,
  readRemoteHttpTtsFile,
  saveHttpTtsList,
  type HttpTtsDraft,
} from '../../api/httpTts'
import type { HttpTts } from '../../types'

const store = useReaderStore()
const appStore = useAppStore()
// 颜色跟随阅读主题：这个面板挂在阅读抽屉之上，用应用主题色会和正文对比突兀。
const theme = computed(() => store.chromeTheme)

const engines = ref<HttpTts[]>([])
const loading = ref(false)
const importing = ref(false)
const remoteUrl = ref('')
const fileInputRef = ref<HTMLInputElement | null>(null)

const emit = defineEmits<{ close: [] }>()

function close() {
  emit('close')
}

async function loadEngines() {
  loading.value = true
  try {
    engines.value = await getHttpTtsList()
  } catch (error) {
    appStore.showToast((error as Error).message || '读取引擎列表失败', 'error')
  } finally {
    loading.value = false
  }
}

/**
 * 把 JSON 解析结果摊平成待保存数组。
 *
 * 宽容是必须的：Legado 各站点的导出既有裸数组，也有 `{data:[...]}` 包装，
 * 单条对象也偶有出现。真正的字段校验（缺 name/url）交给后端，它会把
 * 跳过的条数通过 `skipped` 返回。
 */
function parseEnginePayload(raw: unknown): HttpTtsDraft[] {
  if (Array.isArray(raw)) return raw as HttpTtsDraft[]
  if (raw && typeof raw === 'object' && Array.isArray((raw as { data?: unknown }).data)) {
    return (raw as { data: HttpTtsDraft[] }).data
  }
  if (raw && typeof raw === 'object') return [raw as HttpTtsDraft]
  throw new Error('文件里没有可导入的朗读引擎（需要 JSON 数组）')
}

async function saveImported(drafts: HttpTtsDraft[]) {
  if (!drafts.length) throw new Error('没有解析到可导入的朗读引擎')
  const result = await saveHttpTtsList(drafts)
  const saved = result?.saved ?? 0
  const skipped = result?.skipped ?? 0
  // skipped 必须显式报出来：不提示的话用户会以为整份文件都导进来了。
  appStore.showToast(
    skipped
      ? `已导入 ${saved} 个引擎，跳过 ${skipped} 条无效条目（缺少名称/地址或格式错误）`
      : `已导入 ${saved} 个引擎`,
    skipped ? 'warning' : 'success',
  )
  await loadEngines()
}

async function importFromRemote() {
  const url = remoteUrl.value.trim()
  if (!url) {
    appStore.showToast('请输入引擎文件直链', 'warning')
    return
  }
  if (importing.value) return
  importing.value = true
  try {
    // 后端返回 vec[0] = 整份文件文本，与书源订阅同形态，故逐段 JSON.parse。
    const raw = await readRemoteHttpTtsFile(url)
    const drafts = raw.flatMap((item) => {
      try {
        return parseEnginePayload(JSON.parse(item))
      } catch {
        return []
      }
    })
    await saveImported(drafts)
    remoteUrl.value = ''
  } catch (error) {
    appStore.showToast((error as Error).message || '从直链导入失败', 'error')
  } finally {
    importing.value = false
  }
}

async function importFromFile(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  importing.value = true
  try {
    // 本地文件在前端解析（后端不必多一条读文件命令）；解析失败要给出可读提示，
    // 否则用户只会看到"没反应"。
    const drafts = parseEnginePayload(JSON.parse(await file.text()))
    await saveImported(drafts)
  } catch (error) {
    appStore.showToast((error as Error).message || '读取本地文件失败', 'error')
  } finally {
    input.value = ''
    importing.value = false
  }
}

async function removeEngine(engine: HttpTts) {
  const ok = await appStore.confirmDialog(`确定删除朗读引擎 "${engine.name}" 吗？`, {
    title: '删除朗读引擎',
    danger: true,
  })
  if (!ok) return
  try {
    // 只传 id：后端「id 命中或 name 命中」都会删，同时传会连带删掉同名兄弟条目。
    await deleteHttpTts({ id: engine.id })
    appStore.showToast('已删除朗读引擎', 'success')
    await loadEngines()
  } catch (error) {
    appStore.showToast((error as Error).message || '删除失败', 'error')
  }
}

onMounted(loadEngines)
</script>

<style scoped>
.http-tts-overlay {
  position: fixed;
  inset: 0;
  /* 两个入口：阅读页的抽屉（z 40–90）、主设置里的「阅读增强」面板
     （--z-nested-overlay: 600）。必须压过两者，又在toast(700)之下 ——
     否则「已删除」这类操作提示会被面板挡住。
     删除确认框是 --z-nested-overlay+10(610)，也会盖住本面板。 */
  z-index: var(--z-nested-overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  background: rgba(15, 23, 42, 0.45);
  backdrop-filter: blur(6px);
}

.http-tts-panel {
  width: min(720px, 100%);
  max-height: min(86vh, 760px);
  display: flex;
  flex-direction: column;
  border-radius: 16px;
  box-shadow: 0 24px 60px rgba(0, 0, 0, 0.28);
  overflow: hidden;
}

.http-tts-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  padding: 18px 20px 14px;
  border-bottom: 1px solid color-mix(in srgb, currentColor 12%, transparent);
  flex-shrink: 0;
}

.http-tts-heading h3 {
  margin: 0;
  font-size: 16px;
}

.http-tts-subtitle {
  margin: 6px 0 0;
  font-size: 12px;
  line-height: 1.5;
  opacity: 0.65;
}

.http-tts-close {
  width: 32px;
  height: 32px;
  flex-shrink: 0;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: inherit;
  font-size: 20px;
  line-height: 1;
  opacity: 0.6;
  cursor: pointer;
}

.http-tts-close:hover {
  opacity: 1;
  background: color-mix(in srgb, currentColor 8%, transparent);
}

.http-tts-import {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px 20px;
  border-bottom: 1px solid color-mix(in srgb, currentColor 12%, transparent);
  flex-shrink: 0;
}

.http-tts-import-row {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.http-tts-import-note {
  font-size: 12px;
  line-height: 1.5;
  opacity: 0.6;
}

.http-tts-input {
  flex: 1;
  min-width: 200px;
  padding: 9px 12px;
  border-radius: 10px;
  border: 1px solid color-mix(in srgb, currentColor 18%, transparent);
  background: color-mix(in srgb, currentColor 6%, transparent);
  color: inherit;
  outline: none;
  font-size: 13px;
}

.http-tts-input::placeholder {
  color: currentColor;
  opacity: 0.45;
}

.http-tts-input:focus {
  border-color: color-mix(in srgb, currentColor 34%, transparent);
  box-shadow: 0 0 0 3px rgba(201, 127, 58, 0.16);
}

.http-tts-btn {
  padding: 8px 14px;
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, currentColor 22%, transparent);
  background: transparent;
  color: inherit;
  font-size: 13px;
  cursor: pointer;
}

.http-tts-btn:hover:not(:disabled) {
  background: color-mix(in srgb, currentColor 8%, transparent);
}

.http-tts-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.http-tts-btn.primary {
  background: color-mix(in srgb, var(--color-primary, #c97f3a) 16%, transparent);
  border-color: color-mix(in srgb, var(--color-primary, #c97f3a) 38%, transparent);
}

.http-tts-btn.danger {
  color: #ef4444;
  border-color: rgba(239, 68, 68, 0.28);
}

.http-tts-hidden-input {
  display: none;
}

.http-tts-list {
  flex: 1;
  min-height: 120px;
  overflow-y: auto;
  padding: 12px 20px 18px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  -webkit-overflow-scrolling: touch;
  overscroll-behavior: contain;
}

.http-tts-empty {
  padding: 36px 0;
  text-align: center;
  font-size: 13px;
  opacity: 0.55;
}

.http-tts-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 12px;
  border-radius: 10px;
  background: color-mix(in srgb, currentColor 5%, transparent);
  border: 1px solid color-mix(in srgb, currentColor 10%, transparent);
}

.http-tts-item-main {
  flex: 1;
  min-width: 0;
}

.http-tts-item-name {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 600;
}

.http-tts-badge {
  font-size: 10px;
  font-weight: 400;
  padding: 1px 6px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--color-primary, #c97f3a) 16%, transparent);
}

.http-tts-item-url {
  margin-top: 4px;
  font-size: 12px;
  opacity: 0.55;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
