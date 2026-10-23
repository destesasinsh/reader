<template>
  <Teleport to="body">
    <Transition name="fade">
      <div v-if="appStore.confirmState" class="confirm-overlay" @click.self="cancel">
        <div class="confirm-dialog" :class="{ danger: appStore.confirmState.danger }">
          <h3 v-if="appStore.confirmState.title" class="confirm-title">{{ appStore.confirmState.title }}</h3>
          <p class="confirm-message">{{ appStore.confirmState.message }}</p>
          <div class="confirm-actions">
            <button class="confirm-btn" type="button" @click="cancel">取消</button>
            <button class="confirm-btn primary" type="button" @click="ok">确定</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
import { useAppStore } from '../stores/app'

const appStore = useAppStore()

function ok() {
  appStore.resolveConfirm(true)
}

function cancel() {
  appStore.resolveConfirm(false)
}
</script>

<style scoped>
.confirm-overlay {
  position: fixed;
  inset: 0;
  /* 确认框必须压在**任何**面板之上：它可能从主设置里打开的嵌套面板
     （--z-nested-overlay: 600）中触发，用 --z-modal+10 会被面板盖住、点不到。 */
  z-index: calc(var(--z-nested-overlay) + 10);
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.45);
  backdrop-filter: blur(2px);
  padding: 24px;
}

.confirm-dialog {
  width: min(360px, 100%);
  background: var(--color-bg-elevated);
  border-radius: var(--radius-lg);
  padding: 20px;
  box-shadow: var(--shadow-xl);
}

.confirm-title {
  margin: 0 0 8px;
  font-size: 15px;
  font-weight: 700;
}

.confirm-message {
  margin: 0 0 18px;
  font-size: 13px;
  line-height: 1.6;
  color: var(--color-text-secondary);
  word-break: break-all;
}

.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}

.confirm-btn {
  min-height: 34px;
  padding: 0 16px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--color-text-secondary);
  font-size: 13px;
  cursor: pointer;
}

.confirm-btn.primary {
  background: var(--color-primary-soft-bg);
  border-color: var(--color-primary-soft-border);
  color: var(--color-primary-soft-text);
}

.confirm-dialog.danger .confirm-btn.primary {
  /* 危险操作也要覆盖 color: 84-88 行把文字换成了主色系 soft-text, 若此处只换
   * 底色, 就会得到"红底 + 深棕字"(浅色 1.5:1 / 深色 2.5:1, 几乎不可见)。
   * 与主色按钮同风格: 淡红底 + 红字压深, 浅色 5.2:1 / 深色 4.5:1, 都过 AA。 */
  background: color-mix(in srgb, var(--color-danger) 12%, transparent);
  border-color: color-mix(in srgb, var(--color-danger) 34%, transparent);
  color: color-mix(in srgb, var(--color-danger) 75%, var(--color-text));
}

.confirm-btn:hover {
  background: var(--color-bg-hover);
}

.confirm-btn.primary:hover {
  background: var(--color-primary-soft-bg-hover);
  border-color: var(--color-primary);
}

.confirm-dialog.danger .confirm-btn.primary:hover {
  /* 不能沿用"纯红底 + opacity": 上面的文字本身就是红系, 红底压红字读不出来。 */
  background: color-mix(in srgb, var(--color-danger) 20%, transparent);
  border-color: var(--color-danger);
  /* hover 把文字里的正文色占比从 25% 提到 35%。底色压深后(20% 红底)原来的
   * 75% 红字在暗色主题只有 4.13:1, 不到 AA; 提到 65% 红/35% 正文色后
   * 浅色 5.40:1、暗色 4.73:1, 两个主题都过。常态(12% 底)仍用上面的 75% 配色,
   * 浅色 5.2:1 / 暗色 4.5:1 已达标, 不必动。 */
  color: color-mix(in srgb, var(--color-danger) 65%, var(--color-text));
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.15s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
