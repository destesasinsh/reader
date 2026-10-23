<template>
  <div
    class="read-settings"
    :class="{ 'is-dark': isDarkSettings }"
    :style="{
      background: theme.popup,
      color: theme.fontColor,
      '--settings-popup': theme.popup,
      '--settings-font': theme.fontColor,
    }"
  >
    <div class="settings-header">
      <h3 class="settings-title">设置</h3>
      <button
        v-if="!standalone"
        class="reset-btn"
        title="重置排版参数；阅读配色与背景图会保留"
        @click="store.resetConfig()"
      >重置为默认配置</button>
      <!-- 独立模式从主设置抽屉打开, 需要一个自己的关闭入口。 -->
      <button v-else class="reset-btn" @click="emit('close')">关闭</button>
    </div>
    <div class="settings-sep"></div>

    <nav class="reader-settings-tabs" aria-label="阅读设置分类">
      <button
        v-for="tab in visibleTabs"
        :key="tab.value"
        type="button"
        :class="{ active: activeSettingsTab === tab.value }"
        @click="activeSettingsTab = tab.value"
      >{{ tab.label }}</button>
    </nav>

    <div class="settings-body">
      <section v-show="activeSettingsTab === 'display'" class="settings-group">
      <!-- 阅读主题 -->
      <div class="setting-row">
        <label>阅读主题</label>
        <div class="theme-swatches">
          <button
            v-for="(t, i) in themePresets"
            :key="i"
            class="swatch"
            :class="{ active: isThemeActive(i) }"
            :style="{ background: t.body }"
            @click="store.setThemeIndex(i)"
          >
            <svg v-if="isThemeActive(i)" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3"><path d="M20 6 9 17l-5-5" /></svg>
          </button>
        </div>
      </div>

      <!-- 自定义阅读配色 -->
      <div class="setting-block">
        <div class="setting-block-head">
          <label>阅读配色</label>
          <button
            v-if="config.readerBackgroundColor || config.fontColor"
            class="link-btn"
            @click="resetReaderColors"
          >还原默认</button>
        </div>
        <div class="color-grid">
          <div class="color-card" :class="{ customized: config.readerBackgroundColor }">
            <span class="color-card-label">背景</span>
            <label class="color-swatch" :style="{ background: config.readerBackgroundColor || theme.body }">
              <input
                class="color-input"
                type="color"
                :value="config.readerBackgroundColor || theme.body"
                @input="store.updateConfig('readerBackgroundColor', ($event.target as HTMLInputElement).value)"
              >
            </label>
            <span class="color-card-value">{{ config.readerBackgroundColor || '跟随主题' }}</span>
            <button
              v-if="config.readerBackgroundColor"
              class="icon-btn"
              title="还原为跟随主题"
              @click="store.updateConfig('readerBackgroundColor', '')"
            >×</button>
          </div>
          <div class="color-card" :class="{ customized: config.fontColor }">
            <span class="color-card-label">文字</span>
            <label class="color-swatch" :style="{ background: config.fontColor || theme.fontColor }">
              <input
                class="color-input"
                type="color"
                :value="config.fontColor || theme.fontColor"
                @input="store.updateConfig('fontColor', ($event.target as HTMLInputElement).value)"
              >
            </label>
            <span class="color-card-value">{{ config.fontColor || '跟随主题' }}</span>
            <button
              v-if="config.fontColor"
              class="icon-btn"
              title="还原为跟随主题"
              @click="store.updateConfig('fontColor', '')"
            >×</button>
          </div>
        </div>
      </div>

      <!-- 阅读页背景图 -->
      <!--
        Web 端隐藏: 选图后要 `uploadBackgroundImage` 把文件落到应用目录,
        那是 Tauri 的 octet-stream 上传命令, 浏览器里没有等价路由
        (http.ts 的 upload 只认三种本地书)。留着按钮只会让人选完图报错。
      -->
      <div v-if="!appStore.isWebMode" class="setting-block">
        <div class="setting-block-head">
          <label>阅读页背景图</label>
          <button
            v-if="config.readerBackgroundImage"
            class="link-btn danger"
            @click="removeReaderBackgroundImage"
          >移除</button>
        </div>
        <div class="image-picker-row">
          <button
            class="image-thumb"
            :class="{ empty: !config.readerBackgroundImage }"
            type="button"
            :disabled="processingReaderImage"
            :style="config.readerBackgroundImage
              ? { backgroundImage: `url('${config.readerBackgroundImage}')` }
              : undefined"
            @click="readerImageInputRef?.click()"
          >
            <span v-if="!config.readerBackgroundImage">点击选择图片</span>
          </button>
          <div class="image-picker-meta">
            <button class="opt-btn" type="button" :disabled="processingReaderImage" @click="readerImageInputRef?.click()">
              {{ processingReaderImage ? '正在处理…' : (config.readerBackgroundImage ? '更换图片' : '选择图片') }}
            </button>
            <span class="image-picker-note">仅用于阅读页，与桌面背景图互不影响</span>
          </div>
          <input
            ref="readerImageInputRef"
            class="hidden-input"
            type="file"
            accept="image/jpeg,image/png,image/webp,image/bmp"
            hidden
            @change="handleReaderImageChange"
          >
        </div>
        <div v-if="config.readerBackgroundImage" class="inline-slider">
          <label for="reader-image-opacity">透明度</label>
          <input
            id="reader-image-opacity"
            class="settings-slider"
            type="range"
            min="0"
            max="1"
            :step="READER_BACKGROUND_OPACITY_STEP"
            :value="config.readerImageOpacity"
            @input="store.updateConfig('readerImageOpacity', Number(($event.target as HTMLInputElement).value))"
          >
          <label class="opacity-input-wrap" title="可直接输入 0-100 的数值">
            <input
              class="opacity-input"
              type="number"
              min="0"
              max="100"
              step="1"
              :value="readerOpacityToPercent(config.readerImageOpacity)"
              @change="applyReaderOpacityPercent(($event.target as HTMLInputElement).value, $event.target as HTMLInputElement)"
            >
            <span class="opacity-unit">%</span>
          </label>
        </div>
      </div>

      <!-- 面板不透明度: 目录/书签/设置抽屉与右键菜单的底板 -->
      <div class="setting-block">
        <div class="setting-block-head">
          <label for="reader-panel-opacity">面板不透明度</label>
          <button
            v-if="config.panelOpacity < 1"
            class="link-btn"
            @click="resetPanelOpacity"
          >还原实色</button>
        </div>
        <div class="inline-slider">
          <input
            id="reader-panel-opacity"
            class="settings-slider"
            type="range"
            :min="READER_PANEL_OPACITY_MIN"
            max="1"
            :step="READER_BACKGROUND_OPACITY_STEP"
            :value="config.panelOpacity"
            @input="store.updateConfig('panelOpacity', Number(($event.target as HTMLInputElement).value))"
          >
          <label class="opacity-input-wrap" title="可直接输入百分比数值">
            <input
              class="opacity-input"
              type="number"
              :min="Math.round(READER_PANEL_OPACITY_MIN * 100)"
              max="100"
              step="1"
              :value="readerOpacityToPercent(config.panelOpacity)"
              @change="applyPanelOpacityPercent(($event.target as HTMLInputElement).value, $event.target as HTMLInputElement)"
            >
            <span class="opacity-unit">%</span>
          </label>
        </div>
      </div>

      <!-- 正文字体 -->
      <div class="setting-row">
        <label>正文字体</label>
        <div class="font-picker">
          <div class="btn-group">
            <button
              v-for="f in fontPresets"
              :key="f.value"
              class="opt-btn"
              :class="{ active: config.fontFamily === f.value }"
              @click="store.updateConfig('fontFamily', f.value)"
            >{{ f.label }}</button>
            <button
              v-for="font in store.customFonts"
              :key="font.id"
              class="opt-btn custom-font-btn"
              :title="font.name"
              :class="{ active: config.fontFamily === `custom:${font.id}` }"
              @click="store.updateConfig('fontFamily', `custom:${font.id}`)"
            >{{ font.name }}</button>
          </div>
          <div class="font-actions">
            <!-- 导入/删除字体走 Tauri 上传通道, Web 端没有 HTTP 路由 -->
            <button v-if="!appStore.isWebMode" class="opt-btn" @click="fontInputRef?.click()">导入字体</button>
            <button
              v-if="activeCustomFont && !appStore.isWebMode"
              class="opt-btn danger"
              @click="removeActiveCustomFont"
            >删除当前字体</button>
            <input
              ref="fontInputRef"
              type="file"
              accept=".ttf,.otf,.woff,.woff2,font/ttf,font/otf,font/woff,font/woff2"
              hidden
              @change="handleFontFile"
            >
          </div>
        </div>
      </div>

      <!-- 简繁转换 -->
      <div class="setting-row">
        <label>简繁转换</label>
        <div class="btn-group">
          <button class="opt-btn" :class="{ active: config.chineseMode === 'off' }" @click="store.updateConfig('chineseMode', 'off')">关闭</button>
          <button class="opt-btn" :class="{ active: config.chineseMode === 'simplified' }" @click="store.updateConfig('chineseMode', 'simplified')">简体</button>
          <button class="opt-btn" :class="{ active: config.chineseMode === 'traditional' }" @click="store.updateConfig('chineseMode', 'traditional')">繁体</button>
        </div>
      </div>

      <!-- 预加载 -->
      <div class="setting-row">
        <label>&#x9884;&#x52A0;&#x8F7D;</label>
        <div class="btn-group">
          <button class="opt-btn" :class="{ active: config.enablePreload }" @click="store.updateConfig('enablePreload', true)">&#x5F00;&#x542F;</button>
          <button class="opt-btn" :class="{ active: !config.enablePreload }" @click="store.updateConfig('enablePreload', false)">&#x5173;&#x95ED;</button>
        </div>
      </div>
      <!-- 字体大小 -->
      <div class="setting-row slider-row">
        <div class="slider-head">
          <label>字体大小</label>
          <span class="slider-value">{{ config.fontSize }}</span>
        </div>
        <input
          type="range"
          class="settings-slider"
          :min="READER_FONT_SIZE_MIN"
          :max="READER_FONT_SIZE_MAX"
          step="1"
          :value="config.fontSize"
          @input="updateSlider('fontSize', $event)"
        >
      </div>

      <!-- 字体粗细 -->
      <div class="setting-row slider-row">
        <div class="slider-head">
          <label>字体粗细</label>
          <span class="slider-value">{{ config.fontWeight }}</span>
        </div>
        <input
          type="range"
          class="settings-slider"
          min="100"
          max="900"
          step="100"
          :value="config.fontWeight"
          @input="updateSlider('fontWeight', $event)"
        >
      </div>

      <!-- 段落行高 -->
      <div class="setting-row">
        <label>段落行高</label>
        <div class="stepper">
          <button class="step-btn" @click="stepFloat('lineHeight', -0.1, 1.0, 3.0)">—</button>
          <span class="step-val">{{ config.lineHeight.toFixed(1) }}</span>
          <button class="step-btn" @click="stepFloat('lineHeight', 0.1, 1.0, 3.0)">+</button>
        </div>
      </div>

      <!-- 段落间距 -->
      <div class="setting-row">
        <label>段落间距</label>
        <div class="stepper">
          <button class="step-btn" @click="stepFloat('paragraphSpacing', -0.1, 0, 2.0)">—</button>
          <span class="step-val">{{ config.paragraphSpacing.toFixed(1) }}</span>
          <button class="step-btn" @click="stepFloat('paragraphSpacing', 0.1, 0, 2.0)">+</button>
        </div>
      </div>

      <div class="setting-row">
        <label>首行缩进</label>
        <div class="btn-group">
          <button class="opt-btn" :class="{ active: config.firstLineIndent }" @click="store.updateConfig('firstLineIndent', true)">开启</button>
          <button class="opt-btn" :class="{ active: !config.firstLineIndent }" @click="store.updateConfig('firstLineIndent', false)">关闭</button>
        </div>
      </div>

      <!-- 页面宽度：左右分页时正文的行宽（窗口更宽时正文居中、两侧留白） -->
      <div class="setting-row slider-row">
        <div class="slider-head">
          <label>页面宽度</label>
          <span class="slider-value">{{ config.pageWidth }} px</span>
        </div>
        <input
          type="range"
          class="settings-slider"
          :min="READER_PAGE_WIDTH_MIN"
          :max="READER_PAGE_WIDTH_MAX"
          step="50"
          :value="config.pageWidth"
          @input="updateSlider('pageWidth', $event)"
        >
      </div>

      <!-- 左右分页时每页正文的上下留白 -->
      <div class="setting-row slider-row">
        <div class="slider-head">
          <label>左右页上下边距</label>
          <span class="slider-value">{{ config.pageVerticalPadding }} px</span>
        </div>
        <input
          type="range"
          class="settings-slider"
          min="0"
          :max="READER_PAGE_VERTICAL_PADDING_MAX"
          step="4"
          :value="config.pageVerticalPadding"
          @input="updateSlider('pageVerticalPadding', $event)"
        >
      </div>
      </section>

      <section v-show="activeSettingsTab === 'paging'" class="settings-group">

      <!-- 翻页方式 -->
      <div class="setting-row">
        <label>翻页方式</label>
        <div class="btn-group">
          <button class="opt-btn" :class="{ active: config.readMethod === '上下滑动' }" title="一次阅读一章内容，手动上下滚动" @click="store.updateConfig('readMethod', '上下滑动')">单章滚动</button>
          <button class="opt-btn" :class="{ active: config.readMethod === '左右翻页' }" title="将一章内容分成多个页面，左右切换" @click="store.updateConfig('readMethod', '左右翻页')">左右分页</button>
          <button class="opt-btn" :class="{ active: config.readMethod === '上下滚动' }" title="多章内容首尾相接，向下连续阅读" @click="store.updateConfig('readMethod', '上下滚动')">连续阅读</button>
          <button class="opt-btn" :class="{ active: config.readMethod === '上下滚动2' }" title="连续阅读，并自动隐藏已经读过的章节" @click="store.updateConfig('readMethod', '上下滚动2')">隐藏已读</button>
        </div>
      </div>

      <!-- 动画时长 -->
      <div class="setting-row slider-row">
        <div class="slider-head">
          <label>动画时长</label>
          <span class="slider-value">{{ config.animateDuration === 0 ? '关闭' : `${config.animateDuration} ms` }}</span>
        </div>
        <input
          type="range"
          class="settings-slider"
          min="0"
          max="1000"
          step="50"
          :value="config.animateDuration"
          @input="updateSlider('animateDuration', $event)"
        >
      </div>

      <!-- 自动阅读 -->
      <div class="setting-row">
        <label>自动阅读</label>
        <button
          class="opt-btn wide"
          :class="{ active: store.isAutoScrolling }"
          @click="store.isAutoScrolling = !store.isAutoScrolling"
        >
          {{ store.isAutoScrolling ? '停止滚动' : '开启自动平滑滚动' }}
        </button>
      </div>

      <!-- 滚动速度 -->
      <div class="setting-row slider-row">
        <div class="slider-head">
          <label>滚动速度</label>
          <span class="slider-value">{{ config.autoScrollSpeed }} px/秒</span>
        </div>
        <input
          type="range"
          class="settings-slider"
          min="0"
          max="100"
          step="1"
          :value="autoScrollSpeedSlider"
          @input="updateAutoScrollSpeedSlider"
        >
      </div>

      <!-- 点击翻页 (全屏热区) -->
      <div class="setting-row">
        <label>点击翻页</label>
        <div class="btn-group">
          <button class="opt-btn" :class="{ active: config.clickAction === 'auto' }" title="点击上方后退，点击下方前进，到章节边界时自动切换章节" @click="store.updateConfig('clickAction', 'auto')">自动翻页</button>
          <button class="opt-btn" :class="{ active: config.clickAction === 'next' }" title="点击正文上方或下方，都只向后阅读" @click="store.updateConfig('clickAction', 'next')">仅下滚</button>
          <button class="opt-btn" :class="{ active: config.clickAction === 'none' }" title="点击正文不执行翻页或滚动" @click="store.updateConfig('clickAction', 'none')">禁用</button>
        </div>
      </div>

      <!-- 选择文字 -->
      <div class="setting-row">
        <label>选择文字</label>
        <div class="btn-group">
          <button class="opt-btn" :class="{ active: config.selectAction === 'popup' }" @click="store.updateConfig('selectAction', 'popup')">操作弹窗</button>
          <button class="opt-btn" :class="{ active: config.selectAction === 'ignore' }" @click="store.updateConfig('selectAction', 'ignore')">忽略</button>
        </div>
      </div>
      </section>

      <section v-show="activeSettingsTab === 'speech'" class="settings-group">

      <!-- 朗读引擎 -->
      <div class="setting-row">
        <label>朗读引擎</label>
        <div class="btn-group">
          <button
            class="opt-btn"
            :class="{ active: store.speechConfig.provider === 'system' }"
            :disabled="!store.systemSpeechSupported"
            :title="store.systemSpeechSupported ? '使用系统语音' : '当前系统不支持系统语音，请改用 API 语音'"
            @click="store.setSpeechProvider('system')"
          >系统语音</button>
          <button class="opt-btn" :class="{ active: store.speechConfig.provider === 'openai' }" @click="store.setSpeechProvider('openai')">API 语音</button>
          <button
            class="opt-btn"
            :class="{ active: store.speechConfig.provider === 'http' }"
            :disabled="appStore.isWebMode"
            :title="appStore.isWebMode ? '在线朗读引擎走 Tauri 命令，Web 阅读模式没有对应接口' : '使用 Legado 在线朗读引擎（httpTTS）'"
            @click="store.setSpeechProvider('http')"
          >在线朗读引擎</button>
          <button
            class="opt-btn"
            :class="{ active: store.speechConfig.provider === 'edge' }"
            :disabled="appStore.isWebMode"
            :title="appStore.isWebMode ? 'Edge 朗读需要 WebSocket 直连，浏览器跨域会被拦' : '使用微软 Edge 在线朗读（无需 API Key）'"
            @click="store.setSpeechProvider('edge')"
          >Edge 朗读</button>
          <button
            class="opt-btn"
            :class="{ active: store.speechConfig.provider === 'msdemo' }"
            :disabled="appStore.isWebMode"
            :title="appStore.isWebMode ? '微软朗读走 Tauri 命令，Web 阅读模式没有对应接口' : '微软朗读（演示接口）：支持风格与更高音质，无需 API Key'"
            @click="store.setSpeechProvider('msdemo')"
          >微软朗读</button>
        </div>
      </div>

      <!-- 旁白/对话模式：先选模式再配音色（原来在页签末尾，用户实测找不到）。 -->
      <div class="setting-row">
        <label>旁白/对话</label>
        <div class="btn-group">
          <button
            class="opt-btn"
            :class="{ active: !store.speechConfig.dualVoiceEnabled }"
            @click="store.setDualVoiceEnabled(false)"
          >同音色</button>
          <button
            class="opt-btn"
            :class="{ active: store.speechConfig.dualVoiceEnabled }"
            title="按中文引号切分正文：旁白与对话分别用不同音色"
            @click="store.setDualVoiceEnabled(true)"
          >分音色</button>
        </div>
      </div>
      <p v-if="store.speechConfig.dualVoiceEnabled" class="setting-hint">
        对话指中文引号（“…” / 「…」 / 『…』）内的内容；引号未闭合时，其后一直按对话朗读。
      </p>

      <!-- 系统语音：整块包在 template 里，保证下面其它通道仍能与它组成
           v-if / v-else-if / v-else 链（中间插一个独立 v-if 会截断链，
           其它 provider 会落进最后一个 v-else 里渲染出错误的表单）。 -->
      <template v-if="store.speechConfig.provider === 'system'">
        <template v-if="store.speechConfig.dualVoiceEnabled">
          <div class="setting-block voice-partition">
            <div class="setting-block-head"><label>旁白</label></div>
            <select class="voice-select" :value="store.speechConfig.voiceName" @change="handleVoiceChange">
              <option value="">系统默认</option>
              <option v-for="voice in store.voiceList" :key="voice.name" :value="voice.name">
                {{ voice.name }} ({{ voice.lang }})
              </option>
            </select>
          </div>
          <div class="setting-block voice-partition">
            <div class="setting-block-head"><label>对话</label></div>
            <select
              class="voice-select"
              :value="store.speechConfig.dialogueVoiceName"
              @change="store.setDialogueVoiceName(($event.target as HTMLSelectElement).value)"
            >
              <option value="">跟随旁白音色</option>
              <option v-for="voice in store.voiceList" :key="voice.name" :value="voice.name">
                {{ voice.name }} ({{ voice.lang }})
              </option>
            </select>
          </div>
        </template>
        <div v-else class="setting-block">
          <div class="setting-block-head"><label>音色</label></div>
          <select class="voice-select" :value="store.speechConfig.voiceName" @change="handleVoiceChange">
            <option value="">系统默认</option>
            <option v-for="voice in store.voiceList" :key="voice.name" :value="voice.name">
              {{ voice.name }} ({{ voice.lang }})
            </option>
          </select>
        </div>
      </template>

      <!-- 注意不能写 v-else：第三方 provider 会把下面的 API 表单一起吃掉。
           在线朗读引擎没有 API 表单，必须显式排除。 -->
      <template v-else-if="store.speechConfig.provider === 'openai'">
        <!-- 音色放在最前：旁白/对话两个独立分区，后面才是共享的接口参数。 -->
        <template v-if="store.speechConfig.dualVoiceEnabled">
          <div class="setting-block voice-partition">
            <div class="setting-block-head"><label>旁白</label></div>
            <input
              class="voice-select"
              type="text"
              :value="store.speechConfig.openaiVoice"
              :placeholder="selectedSpeechApiFormat.voicePlaceholder"
              @input="store.setOpenAISpeechVoice(($event.target as HTMLInputElement).value)"
            >
          </div>
          <div class="setting-block voice-partition">
            <div class="setting-block-head"><label>对话</label></div>
            <input
              class="voice-select"
              type="text"
              :value="store.speechConfig.dialogueOpenaiVoice"
              :placeholder="`留空跟随旁白音色（${store.speechConfig.openaiVoice || '默认'}）`"
              @input="store.setDialogueOpenaiVoice(($event.target as HTMLInputElement).value)"
            >
          </div>
        </template>
        <div v-else class="setting-block">
          <div class="setting-block-head"><label>音色</label></div>
          <input
            class="voice-select"
            type="text"
            :value="store.speechConfig.openaiVoice"
            :placeholder="selectedSpeechApiFormat.voicePlaceholder"
            @input="store.setOpenAISpeechVoice(($event.target as HTMLInputElement).value)"
          >
        </div>

        <div class="setting-row setting-row-top">
          <label>接口格式</label>
          <select
            class="voice-select"
            :value="store.speechConfig.apiFormat"
            @change="store.setSpeechApiFormat(($event.target as HTMLSelectElement).value as SpeechApiFormat)"
          >
            <option v-for="option in speechApiFormatOptions" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
        </div>

        <div class="setting-row setting-row-top">
          <label>服务地址</label>
          <input
            class="voice-select"
            type="url"
            :value="store.speechConfig.openaiBaseUrl"
            :placeholder="selectedSpeechApiFormat.baseUrlPlaceholder"
            @input="store.setOpenAISpeechBaseUrl(($event.target as HTMLInputElement).value)"
          >
        </div>

        <div class="setting-row setting-row-top">
          <label>HTTP 代理</label>
          <input
            class="voice-select"
            type="url"
            :value="store.speechConfig.speechProxyUrl"
            placeholder="可选，例如 http://127.0.0.1:7890"
            @input="store.setSpeechProxyUrl(($event.target as HTMLInputElement).value)"
          >
        </div>

        <div class="setting-row setting-row-top">
          <label>访问密钥</label>
          <input
            class="voice-select"
            type="password"
            :value="store.speechConfig.openaiApiKey"
            placeholder="请输入服务密钥"
            autocomplete="off"
            @input="store.setOpenAISpeechApiKey(($event.target as HTMLInputElement).value)"
          >
        </div>

        <div class="setting-row setting-row-top">
          <label>{{ selectedSpeechApiFormat.modelLabel }}</label>
          <input
            class="voice-select"
            type="text"
            :value="store.speechConfig.openaiModel"
            :placeholder="selectedSpeechApiFormat.modelPlaceholder"
            @input="store.setOpenAISpeechModel(($event.target as HTMLInputElement).value)"
          >
        </div>

        <div class="setting-row setting-row-top">
          <label>音频格式</label>
          <select
            class="voice-select"
            :value="store.speechConfig.openaiFormat"
            @change="store.setOpenAISpeechFormat(($event.target as HTMLSelectElement).value as SpeechAudioFormat)"
          >
            <option v-for="format in selectedSpeechApiFormat.supportedFormats" :key="format" :value="format">
              {{ format }}
            </option>
          </select>
        </div>

        <div class="setting-row setting-row-top">
          <label>请求模式</label>
          <div class="btn-group">
            <button
              class="opt-btn"
              :class="{ active: store.speechConfig.openaiRequestMode === 'chunked' }"
              @click="store.setOpenAISpeechRequestMode('chunked')"
            >
              少字多请求
            </button>
            <button
              class="opt-btn"
              :class="{ active: store.speechConfig.openaiRequestMode === 'merged' }"
              @click="store.setOpenAISpeechRequestMode('merged')"
            >
              多字少请求
            </button>
          </div>
        </div>
      </template>

      <template v-else-if="store.speechConfig.provider === 'http'">
        <!-- 一个引擎就是一个音色：双音色时旁白/对话各选一个引擎（两个独立分区），
             引擎管理入口放在两块之后（共享操作）。 -->
        <template v-if="store.speechConfig.dualVoiceEnabled">
          <div class="setting-block voice-partition">
            <div class="setting-block-head"><label>旁白</label></div>
            <select
              class="voice-select"
              :value="selectedHttpTtsId"
              @change="handleHttpTtsChange"
            >
              <option value="">请选择在线朗读引擎</option>
              <option v-for="engine in httpTtsList" :key="engine.id" :value="String(engine.id)">
                {{ engine.name }}
              </option>
            </select>
          </div>
          <div class="setting-block voice-partition">
            <div class="setting-block-head"><label>对话</label></div>
            <select
              class="voice-select"
              :value="selectedDialogueHttpTtsId"
              @change="handleDialogueHttpTtsChange"
            >
              <option value="">跟随旁白引擎</option>
              <option v-for="engine in httpTtsList" :key="engine.id" :value="String(engine.id)">
                {{ engine.name }}
              </option>
            </select>
          </div>
        </template>
        <div v-else class="setting-block">
          <div class="setting-block-head"><label>音色</label></div>
          <div class="http-tts-picker">
            <select
              class="voice-select"
              :value="selectedHttpTtsId"
              @change="handleHttpTtsChange"
            >
              <option value="">请选择在线朗读引擎</option>
              <option v-for="engine in httpTtsList" :key="engine.id" :value="String(engine.id)">
                {{ engine.name }}
              </option>
            </select>
          </div>
        </div>
        <div class="setting-row">
          <label>引擎管理</label>
          <div class="btn-group">
            <button class="opt-btn" type="button" @click="showHttpTtsManager = true">管理引擎</button>
          </div>
        </div>
        <p v-if="!httpTtsList.length" class="setting-hint">
          {{ httpTtsLoading
            ? '正在读取已导入的引擎…'
            : '还没有导入在线朗读引擎。点「管理引擎」，可从直链或本地 JSON 文件导入。' }}
        </p>
      </template>

      <!-- Edge 朗读：音色来自微软公共接口（后端代理），默认只列中文音色；
           322 个音色全列出来会淹没选择，切「显示全部语言」才展开。 -->
      <template v-else-if="store.speechConfig.provider === 'edge'">
        <!-- 双音色：旁白/对话两个独立分区，各自选音色；语言筛选开关两块共用。 -->
        <template v-if="store.speechConfig.dualVoiceEnabled">
          <div class="setting-block voice-partition">
            <div class="setting-block-head">
              <label>旁白</label>
              <button class="link-btn" type="button" @click="showAllEdgeLocales = !showAllEdgeLocales">
                {{ showAllEdgeLocales ? '只看中文' : '显示全部语言' }}
              </button>
            </div>
            <select
              class="voice-select"
              :value="store.speechConfig.edgeVoice"
              @change="store.setEdgeVoice(($event.target as HTMLSelectElement).value)"
            >
              <option v-if="!edgeVoiceOptions.length" :value="store.speechConfig.edgeVoice">
                {{ store.speechConfig.edgeVoice }}
              </option>
              <option v-for="voice in edgeVoiceOptions" :key="voice.shortName" :value="voice.shortName">
                {{ edgeVoiceLabel(voice) }}
              </option>
            </select>
          </div>
          <div class="setting-block voice-partition">
            <div class="setting-block-head"><label>对话</label></div>
            <select
              class="voice-select"
              :value="store.speechConfig.edgeDialogueVoice"
              @change="store.setEdgeDialogueVoice(($event.target as HTMLSelectElement).value)"
            >
              <option value="">跟随旁白音色</option>
              <option v-for="voice in edgeVoiceOptions" :key="voice.shortName" :value="voice.shortName">
                {{ edgeVoiceLabel(voice) }}
              </option>
            </select>
          </div>
        </template>
        <div v-else class="setting-block">
          <div class="setting-block-head">
            <label>音色</label>
            <button class="link-btn" type="button" @click="showAllEdgeLocales = !showAllEdgeLocales">
              {{ showAllEdgeLocales ? '只看中文' : '显示全部语言' }}
            </button>
          </div>
          <select
            class="voice-select"
            :value="store.speechConfig.edgeVoice"
            @change="store.setEdgeVoice(($event.target as HTMLSelectElement).value)"
          >
            <option v-if="!edgeVoiceOptions.length" :value="store.speechConfig.edgeVoice">
              {{ store.speechConfig.edgeVoice }}
            </option>
            <option v-for="voice in edgeVoiceOptions" :key="voice.shortName" :value="voice.shortName">
              {{ edgeVoiceLabel(voice) }}
            </option>
          </select>
        </div>
        <!-- 共享参数（音调/音量/音质）放在音色分区之后，双音色时只出现一次。 -->
        <!-- Edge 音调/音量：SSML 的 pitch/volume，只对 Edge 通道生效；
             语速继续用全局「朗读语速」，不在这里重复。 -->
        <div class="setting-row slider-row">
          <div class="slider-head">
            <label>音调</label>
            <span class="slider-value">{{ store.speechConfig.edgePitch }} Hz</span>
            <button
              v-if="store.speechConfig.edgePitch !== 0"
              class="link-btn"
              type="button"
              @click="store.setEdgePitch(0)"
            >回正</button>
          </div>
          <input
            class="settings-slider"
            type="range"
            :min="EDGE_PITCH_MIN"
            :max="EDGE_PITCH_MAX"
            step="5"
            :value="store.speechConfig.edgePitch"
            @input="store.setEdgePitch(Number(($event.target as HTMLInputElement).value))"
          >
        </div>
        <div class="setting-row slider-row">
          <div class="slider-head">
            <label>音量</label>
            <span class="slider-value">{{ store.speechConfig.edgeVolume }} %</span>
            <button
              v-if="store.speechConfig.edgeVolume !== 0"
              class="link-btn"
              type="button"
              @click="store.setEdgeVolume(0)"
            >回正</button>
          </div>
          <input
            class="settings-slider"
            type="range"
            :min="EDGE_VOLUME_MIN"
            :max="EDGE_VOLUME_MAX"
            step="5"
            :value="store.speechConfig.edgeVolume"
            @input="store.setEdgeVolume(Number(($event.target as HTMLInputElement).value))"
          >
        </div>
        <!-- 音质：Edge 只支持 48k/96k 两档 outputFormat，96k 字节数约为 2 倍。 -->
        <div class="setting-row setting-row-top">
          <label>音质</label>
          <select
            class="voice-select"
            :value="store.speechConfig.edgeFormat"
            @change="store.setEdgeFormat(($event.target as HTMLSelectElement).value as EdgeTtsFormat)"
          >
            <option value="48k">标准（48kbps）</option>
            <option value="96k">高（96kbps，音质更好、流量约 2 倍）</option>
          </select>
        </div>

        <p class="setting-hint">
          音调 / 音量 / 音质只对 Edge 通道生效；语速请用下方「朗读语速」。
        </p>

        <p class="setting-hint">
          {{ edgeVoicesLoading
            ? '正在读取 Edge 音色…'
            : '无需 API Key；这是微软的公共接口，偶尔可能不可用。' }}
          <template v-if="edgeVoiceError">（{{ edgeVoiceError }}）</template>
        </p>
      </template>

      <!-- 微软朗读（演示接口）：比 Edge 多了风格 style / 风格强度，音质有 192k 档。
           音色 958 个、zh-CN 84 个，默认只列中文，可切「显示全部语言」。 -->
      <template v-else>
        <!-- 双音色：旁白/对话两个分区，各自「音色 + 风格」；共享参数在后面只出现一次。 -->
        <template v-if="store.speechConfig.dualVoiceEnabled">
          <div class="setting-block voice-partition">
            <div class="setting-block-head">
              <label>旁白</label>
              <button class="link-btn" type="button" @click="showAllMsLocales = !showAllMsLocales">
                {{ showAllMsLocales ? '只看中文' : '显示全部语言' }}
              </button>
            </div>
            <div class="setting-row setting-row-top">
              <label>音色</label>
              <select
                class="voice-select"
                :value="store.speechConfig.msVoice"
                @change="store.setMsVoice(($event.target as HTMLSelectElement).value)"
              >
                <option v-if="!msVoiceOptions.length" :value="store.speechConfig.msVoice">
                  {{ store.speechConfig.msVoice }}
                </option>
                <option v-for="voice in msVoiceOptions" :key="voice.shortName" :value="voice.shortName">
                  {{ msVoiceLabel(voice) }}
                </option>
              </select>
            </div>
            <div class="setting-row setting-row-top">
              <label>风格</label>
              <select
                class="voice-select"
                :value="store.speechConfig.msStyle"
                @change="store.setMsStyle(($event.target as HTMLSelectElement).value)"
              >
                <option value="">无风格</option>
                <option v-for="style in msStyleOptions" :key="style" :value="style">{{ style }}</option>
              </select>
            </div>
          </div>
          <div class="setting-block voice-partition">
            <div class="setting-block-head"><label>对话</label></div>
            <div class="setting-row setting-row-top">
              <label>音色</label>
              <select
                class="voice-select"
                :value="store.speechConfig.msDialogueVoice"
                @change="store.setMsDialogueVoice(($event.target as HTMLSelectElement).value)"
              >
                <option value="">跟随旁白音色</option>
                <option v-for="voice in msVoiceOptions" :key="voice.shortName" :value="voice.shortName">
                  {{ msVoiceLabel(voice) }}
                </option>
              </select>
            </div>
            <div class="setting-row setting-row-top">
              <label>风格</label>
              <select
                class="voice-select"
                :value="store.speechConfig.msDialogueStyle"
                @change="store.setMsDialogueStyle(($event.target as HTMLSelectElement).value)"
              >
                <option value="">跟随旁白风格</option>
                <option v-for="style in msDialogueStyleOptions" :key="style" :value="style">{{ style }}</option>
              </select>
            </div>
          </div>
        </template>
        <template v-else>
          <div class="setting-block">
            <div class="setting-block-head">
              <label>音色</label>
              <button class="link-btn" type="button" @click="showAllMsLocales = !showAllMsLocales">
                {{ showAllMsLocales ? '只看中文' : '显示全部语言' }}
              </button>
            </div>
            <select
              class="voice-select"
              :value="store.speechConfig.msVoice"
              @change="store.setMsVoice(($event.target as HTMLSelectElement).value)"
            >
              <option v-if="!msVoiceOptions.length" :value="store.speechConfig.msVoice">
                {{ store.speechConfig.msVoice }}
              </option>
              <option v-for="voice in msVoiceOptions" :key="voice.shortName" :value="voice.shortName">
                {{ msVoiceLabel(voice) }}
              </option>
            </select>
          </div>
          <!-- 风格选项来自当前音色自带的 StyleList；没有风格的音色只有「无风格」。 -->
          <div class="setting-block">
            <div class="setting-block-head"><label>风格</label></div>
            <select
              class="voice-select"
              :value="store.speechConfig.msStyle"
              @change="store.setMsStyle(($event.target as HTMLSelectElement).value)"
            >
              <option value="">无风格</option>
              <option v-for="style in msStyleOptions" :key="style" :value="style">{{ style }}</option>
            </select>
          </div>
        </template>

        <!-- 共享参数（风格强度/音调/音量/音质）放在音色分区之后，双音色时只出现一次。 -->
        <!-- 风格强度只在有风格时生效，所以没有风格时不显示，避免误导。 -->
        <div v-if="showMsStyleDegree" class="setting-row slider-row">
          <div class="slider-head">
            <label>风格强度</label>
            <span class="slider-value">{{ store.speechConfig.msStyleDegree.toFixed(1) }}</span>
          </div>
          <input
            class="settings-slider"
            type="range"
            :min="MS_STYLE_DEGREE_MIN"
            :max="MS_STYLE_DEGREE_MAX"
            step="0.1"
            :value="store.speechConfig.msStyleDegree"
            @input="store.setMsStyleDegree(Number(($event.target as HTMLInputElement).value))"
          >
        </div>

        <div class="setting-row slider-row">
          <div class="slider-head">
            <label>音调</label>
            <span class="slider-value">{{ store.speechConfig.msPitch }} %</span>
            <button
              v-if="store.speechConfig.msPitch !== 0"
              class="link-btn"
              type="button"
              @click="store.setMsPitch(0)"
            >回正</button>
          </div>
          <input
            class="settings-slider"
            type="range"
            :min="MS_PITCH_MIN"
            :max="MS_PITCH_MAX"
            step="5"
            :value="store.speechConfig.msPitch"
            @input="store.setMsPitch(Number(($event.target as HTMLInputElement).value))"
          >
        </div>
        <div class="setting-row slider-row">
          <div class="slider-head">
            <label>音量</label>
            <span class="slider-value">{{ store.speechConfig.msVolume }} %</span>
            <button
              v-if="store.speechConfig.msVolume !== 0"
              class="link-btn"
              type="button"
              @click="store.setMsVolume(0)"
            >回正</button>
          </div>
          <input
            class="settings-slider"
            type="range"
            :min="MS_VOLUME_MIN"
            :max="MS_VOLUME_MAX"
            step="5"
            :value="store.speechConfig.msVolume"
            @input="store.setMsVolume(Number(($event.target as HTMLInputElement).value))"
          >
        </div>

        <div class="setting-row setting-row-top">
          <label>音质</label>
          <select
            class="voice-select"
            :value="store.speechConfig.msFormat"
            @change="store.setMsFormat(($event.target as HTMLSelectElement).value as MsTtsFormat)"
          >
            <option v-for="format in MS_FORMATS" :key="format" :value="format">
              {{ format === '48k' ? '标准 48k' : format === '96k' ? '高 96k（默认）' : '超高 48kHz·192k' }}
            </option>
          </select>
        </div>

        <p class="setting-hint">
          {{ msVoicesLoading
            ? '正在读取微软朗读音色…'
            : '无需 API Key；这是微软翻译客户端的演示接口，风格仅部分音色支持。' }}
          <template v-if="msVoiceError">（{{ msVoiceError }}）</template>
        </p>
      </template>

      <!-- 缓冲字符数：缓冲流按这个配额预合成音频，越大越不容易断档 -->
      <div
        v-if="usesBufferedSpeech"
        class="setting-row slider-row"
      >
        <div class="slider-head">
          <label>缓冲字符数</label>
          <span class="slider-value">{{ store.speechConfig.bufferChars }} 字</span>
        </div>
        <input
          class="settings-slider"
          type="range"
          :min="BUFFER_CHARS_MIN"
          :max="BUFFER_CHARS_MAX"
          step="100"
          :value="store.speechConfig.bufferChars"
          @input="store.setBufferChars(Number(($event.target as HTMLInputElement).value))"
        >
      </div>
      <p
        v-if="usesBufferedSpeech"
        class="setting-hint"
      >
        自动朗读会提前合成约这么多字的音频再边播边补：调大更抗网络抖动，调小更省请求。
      </p>

      <div class="setting-row setting-row-top">
        <label>朗读语速</label>
        <div class="stepper">
          <button class="step-btn" @click="adjustSpeechRate(-0.1)">—</button>
          <span class="step-val">{{ store.speechConfig.speechRate.toFixed(1) }}</span>
          <button class="step-btn" @click="adjustSpeechRate(0.1)">+</button>
        </div>
      </div>

      <div v-if="store.speechConfig.provider === 'system'" class="setting-row">
        <label>朗读音调</label>
        <div class="stepper">
          <button class="step-btn" @click="adjustSpeechPitch(-0.1)">—</button>
          <span class="step-val">{{ store.speechConfig.speechPitch.toFixed(1) }}</span>
          <button class="step-btn" @click="adjustSpeechPitch(0.1)">+</button>
        </div>
      </div>

      <div class="setting-row setting-row-top">
        <label>定时停止</label>
        <div class="btn-group">
          <button class="opt-btn" :class="{ active: store.speechConfig.stopAfterMinutes === 0 }" @click="store.setSpeechStopTimer(0)">关闭</button>
          <button class="opt-btn" :class="{ active: store.speechConfig.stopAfterMinutes === 15 }" @click="store.setSpeechStopTimer(15)">15分钟</button>
          <button class="opt-btn" :class="{ active: store.speechConfig.stopAfterMinutes === 30 }" @click="store.setSpeechStopTimer(30)">30分钟</button>
          <button class="opt-btn" :class="{ active: store.speechConfig.stopAfterMinutes === 60 }" @click="store.setSpeechStopTimer(60)">60分钟</button>
          <button class="opt-btn" :class="{ active: store.speechConfig.stopAfterMinutes === 120 }" @click="store.setSpeechStopTimer(120)">120分钟</button>
        </div>
      </div>

      <!-- 语音测试：用当前通道 / 音色 / 参数试听一段可编辑文本。
           试听状态与书内朗读完全隔离（见 store.previewSpeech），绝不改 isSpeaking 等。 -->
      <div class="setting-block speech-test">
        <div class="setting-block-head">
          <label>语音测试</label>
          <span class="block-tag">当前通道：{{ store.speechProviderLabel }}</span>
        </div>
        <textarea
          v-model="speechTestText"
          class="speech-test-input"
          rows="2"
          maxlength="300"
          placeholder="输入要试听的文本"
        ></textarea>
        <div class="speech-test-actions">
          <button
            v-if="store.isSpeechPreviewing"
            class="opt-btn active"
            type="button"
            @click="store.stopSpeechPreview()"
          >停止</button>
          <button
            v-else
            class="opt-btn"
            type="button"
            :disabled="store.isSpeechPreviewLoading"
            @click="store.previewSpeech(speechTestText)"
          >{{ store.isSpeechPreviewLoading ? '生成中…' : '试听' }}</button>
          <span class="speech-test-note">试听会先停止书内朗读；用当前音色与参数合成。</span>
        </div>
      </div>
      </section>

      <section v-show="activeSettingsTab === 'more'" class="settings-group">

      <!-- 悬停呼出工具栏: 默认关闭。开启后鼠标移到阅读区边缘会自动弹出工具栏与悬浮钮,
           代价是边缘热区吃掉正文的点击(左右各 84px), 因此作为可选功能。 -->
      <div class="setting-row">
        <label>悬停呼出工具栏</label>
        <div class="btn-group">
          <button class="opt-btn" :class="{ active: config.hoverRevealControls }" title="鼠标移到阅读区上下左右边缘时自动展开工具栏与悬浮钮" @click="store.updateConfig('hoverRevealControls', true)">开启</button>
          <button class="opt-btn" :class="{ active: !config.hoverRevealControls }" title="鼠标移到边缘不再弹出按钮，边缘区域的点击可以正常用于翻页" @click="store.updateConfig('hoverRevealControls', false)">关闭</button>
        </div>
      </div>

      <!-- 快捷键: 以「功能」为主轴, 一个功能可绑多个按键 -->
      <div class="setting-block">
        <div class="setting-block-head">
          <label>快捷键</label>
          <div class="hotkey-head-actions">
            <span class="block-tag">{{ store.hotkeysAreDefault ? '默认方案' : '自定义' }}</span>
            <button class="link-btn" type="button" @click="store.resetHotkeyBindings()">恢复默认</button>
          </div>
        </div>
        <div class="function-list">
          <div v-for="action in bindableActions" :key="action.id" class="function-item">
            <div class="function-name">
              <span>{{ action.label }}</span>
              <span v-if="action.scope !== 'both'" class="function-scope">
                {{ action.scope === 'paged' ? '分页' : '滚动' }}
              </span>
            </div>
            <div class="function-keys">
              <span v-if="!keysForAction(action.id).length" class="function-empty">未绑定</span>
              <button
                v-for="keyId in keysForAction(action.id)"
                :key="keyId"
                type="button"
                class="key-chip"
                :class="{ conflict: keyHasConflict(keyId) }"
                :title="keyHasConflict(keyId) ? keyConflictTitle(keyId) : '点击解除绑定'"
                @click="unbindKey(action.id, keyId)"
              >
                {{ hotkeyLabel(keyId) }}
                <span v-if="keyHasConflict(keyId)" class="key-chip-warn">!</span>
                <span class="key-chip-remove">×</span>
              </button>
              <!-- 录入按键: 点开后直接按键盘, 任意键都能绑。
                   平时只留一个「＋」, 悬停或键盘聚焦到该行才显形 —— 十四个功能
                   各挂一个虚线框会连成一片噪音。 -->
              <button
                v-if="capturingAction !== action.id"
                type="button"
                class="key-capture"
                :title="`按下任意键或鼠标键为「${action.label}」绑定快捷键`"
                :aria-label="`为「${action.label}」录入快捷键`"
                @click="startCapture(action.id)"
              >＋</button>
              <span v-else class="key-capturing">
                请按键/鼠标键…
                <button type="button" class="key-capture-cancel" @click="cancelCapture">取消</button>
              </span>
            </div>
          </div>
        </div>
        <div v-if="conflictNotices.length" class="conflict-notice">
          <div v-for="notice in conflictNotices" :key="`${notice.key}-${notice.mode}`" class="conflict-line">
            <span class="conflict-key">{{ keyLabel(notice.key) }}</span>
            在{{ notice.mode }}模式下只会执行「{{ notice.winner }}」，「{{ notice.ignored }}」不会触发
          </div>
        </div>
      </div>

      <!-- 更多操作 -->
      <div class="setting-row">
        <label>离线缓存</label>
        <button class="opt-btn wide" @click="store.openPanel('cache', 'settings')">批量缓存章节</button>
      </div>

      <div class="setting-row">
        <label>内容净化</label>
        <button class="opt-btn wide" @click="store.openPanel('rule', 'settings')">管理净化规则</button>
      </div>
      </section>
    </div>

    <!-- 引擎管理必须 Teleport 到 body：设置面板自身 overflow-y:auto，fixed 子元素
         会被祖先的 overflow 裁掉（在抽屉里只剩一条缝）。 -->
    <HttpTtsManager
      v-if="showHttpTtsManager"
      @close="handleHttpTtsManagerClosed"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useReaderStore, themePresets, nightThemeIndex, fontPresets } from '../../stores/reader'
import { useAppStore } from '../../stores/app'
import {
  getSpeechApiFormatOption,
  speechApiFormatOptions,
  type SpeechApiFormat,
  type SpeechAudioFormat,
} from '../../utils/openaiSpeech'
import { READER_FONT_SIZE_MAX, READER_FONT_SIZE_MIN } from '../../utils/readerFontSize'
import { getHttpTtsList } from '../../api/httpTts'
import { listEdgeTtsVoices, type EdgeTtsFormat, type EdgeTtsVoice } from '../../api/edgeTts'
import { listMsTtsVoices, type MsTtsFormat, type MsTtsVoice } from '../../api/msTts'
import HttpTtsManager from './HttpTtsManager.vue'
import type { HttpTts } from '../../types'
import {
  BUFFER_CHARS_MAX,
  BUFFER_CHARS_MIN,
  EDGE_PITCH_MAX,
  EDGE_PITCH_MIN,
  EDGE_VOLUME_MAX,
  EDGE_VOLUME_MIN,
  MS_FORMATS,
  MS_PITCH_MAX,
  MS_PITCH_MIN,
  MS_STYLE_DEGREE_MAX,
  MS_STYLE_DEGREE_MIN,
  MS_VOLUME_MAX,
  MS_VOLUME_MIN,
  READER_PAGE_VERTICAL_PADDING_MAX,
  READER_PAGE_WIDTH_MAX,
  READER_PAGE_WIDTH_MIN,
} from '../../stores/reader'
import {
  DEFAULT_READER_PANEL_OPACITY,
  READER_BACKGROUND_OPACITY_STEP,
  READER_PANEL_OPACITY_MIN,
  normalizeReaderPanelOpacity,
  percentToReaderOpacity,
  prepareReaderBackgroundImage,
  readerOpacityToPercent,
} from '../../utils/readerBackground'
import { uploadBackgroundImage } from '../../api/background'
import {
  HOTKEY_ACTIONS,
  HOTKEY_ACTION_MAP,
  HOTKEY_KEYS,
  bareMouseButtonKeyId,
  hotkeyIdFromEvent,
  hotkeyIdFromMouseEvent,
  hotkeyIdFromWheelEvent,
  hotkeyLabel,
  isMouseButtonKeyId,
  setHotkeyCaptureActive,
  type HotkeyActionId,
} from '../../utils/readerHotkeys'

const store = useReaderStore()
const appStore = useAppStore()
const fontInputRef = ref<HTMLInputElement | null>(null)
const readerSettingsTabs = [
  { value: 'display', label: '显示' },
  { value: 'paging', label: '翻页' },
  { value: 'speech', label: '朗读' },
  { value: 'more', label: '更多' },
] as const

/**
 * 独立模式：从主设置抽屉打开「朗读与听书」时用。
 *
 * 此时没有阅读上下文, 只保留「朗读」页签 —— 「显示/翻页」调的是阅读页排版(没有
 * 预览就没有意义), 「更多」里的离线缓存与净化入口又依赖阅读面板栈。这样主设置
 * 与阅读页用的是同一份实现, 不会出现两套配置界面各自漂移。
 */
const props = withDefaults(defineProps<{ standalone?: boolean }>(), { standalone: false })
const emit = defineEmits<{ close: [] }>()

const visibleTabs = computed(() =>
  props.standalone
    ? readerSettingsTabs.filter((tab) => tab.value === 'speech')
    : readerSettingsTabs,
)
const activeSettingsTab = ref<(typeof readerSettingsTabs)[number]['value']>(
  props.standalone ? 'speech' : 'display',
)
const activeCustomFont = computed(() => {
  if (!config.value.fontFamily.startsWith('custom:')) return null
  const id = config.value.fontFamily.slice('custom:'.length)
  return store.customFonts.find((font) => font.id === id) || null
})

async function handleFontFile(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  try {
    await store.importCustomFont(file)
    appStore.showToast('字体已导入并应用', 'success')
  } catch (error) {
    appStore.showToast((error as Error).message || '字体导入失败', 'error')
  } finally {
    input.value = ''
  }
}

async function removeActiveCustomFont() {
  if (!activeCustomFont.value) return
  try {
    await store.removeCustomFont(activeCustomFont.value.id)
    appStore.showToast('字体已删除', 'success')
  } catch (error) {
    appStore.showToast((error as Error).message || '字体删除失败', 'error')
  }
}
const config = computed(() => store.config)
const theme = computed(() => store.chromeTheme)
const isDarkSettings = computed(() => store.isNight || theme.value.name === '暗灰')
const selectedSpeechApiFormat = computed(() => getSpeechApiFormatOption(store.speechConfig.apiFormat))

/* ─── 阅读页背景图 ─── */
const readerImageInputRef = ref<HTMLInputElement | null>(null)
const processingReaderImage = ref(false)

async function handleReaderImageChange(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  processingReaderImage.value = true
  try {
    // 与桌面背景图一致: 原分辨率上传落盘, 不再为 localStorage 配额压图。
    const prepared = await prepareReaderBackgroundImage(file)
    const url = await uploadBackgroundImage('reader', prepared.blob, prepared.fileName)
    store.setReaderBackgroundImage(url)
    appStore.showToast('阅读页背景图已更新', 'success')
  } catch (error) {
    appStore.showToast((error as Error).message || '背景图片处理失败', 'error')
  } finally {
    processingReaderImage.value = false
    input.value = ''
  }
}

function removeReaderBackgroundImage() {
  store.clearReaderBackgroundImage()
  appStore.showToast('已移除阅读页背景图', 'success')
}

/** 百分比输入框: 非数字输入不写回(保持原值), 越界自动夹到 0-100。
 *
 *  无论接受与否, 回调结束时都把 DOM 里的显示值重新对齐到 store 的真实值。
 *  输入框是单向绑定(`:value`), store 值没变化时 Vue 不会重写 DOM —— 于是
 *  「清空输入框」「输入 150」「输入 -5」这类情况会让框里停着非法文本, 而实际
 *  透明度没变, UI 与真实值脱节, 直到面板重开才恢复。 */
function syncOpacityInput(input: HTMLInputElement, applied: number) {
  const actual = readerOpacityToPercent(applied)
  if (input.value !== String(actual)) input.value = String(actual)
}

function applyReaderOpacityPercent(raw: string, input: HTMLInputElement) {
  const next = percentToReaderOpacity(raw)
  if (next === null) {
    syncOpacityInput(input, store.config.readerImageOpacity)
    return
  }
  store.updateConfig('readerImageOpacity', next)
  syncOpacityInput(input, next)
}

/** 面板不透明度输入框: 与背景图透明度同一套「非数字不写回」的规则,
 *  但下限是 READER_PANEL_OPACITY_MIN, 由 normalizeReaderPanelOpacity 统一钳制。 */
function applyPanelOpacityPercent(raw: string, input: HTMLInputElement) {
  const parsed = percentToReaderOpacity(raw)
  if (parsed === null) {
    syncOpacityInput(input, store.config.panelOpacity)
    return
  }
  const next = normalizeReaderPanelOpacity(parsed)
  store.updateConfig('panelOpacity', next)
  syncOpacityInput(input, next)
}

function resetPanelOpacity() {
  store.updateConfig('panelOpacity', DEFAULT_READER_PANEL_OPACITY)
}

/** 一键把背景色与文字色都还原为跟随主题。 */
function resetReaderColors() {
  store.updateConfig('readerBackgroundColor', '')
  store.updateConfig('fontColor', '')
}

/* ─── Hotkeys ─── */
/** 可绑定的功能列表。排除「不操作」: 它是某个按键的行为(解除该键的一切动作),
 *  不是一条可以分配按键的「功能」, 列在这里会让语义混乱。 */
const bindableActions = computed(() => HOTKEY_ACTIONS.filter((action) => action.id !== 'none'))

/** 正在等待录入的「功能」。非 null 时监听键盘, 按下任意键即绑定。 */
const capturingAction = ref<HotkeyActionId | null>(null)

/** 该功能已绑定的按键 id(按当前绑定里的顺序, 保持录入顺序稳定)。 */
function keysForAction(actionId: HotkeyActionId): string[] {
  return Object.keys(store.hotkeyBindings)
    .filter((keyId) => (store.hotkeyBindings[keyId] || []).includes(actionId))
    // 内置预设键排前面(按预设顺序), 用户自建的组合键排后面。
    .sort((a, b) => {
      const ia = HOTKEY_KEYS.findIndex((key) => key.id === a)
      const ib = HOTKEY_KEYS.findIndex((key) => key.id === b)
      if (ia >= 0 && ib >= 0) return ia - ib
      if (ia >= 0) return -1
      if (ib >= 0) return 1
      return a.localeCompare(b)
    })
}

/** 录入按键: 按下任意键(或鼠标键/滚轮)即绑到当前功能。
 *
 *  这些捕获监听是点「＋」时才注册的, 比阅读页的同阶段监听**更晚**, 因此同一次
 *  操作里阅读页的 handler 会先跑 —— 真正让运行时整体让路的是
 *  `setHotkeyCaptureActive(true)`(见 readerHotkeys 的「录入态」与 dispatchHotkey)。
 *  这里的 stopPropagation 是第二道保险: 让后续同阶段/冒泡阶段的其它监听也收不到
 *  这次操作, 免得面板内的滚轮滚动、选区菜单等被牵连。 */
function handleCaptureKeydown(event: KeyboardEvent) {
  if (!capturingAction.value) return
  event.preventDefault()
  event.stopPropagation()
  if (event.key === 'Escape' && !event.ctrlKey && !event.altKey && !event.metaKey) {
    // Esc 是「取消录入」的约定键, 不能既当取消又当绑定目标。
    cancelCapture()
    return
  }
  const keyId = hotkeyIdFromEvent(event)
  if (!keyId) return
  finishCapture(keyId)
}

function handleCaptureMouseDown(event: MouseEvent) {
  if (!capturingAction.value) return
  // 左键/右键按下先不拦: 用户可能只是想点别处或取消。点「取消」按钮走 click,
  // 由 cancelCapture 处理; 中键/侧键才是要录的鼠标键。
  const keyId = hotkeyIdFromMouseEvent(event)
  if (!keyId) {
    // 左键点在录入区之外 = 放弃录入, 保持与 Esc 一致的心智模型。
    if (event.button === 0 && !(event.target as HTMLElement | null)?.closest?.('.key-capturing')) {
      cancelCapture()
    }
    return
  }
  event.preventDefault()
  event.stopPropagation()
  finishCapture(keyId)
}

function handleCaptureWheel(event: WheelEvent) {
  if (!capturingAction.value) return
  const keyId = hotkeyIdFromWheelEvent(event)
  if (!keyId) return
  event.preventDefault()
  event.stopPropagation()
  finishCapture(keyId)
}

/** 录入成功: 落库并提示。
 *
 *  **顺序不能动**: 抑制窗口必须在 `capturingAction.value = null` **之前**写好。
 *  那个赋值会通过 watch(flush:'sync') **同步**跑 releaseCaptureListeners(),
 *  它靠读窗口值算拆除延迟。若窗口还写着 0(或已过期), delayMs 就是 0, 五个捕获
 *  监听器在下一宏任务被摘除 —— 而窗口的唯一读取者正是这些监听器, 于是窗口成了
 *  没人读的死代码, 「按下录完、松开却导航」照旧发生。
 *  曾把窗口赋值放在末尾, 靠手工模拟顺序的脚本"验证通过", 实际是假的:
 *  真实 Vue 的同步 watch 在赋值那一行就跑完了。 */
function finishCapture(keyId: string) {
  const actionId = capturingAction.value
  if (!actionId) return
  // 鼠标键的默认行为(中键自动滚动、侧键前进后退)在**松开**时才发生, 而录入是在
  // 按下时完成的 —— 清空 capturingAction 后单靠它拦不住紧接着的 mouseup。
  // 若漏掉, 侧键一按一放就导航一次: 本应用用 hash 路由, hash 一变即跳路由、
  // 设置面板被卸载, 用户看到的就是「侧键录完就失灵」。因此先开抑制窗口, 再清状态。
  if (isMouseButtonKeyId(keyId)) {
    suppressedMouseButtonKey = bareMouseButtonKeyId(keyId)
    suppressMouseButtonNavUntil = Date.now() + CAPTURE_MOUSEUP_SUPPRESS_MS
  }
  capturingAction.value = null
  store.bindHotkeyKey(keyId, actionId)
  appStore.showToast(`已绑定「${hotkeyLabel(keyId)}」`, 'success')
}

/** 刚录完鼠标键后, 短暂抑制其默认导航的时间窗口(毫秒)。
 *  只需覆盖同一次操作的 mouseup, 取值远大于「按下到松开」的间隔即可。 */
const CAPTURE_MOUSEUP_SUPPRESS_MS = 1000
let suppressMouseButtonNavUntil = 0
/** 窗口内要抑制的**具体键位**。
 *
 *  必须记键位而不能只判断"是不是鼠标键": 用户录完侧键1 后一秒内若去按侧键2,
 *  后者(通常未绑定)的松开会本应正常导航, 只按类别判断会把这次也吞掉 ——
 *  表现为「刚设过快捷键, 另一个侧键就失灵了一秒」。 */
let suppressedMouseButtonKey = ''
/** 延迟拆除捕获监听的定时器句柄 —— 见 releaseCaptureListeners 的说明。 */
let captureReleaseTimer: ReturnType<typeof setTimeout> | null = null

/** 是否处于「刚录完该鼠标键」的抑制窗口内。只对同一个键生效。 */
function isMouseButtonNavSuppressed(keyId: string | null): boolean {
  if (!keyId || keyId !== suppressedMouseButtonKey) return false
  return Date.now() < suppressMouseButtonNavUntil
}

function startCapture(actionId: HotkeyActionId) {
  // 开始新一次录入即作废上一次的抑制窗口, 避免它挡住用户接下来正常的侧键操作。
  suppressMouseButtonNavUntil = 0
  suppressedMouseButtonKey = ''
  if (captureReleaseTimer !== null) {
    clearTimeout(captureReleaseTimer)
    captureReleaseTimer = null
  }
  capturingAction.value = actionId
}

/** 拆除录入期的捕获监听。
 *
 *  不能一退出录入态就立即拆: 抑制窗口(mouseup 拦截)的判定函数正是这些监听器,
 *  拆掉之后写入的窗口就成了没人读的死代码 —— 而「按下录完、松开导航」恰好发生
 *  在这个窗口内。因此推迟到窗口结束后再拆。 */
function releaseCaptureListeners() {
  if (typeof window === 'undefined') return
  if (captureReleaseTimer !== null) clearTimeout(captureReleaseTimer)
  const delayMs = Math.max(0, suppressMouseButtonNavUntil - Date.now())
  captureReleaseTimer = setTimeout(() => {
    captureReleaseTimer = null
    // 期间用户可能又开始录入 —— 那就不能拆。
    if (capturingAction.value) return
    window.removeEventListener('keydown', handleCaptureKeydown, true)
    window.removeEventListener('mousedown', handleCaptureMouseDown, true)
    window.removeEventListener('wheel', handleCaptureWheel, { capture: true })
    window.removeEventListener('auxclick', handleCaptureAuxClick, true)
    window.removeEventListener('mouseup', handleCaptureMouseUp, true)
  }, delayMs)
}

function cancelCapture() {
  capturingAction.value = null
}

watch(capturingAction, (active) => {
  if (typeof window === 'undefined') return
  // 录入期间让运行时的快捷键派发整体让路(见 readerHotkeys 的「录入态」):
  // 否则「把侧键改绑到目录」会先被阅读页接走、顺手切换面板。
  setHotkeyCaptureActive(Boolean(active))
  if (active) {
    // 捕获阶段 + 非 passive: 必须能在阅读页的 wheel 监听之前消费掉这次滚动。
    window.addEventListener('keydown', handleCaptureKeydown, true)
    window.addEventListener('mousedown', handleCaptureMouseDown, true)
    window.addEventListener('wheel', handleCaptureWheel, { capture: true, passive: false })
    window.addEventListener('auxclick', handleCaptureAuxClick, true)
    // 侧键的前进/后退导航在**松开**时触发(Chromium 行为, 见 MDN 与
    // three.js issue #15713): 只拦 mousedown 不够, 录制期间松开侧键仍会导航 ——
    // 本应用用 hash 路由, hash 一变就跳路由、设置面板被卸载, 录入随之中断。
    window.addEventListener('mouseup', handleCaptureMouseUp, true)
  } else {
    // 不立即拆: 刚录完鼠标键时还要靠这些监听器拦住同一次操作的 mouseup,
    // 详见 releaseCaptureListeners。拆掉之前它们是抑制窗口的唯一读取者。
    releaseCaptureListeners()
  }
  // flush: 'sync' —— 标志必须在同一次用户操作内立即生效。默认的异步 flush 会
  // 让「点开录入」这一拍之后的那次按键仍按运行时派发, 恰是要拦掉的那一次。
}, { flush: 'sync' })

/** 中键/侧键的默认行为(中键自动滚动、侧键前进后退)必须在按下时就被拦掉,
 *  否则松开时浏览器仍会执行一次。这里只拦, 不录 —— 录入在 mousedown 完成。 */
function handleCaptureAuxClick(event: MouseEvent) {
  const keyId = hotkeyIdFromMouseEvent(event)
  if (!capturingAction.value && !isMouseButtonNavSuppressed(keyId)) return
  if (!keyId) return
  event.preventDefault()
  event.stopPropagation()
}

/** 侧键导航在松开时触发, 录制期间必须一并拦掉。
 *
 *  两个触发条件缺一不可: 正在录入, 或刚录完**同一个**鼠标键(此时 capturingAction
 *  已清空)。后者针对的正是「按下录完、松开却导航」的那一次 —— 详见 finishCapture。
 *
 *  注意它比对的是具体键位而非"是不是鼠标键": 带修饰键录入(如 Ctrl+侧键1)后先松
 *  Ctrl 再松侧键1, mouseup 的标识是未绑定的 `Mouse4`, 阅读页那边不会拦, 只能靠
 *  这个窗口挡住 —— 所以窗口必须存在且能被读到(见 finishCapture 的顺序说明)。 */
function handleCaptureMouseUp(event: MouseEvent) {
  const keyId = hotkeyIdFromMouseEvent(event)
  if (!capturingAction.value && !isMouseButtonNavSuppressed(keyId)) return
  if (!keyId) return
  event.preventDefault()
  event.stopPropagation()
}

onBeforeUnmount(() => {
  // 关闭设置面板 / 离开阅读页：试听必须停下并回收 objectURL，否则声音会一直响
  // 而用户已经没有入口关掉它（试听不挂在听书面板上）。
  store.stopSpeechPreview()
  if (typeof window === 'undefined') return
  // 面板可能在录入途中被卸载(例如换章导致重渲染)。不清标志会让快捷键
  // 永久失效 —— 运行时的派发会被一个永远为真的「正在录入」挡住。
  setHotkeyCaptureActive(false)
  // 抑制窗口与延迟拆除都要清: 面板若在「刚录完鼠标键」时被卸载, 留着定时器
  // 会去操作已经卸载的监听器, 留着窗口会让侧键在一秒内无法导航。
  suppressMouseButtonNavUntil = 0
  suppressedMouseButtonKey = ''
  if (captureReleaseTimer !== null) {
    clearTimeout(captureReleaseTimer)
    captureReleaseTimer = null
  }
  window.removeEventListener('keydown', handleCaptureKeydown, true)
  window.removeEventListener('mousedown', handleCaptureMouseDown, true)
  window.removeEventListener('wheel', handleCaptureWheel, { capture: true })
  window.removeEventListener('auxclick', handleCaptureAuxClick, true)
  window.removeEventListener('mouseup', handleCaptureMouseUp, true)
})

function unbindKey(actionId: HotkeyActionId, keyId: string) {
  store.unbindHotkeyKey(keyId, actionId)
}

/** 该按键上是否存在「同模式多动作」冲突; 有则给标签加警示样式。
 *  运行时只执行第一个适用的动作, 后面的会静默失效, 必须让用户看见。 */
function keyHasConflict(keyId: string): boolean {
  return store.hotkeyConflicts.some((conflict) => conflict.keyId === keyId)
}

function keyConflictTitle(keyId: string): string {
  const details = store.hotkeyConflicts
    .filter((conflict) => conflict.keyId === keyId)
    .map((conflict) => {
      const mode = conflict.mode === 'paged' ? '分页' : '滚动'
      const first = HOTKEY_ACTION_MAP[conflict.actions[0]]?.label || conflict.actions[0]
      const ignored = conflict.actions
        .slice(1)
        .map((id) => HOTKEY_ACTION_MAP[id]?.label || id)
        .join('、')
      return `${mode}模式：只有「${first}」生效，「${ignored}」不会触发`
    })
  return details.join('；')
}

/** 冲突提示文案(只在实际存在冲突时展示)。 */
const conflictNotices = computed(() => store.hotkeyConflicts.map((conflict) => ({
  key: conflict.keyId,
  mode: conflict.mode === 'paged' ? '分页' : '滚动',
  winner: HOTKEY_ACTION_MAP[conflict.actions[0]]?.label || conflict.actions[0],
  ignored: conflict.actions
    .slice(1)
    .map((id) => HOTKEY_ACTION_MAP[id]?.label || id)
    .join('、'),
})))

function keyLabel(keyId: string): string {
  return hotkeyLabel(keyId)
}

function isThemeActive(index: number) {
  return index === nightThemeIndex
    ? store.isNight
    : !store.isNight && store.themeIndex === index
}

function updateSlider(
  key: 'fontSize' | 'fontWeight' | 'animateDuration' | 'pageWidth' | 'pageVerticalPadding',
  event: Event,
) {
  const value = Number((event.target as HTMLInputElement).value)
  if (Number.isFinite(value)) {
    store.updateConfig(key, value)
  }
}

// 滚动速度用对数刻度映射到滑条(2 → 500 px/秒):
// 低速段调节精细, 高速段跨度大, 全程都好拖。
const AUTO_SCROLL_SPEED_MIN = 2
const AUTO_SCROLL_SPEED_MAX = 500

const autoScrollSpeedSlider = computed(() => speedToSlider(config.value.autoScrollSpeed))

function updateAutoScrollSpeedSlider(event: Event) {
  const sliderValue = Number((event.target as HTMLInputElement).value)
  if (!Number.isFinite(sliderValue)) return
  store.updateConfig('autoScrollSpeed', sliderToSpeed(sliderValue))
}

function sliderToSpeed(sliderValue: number) {
  const ratio = Math.pow(AUTO_SCROLL_SPEED_MAX / AUTO_SCROLL_SPEED_MIN, sliderValue / 100)
  return Math.max(
    AUTO_SCROLL_SPEED_MIN,
    Math.min(AUTO_SCROLL_SPEED_MAX, Math.round(AUTO_SCROLL_SPEED_MIN * ratio)),
  )
}

function speedToSlider(speed: number) {
  const sliderValue = 100 * Math.log(Math.max(AUTO_SCROLL_SPEED_MIN, speed) / AUTO_SCROLL_SPEED_MIN)
    / Math.log(AUTO_SCROLL_SPEED_MAX / AUTO_SCROLL_SPEED_MIN)
  return Math.max(0, Math.min(100, Math.round(sliderValue)))
}

function stepFloat(key: 'lineHeight' | 'paragraphSpacing', delta: number, min: number, max: number) {
  const val = Math.max(min, Math.min(max, parseFloat(((config.value[key] as number) + delta).toFixed(1))))
  store.updateConfig(key, val)
}

function adjustSpeechRate(delta: number) {
  const val = Math.max(0.5, Math.min(3, parseFloat((store.speechConfig.speechRate + delta).toFixed(1))))
  store.setSpeechRate(val)
}

function adjustSpeechPitch(delta: number) {
  const val = Math.max(0.5, Math.min(2, parseFloat((store.speechConfig.speechPitch + delta).toFixed(1))))
  store.setSpeechPitch(val)
}

function handleVoiceChange(event: Event) {
  const target = event.target as HTMLSelectElement | null
  store.setVoiceName(target?.value || '')
}

/** 语音测试的示例文本：含引号，方便验证双音色与标点停顿。 */
const speechTestText = ref('他推开门，看见她站在窗边。“你来了。”')

/* ─── 在线朗读引擎（httpTTS） ─── */
const httpTtsList = ref<HttpTts[]>([])
const httpTtsLoading = ref(false)
const showHttpTtsManager = ref(false)

/** select 的 value 必须是字符串：存储里 id 可能是 number（后端下发）或 string。 */
const selectedHttpTtsId = computed(() => (
  store.speechConfig.httpTtsId == null ? '' : String(store.speechConfig.httpTtsId)
))

const selectedDialogueHttpTtsId = computed(() => (
  store.speechConfig.dialogueHttpTtsId == null ? '' : String(store.speechConfig.dialogueHttpTtsId)
))

function syncHttpTtsSelection() {
  const selected = httpTtsList.value.find((engine) => String(engine.id) === selectedHttpTtsId.value)
  if (selected) {
    // 列表里的 contentType 可能比配置里新（重新导入过引擎），补写一次做 MIME 兜底。
    if (!store.speechConfig.httpTtsContentType && selected.contentType) {
      store.setHttpTtsId(selected.id, selected.contentType)
    }
  } else if (httpTtsList.value.length) {
    // 选中的引擎已被删除，或从未选过：列表非空时顺手选中第一个，省一次点击。
    const first = httpTtsList.value[0]
    store.setHttpTtsId(first.id, first.contentType || '')
  } else if (store.speechConfig.httpTtsId != null) {
    // 列表清空却还留着旧 id：必须清掉，否则 startTTS 会拿不存在的 id 去请求。
    store.setHttpTtsId(null)
  }

  // 对话引擎不做「自动选第一个」：旁白/对话本就该是两个引擎，随便填一个会让
  // 用户以为分音色生效了其实没换。只在它指向已删除引擎时清掉，回落到跟随旁白。
  const dialogueExists = httpTtsList.value.some(
    (engine) => String(engine.id) === selectedDialogueHttpTtsId.value,
  )
  if (!dialogueExists && store.speechConfig.dialogueHttpTtsId != null) {
    store.setDialogueHttpTtsId(null)
  }
}

async function loadHttpTtsList() {
  httpTtsLoading.value = true
  try {
    httpTtsList.value = await getHttpTtsList()
    syncHttpTtsSelection()
  } catch (error) {
    httpTtsList.value = []
    appStore.showToast((error as Error).message || '读取在线朗读引擎失败', 'error')
  } finally {
    httpTtsLoading.value = false
  }
}

function handleHttpTtsChange(event: Event) {
  const value = (event.target as HTMLSelectElement | null)?.value || ''
  const engine = httpTtsList.value.find((item) => String(item.id) === value)
  store.setHttpTtsId(engine ? engine.id : null, engine?.contentType || '')
}

function handleDialogueHttpTtsChange(event: Event) {
  const value = (event.target as HTMLSelectElement | null)?.value || ''
  const engine = httpTtsList.value.find((item) => String(item.id) === value)
  store.setDialogueHttpTtsId(engine ? engine.id : null, engine?.contentType || '')
}

function handleHttpTtsManagerClosed() {
  showHttpTtsManager.value = false
  // 管理面板里可能导入/删除了引擎，回到设置页必须刷新下拉与选中项。
  void loadHttpTtsList()
}

/* ─── Edge 朗读 ─── */
const edgeVoices = ref<EdgeTtsVoice[]>([])
const edgeVoicesLoading = ref(false)
const edgeVoiceError = ref('')
/** 默认只列中文：后端返回 322 个音色，全列出来会把下拉淹没。 */
const showAllEdgeLocales = ref(false)

/** 哪些通道走缓冲流（缓冲字符数滑块只对它们有意义）。 */
const usesBufferedSpeech = computed(() => {
  const provider = store.speechConfig.provider
  return provider === 'http' || provider === 'openai' || provider === 'edge' || provider === 'msdemo'
})

/** 当前筛选 + 已选音色（筛掉的也补进来，避免下拉因值不存在而显示空白）。 */
const edgeVoiceOptions = computed(() => {
  const all = edgeVoices.value
  const filtered = showAllEdgeLocales.value
    ? [...all]
    : all.filter((voice) => voice.locale.toLowerCase().startsWith('zh-cn'))
  for (const shortName of [store.speechConfig.edgeVoice, store.speechConfig.edgeDialogueVoice]) {
    if (!shortName) continue
    if (filtered.some((voice) => voice.shortName === shortName)) continue
    const selected = all.find((voice) => voice.shortName === shortName)
    if (selected) filtered.push(selected)
  }
  return filtered
})

function edgeVoiceLabel(voice: EdgeTtsVoice) {
  // 中文音色的 friendlyName 已经足够可读；显示全部语言时补 locale，避免同名混淆。
  const name = voice.friendlyName || voice.shortName
  return showAllEdgeLocales.value ? `${name} · ${voice.locale}` : name
}

async function loadEdgeTtsVoices() {
  if (edgeVoices.value.length || edgeVoicesLoading.value) return
  edgeVoicesLoading.value = true
  edgeVoiceError.value = ''
  try {
    edgeVoices.value = await listEdgeTtsVoices()
  } catch (error) {
    // 音色列表拉不到不该阻断其它通道：保留默认音色，只提示一行可读错误。
    edgeVoiceError.value = (error as Error).message || '读取 Edge 音色失败'
    appStore.showToast(edgeVoiceError.value, 'error')
  } finally {
    edgeVoicesLoading.value = false
  }
}

/* ─── 微软朗读 ─── */
const msVoices = ref<MsTtsVoice[]>([])
const msVoicesLoading = ref(false)
const msVoiceError = ref('')
/** 958 个音色全列会淹没选择：默认只列 zh-CN，可切「显示全部语言」。 */
const showAllMsLocales = ref(false)

function msVoiceStyles(shortName: string): string[] {
  const voice = msVoices.value.find((item) => item.shortName === shortName)
  // 后端 styles 可能为空/含空串：过滤掉，避免下拉出现空白项。
  return (voice?.styles || []).filter((style) => typeof style === 'string' && style.trim())
}

/** 旁白可选风格 = 当前旁白音色自带的风格。 */
const msStyleOptions = computed(() => msVoiceStyles(store.speechConfig.msVoice))
/** 对话可选风格 = 对话音色（未选时跟随旁白音色）自带的风格。 */
const msDialogueStyleOptions = computed(() => (
  msVoiceStyles(store.speechConfig.msDialogueVoice || store.speechConfig.msVoice)
))

/** 风格强度是共享参数：只要旁白或（双音色时）对话任一方有风格就显示一次。 */
const showMsStyleDegree = computed(() => (
  store.speechConfig.dualVoiceEnabled
    ? Boolean(store.speechConfig.msStyle || store.speechConfig.msDialogueStyle)
    : Boolean(store.speechConfig.msStyle)
))

/** 当前筛选 + 已选音色（筛掉的也补进来，避免下拉因值不存在而显示空白）。 */
const msVoiceOptions = computed(() => {
  const all = msVoices.value
  const filtered = showAllMsLocales.value
    ? [...all]
    : all.filter((voice) => voice.locale.toLowerCase().startsWith('zh-cn'))
  for (const shortName of [store.speechConfig.msVoice, store.speechConfig.msDialogueVoice]) {
    if (!shortName) continue
    if (filtered.some((voice) => voice.shortName === shortName)) continue
    const selected = all.find((voice) => voice.shortName === shortName)
    if (selected) filtered.push(selected)
  }
  return filtered
})

function msVoiceLabel(voice: MsTtsVoice) {
  const name = voice.friendlyName || voice.shortName
  return showAllMsLocales.value ? `${name} · ${voice.locale}` : name
}

/**
 * 音色切换后，旧风格可能不在新音色的 StyleList 里 —— 此时回落到「无风格」，
 * 否则会把一个该音色不支持的 style 发给服务端（多半直接报错或静默失效）。
 */
function syncMsStyles() {
  if (!msVoices.value.length) return
  if (store.speechConfig.msStyle && !msStyleOptions.value.includes(store.speechConfig.msStyle)) {
    store.setMsStyle('')
  }
  const dialogueStyle = store.speechConfig.msDialogueStyle
  if (dialogueStyle && !msDialogueStyleOptions.value.includes(dialogueStyle)) {
    store.setMsDialogueStyle('')
  }
}

async function loadMsTtsVoices() {
  if (msVoices.value.length || msVoicesLoading.value) return
  msVoicesLoading.value = true
  msVoiceError.value = ''
  try {
    msVoices.value = await listMsTtsVoices()
    syncMsStyles()
  } catch (error) {
    // 音色列表拉不到不该阻断其它通道：保留默认音色，只提示一行可读错误。
    msVoiceError.value = (error as Error).message || '读取微软朗读音色失败'
    appStore.showToast(msVoiceError.value, 'error')
  } finally {
    msVoicesLoading.value = false
  }
}

watch(() => store.speechConfig.msVoice, () => syncMsStyles())
watch(() => store.speechConfig.msDialogueVoice, () => syncMsStyles())

// 切到在线朗读引擎 / Edge / 微软朗读时才拉列表：绝大多数用户用系统语音，不该多发 IPC。
watch(() => store.speechConfig.provider, (provider) => {
  // 试听用的是「当前通道」，切通道后旧试听的声音已经对不上配置，直接停掉。
  store.stopSpeechPreview()
  if (provider === 'http' && !httpTtsList.value.length) void loadHttpTtsList()
  if (provider === 'edge') void loadEdgeTtsVoices()
  if (provider === 'msdemo') void loadMsTtsVoices()
})

// 离开「朗读」页签时试听按钮已不可见，继续出声会变成关不掉的声音。
watch(activeSettingsTab, (tab) => {
  if (tab !== 'speech') store.stopSpeechPreview()
})

onMounted(async () => {
  store.fetchVoices()
  if (store.speechConfig.provider === 'http') void loadHttpTtsList()
  if (store.speechConfig.provider === 'edge') void loadEdgeTtsVoices()
  if (store.speechConfig.provider === 'msdemo') void loadMsTtsVoices()
})
</script>

<style scoped>
.read-settings {
  --settings-field-bg: rgba(255, 255, 255, 0.58);
  --settings-field-bg-hover: rgba(255, 255, 255, 0.78);
  --settings-field-border: rgba(55, 42, 31, 0.14);
  --settings-field-placeholder: rgba(55, 42, 31, 0.46);
  width: 100%;
  height: 100%;
  /* 必须 border-box: width:100% + padding 若按 content-box 计算, 面板会比
   * 抽屉宽出两倍 padding(原先 48px), 右侧内容(滑块数值等)被挤出可视区。 */
  box-sizing: border-box;
  overflow-y: auto;
  padding: 24px;
  transition: background 0.3s, color 0.3s;
  -webkit-overflow-scrolling: touch;
}

.read-settings.is-dark {
  --settings-field-bg: rgba(255, 255, 255, 0.055);
  --settings-field-bg-hover: rgba(255, 255, 255, 0.085);
  --settings-field-border: rgba(255, 255, 255, 0.15);
  --settings-field-placeholder: rgba(255, 255, 255, 0.38);
}

.settings-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 8px;
}

.settings-title {
  font-size: 20px;
  font-weight: 700;
}

.settings-sep {
  height: 1px;
  background: currentColor;
  opacity: 0.08;
  margin: 16px 0;
  width: 60px;
}

.settings-sep:first-of-type {
  background: var(--color-primary, #c97f3a);
  opacity: 1;
  height: 3px;
  border-radius: 2px;
}

.reset-btn {
  padding: 6px 16px;
  border-radius: 20px;
  /* 描边保留主色以示"重置"语义, 文字改跟随阅读主题: 主色文字压在阅读主题的
   * 浅色底上只有 2.6~2.8:1, 不达标。 */
  border: 1px solid color-mix(in srgb, var(--color-primary) 34%, transparent);
  color: var(--settings-font, currentColor);
  font-size: 13px;
  font-weight: 500;
  transition: all 0.2s;
  background: transparent;
}

.reset-btn:hover {
  background: color-mix(in srgb, var(--color-primary) 14%, transparent);
  border-color: var(--color-primary);
}

.settings-body {
  display: flex;
  flex-direction: column;
}

.reader-settings-tabs {
  position: sticky;
  top: -24px;
  z-index: 2;
  display: grid;
  /* 列数跟随页签数量, 加/减页签时不会被压扁。 */
  grid-template-columns: repeat(auto-fit, minmax(0, 1fr));
  gap: 3px;
  margin-bottom: 20px;
  padding: 3px;
  border-radius: 14px;
  background: color-mix(in srgb, var(--settings-popup) 90%, transparent);
  backdrop-filter: blur(12px);
}

.reader-settings-tabs button {
  min-height: 38px;
  border-radius: 11px;
  color: inherit;
  font-size: 13px;
  font-weight: 600;
  opacity: 0.62;
  transition: background 0.18s, opacity 0.18s, box-shadow 0.18s, transform 0.18s;
}

.reader-settings-tabs button:hover:not(.active) {
  background: color-mix(in srgb, var(--settings-font) 7%, transparent);
  opacity: 0.82;
}

.reader-settings-tabs button:active {
  transform: scale(0.98);
}

.reader-settings-tabs button.active {
  background: var(--settings-field-bg-hover);
  box-shadow: 0 1px 5px rgba(0, 0, 0, 0.12);
  opacity: 1;
}

.settings-group {
  display: flex;
  flex-direction: column;
  gap: 20px;
}

.shortcut-hint {
  margin-top: -8px;
}

/* ─── 行布局：所有「标签 + 控件」行共用同一套栅格 ───
 *
 * 两套布局并存是「没对齐」的根源：普通行是 flex（label min-width 70px）、
 * 滑块行是 grid（label 96px、右对齐、滑块固定 240px）。于是同一屏里标签右缘、
 * 控件左缘都各走各的。
 *
 * 现在统一成一条栅格：`104px` 标签列 + 剩余宽度控件列，标签**左对齐**。
 * 选左对齐而不是右对齐，是因为面板里还有 `.setting-block-head` 的块标题
 *（「阅读配色」「阅读页背景图」）——那些天然是左对齐的块级文字。只有让行标签
 * 也左对齐，整块面板才会只剩**一条**文本竖线，视觉上才是齐的。
 * 104px 是「左右页上下边距」（7 个汉字，14px 字号）不换行的最小宽度。
 */
.setting-row {
  display: grid;
  grid-template-columns: 104px minmax(0, 1fr);
  align-items: center;
  column-gap: 14px;
  row-gap: 4px;
}

.setting-row-top {
  align-items: start;
}

.setting-row > label,
.setting-row .slider-head > label {
  grid-column: 1;
  min-width: 0;
  font-size: 14px;
  font-weight: 500;
  opacity: 0.7;
  text-align: left;
  line-height: 1.5;
  /* 极长标签允许换行，但不要在任何位置硬断（中文标签不该被拆成两行）。 */
  overflow-wrap: break-word;
}

/* 控件列：从第 2 列开始铺到行尾。 */
.setting-row > .btn-group,
.setting-row > .stepper,
.setting-row > .font-picker,
.setting-row > .voice-select,
.setting-row > .opt-btn.wide,
.setting-row > .image-picker-row,
.setting-row > .inline-slider,
.setting-row > .theme-swatches {
  grid-column: 2 / -1;
  min-width: 0;
  justify-self: start;
}

/* 按钮组/步进器/色板按内容宽度排列，不要被拉伸成整行。 */
.setting-row > .btn-group,
.setting-row > .stepper,
.setting-row > .theme-swatches {
  justify-self: start;
}

/* 整行宽按钮（缓存/净化入口）才铺满控件列。 */
.setting-row > .opt-btn.wide {
  justify-self: stretch;
}

/* 下拉框铺满控件列，但绝不超出。 */
.setting-row > .voice-select {
  justify-self: stretch;
}

/* ─── 阅读配色 / 背景图 ─── */
/* 整行块: label 独占一行, 内容不再被挤在固定宽度的右侧列里。 */
.setting-block {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

/* 旁白/对话两个独立分区：带边框与浅底，避免两行音色挤在一起。 */
.voice-partition {
  padding: 12px 14px;
  border: 1px solid color-mix(in srgb, var(--settings-font, currentColor) 14%, transparent);
  border-radius: 14px;
  background: color-mix(in srgb, var(--settings-font, currentColor) 3.5%, transparent);
}

.voice-partition .setting-block-head label {
  font-weight: 600;
  opacity: 0.82;
}

.setting-block-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.setting-block-head label {
  font-size: 14px;
  font-weight: 500;
  opacity: 0.7;
}

/* ─── 语音测试 ─── */
.speech-test-input {
  width: 100%;
  box-sizing: border-box;
  min-height: 56px;
  padding: 10px 12px;
  border-radius: 12px;
  border: 1px solid var(--settings-field-border);
  background: var(--settings-field-bg);
  color: inherit;
  font: inherit;
  line-height: 1.6;
  resize: vertical;
  outline: none;
  transition: background 0.18s, border-color 0.18s, box-shadow 0.18s;
}

.speech-test-input:focus {
  background: var(--settings-field-bg-hover);
  border-color: var(--color-primary, #c97f3a);
  box-shadow: 0 0 0 3px rgba(201, 127, 58, 0.16);
}

.speech-test-input::placeholder {
  color: var(--settings-field-placeholder);
}

.speech-test-actions {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}

.speech-test-note {
  font-size: 12px;
  line-height: 1.5;
  opacity: 0.6;
}

.link-btn {
  padding: 2px 8px;
  border: none;
  background: transparent;
  color: var(--color-primary, #c97f3a);
  font-size: 12px;
  cursor: pointer;
  border-radius: 6px;
}

.link-btn:hover {
  background: color-mix(in srgb, var(--color-primary, #c97f3a) 12%, transparent);
}

.link-btn.danger {
  color: #c43f3a;
}

.link-btn.danger:hover {
  background: color-mix(in srgb, #d14b45 12%, transparent);
}

.color-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
  gap: 10px;
}

.color-card {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 12px;
  border: 1px solid rgba(0, 0, 0, 0.1);
  min-width: 0;
}

.color-card.customized {
  border-color: var(--color-primary, #c97f3a);
}

.color-card-label {
  font-size: 13px;
  opacity: 0.7;
  flex-shrink: 0;
}

/* 色块本身是 label, 点击即打开系统取色器; 原生 color input 藏在其中。 */
.color-swatch {
  position: relative;
  width: 28px;
  height: 28px;
  border-radius: 8px;
  border: 1px solid rgba(0, 0, 0, 0.18);
  cursor: pointer;
  flex-shrink: 0;
  overflow: hidden;
}

.color-input {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  padding: 0;
  border: none;
  opacity: 0;
  cursor: pointer;
}

.color-card-value {
  font-size: 12px;
  opacity: 0.65;
  font-variant-numeric: tabular-nums;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.icon-btn {
  width: 20px;
  height: 20px;
  border-radius: 50%;
  border: none;
  background: rgba(0, 0, 0, 0.06);
  color: inherit;
  font-size: 13px;
  line-height: 1;
  cursor: pointer;
  flex-shrink: 0;
}

.icon-btn:hover {
  background: rgba(0, 0, 0, 0.12);
}

.image-picker-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.image-thumb {
  width: 84px;
  height: 56px;
  border-radius: 10px;
  border: 1px dashed rgba(0, 0, 0, 0.2);
  background-color: transparent;
  background-size: cover;
  background-position: center;
  color: inherit;
  font-size: 11px;
  opacity: 0.85;
  cursor: pointer;
  flex-shrink: 0;
}

.image-thumb.empty {
  display: flex;
  align-items: center;
  justify-content: center;
}

.image-picker-meta {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.image-picker-note {
  font-size: 11px;
  opacity: 0.55;
}

.hidden-input {
  display: none;
}

.inline-slider {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 13px;
}

.inline-slider label {
  opacity: 0.7;
  flex-shrink: 0;
}

.inline-slider input[type="range"] {
  flex: 1;
  min-width: 0;
}

/* 注意选择器要带上 .inline-slider: 上面那条 `.inline-slider label` 是 (0,1,1),
 * 单写 `.opacity-input-wrap` 只有 (0,1,0), 会被它的 opacity: 0.7 压回去。 */
.inline-slider .opacity-input-wrap {
  display: inline-flex;
  align-items: center;
  gap: 0;
  flex-shrink: 0;
  padding: 3px 5px;
  border-radius: 6px;
  /* 透明边框占位: 悬停/聚焦只换颜色, 不会冒出 1px 的布局抖动。 */
  border: 1px solid transparent;
  cursor: text;
  /* 覆盖 .inline-slider label 的 0.7 —— 那是对说明文字的弱化; 输入控件保持正常
   * 对比度, 否则数字看上去像禁用态。 */
  opacity: 1;
  transition: background 0.16s ease, border-color 0.16s ease;
}

.inline-slider .opacity-input-wrap:hover {
  background: color-mix(in srgb, var(--settings-font, currentColor) 6%, transparent);
}

.inline-slider .opacity-input-wrap:focus-within {
  background: color-mix(in srgb, var(--settings-font, currentColor) 10%, transparent);
  border-color: color-mix(in srgb, var(--settings-font, currentColor) 24%, transparent);
}

.opacity-input {
  /* 3.6ch ≈ 1.8em, 恰好容下三位数字(tabular-nums 下每位约 0.55em, "100" 要
   * 1.65em)。再宽就会把数字推离 %, 再窄则三位数会顶格甚至被裁。 */
  width: 3.6ch;
  padding: 0;
  border: none;
  background: transparent;
  color: inherit;
  font-size: 12px;
  font-variant-numeric: tabular-nums;
  text-align: right;
  opacity: 0.85;
  -moz-appearance: textfield;
  appearance: textfield;
}

.opacity-input:focus {
  outline: none;
  opacity: 1;
}

/* number 输入的上下箭头在这么小的框里只会挤掉数字本身。 */
.opacity-input::-webkit-outer-spin-button,
.opacity-input::-webkit-inner-spin-button {
  -webkit-appearance: none;
  margin: 0;
}

.opacity-unit {
  font-size: 12px;
  opacity: 0.6;
}

/* ─── 快捷键: 以功能为主轴 ───
 * 这个面板跟随**阅读主题**(不是应用主题), 所以颜色一律走本组件的 --settings-*
 * 令牌与 --settings-font 的 color-mix。早先这里写死了 rgba(0,0,0,.05) 这类
 * 绝对黑与兜底橙 #c97f3a: 在暗色阅读主题下, 绝对黑的分隔线/label 几乎不可见,
 * 而兜底橙又与面板主色对不上, 整块区域像是从别处搬来的。 */
.block-tag {
  padding: 2px 8px;
  border-radius: 8px;
  font-size: 11px;
  background: color-mix(in srgb, var(--settings-font, currentColor) 8%, transparent);
  opacity: 0.75;
}

.hotkey-head-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.function-list {
  display: flex;
  flex-direction: column;
}

.function-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 4px 6px;
  margin: 0 -6px;
  border-radius: 8px;
  transition: background 0.16s;
}

/* 用悬停底色区分行, 而不是给每一行都画一条分隔线 —— 分隔线 + 常驻按钮
   会把这块变成一张表格。 */
.function-item:hover,
.function-item:focus-within {
  background: color-mix(in srgb, var(--settings-font, currentColor) 6%, transparent);
}

.function-name {
  display: flex;
  align-items: center;
  gap: 5px;
  min-width: 78px;
  font-size: 13px;
  flex-shrink: 0;
}

.function-scope {
  padding: 0 4px;
  border-radius: 4px;
  font-size: 10px;
  opacity: 0.6;
  background: color-mix(in srgb, var(--settings-font, currentColor) 10%, transparent);
}

.function-keys {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 5px;
  flex: 1;
  min-width: 0;
}

.function-empty {
  font-size: 12px;
  opacity: 0.35;
}

/* 键位胶囊: 用中性描边承载, 主色留给真正的选中/告警状态。
   十来个胶囊全用主色描边会连成一片橙色, 反而看不出哪个键有冲突。 */
.key-chip {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 3px 8px;
  border-radius: 7px;
  border: 1px solid var(--settings-field-border);
  background: var(--settings-field-bg);
  color: inherit;
  font-size: 12px;
  font-variant-numeric: tabular-nums;
  cursor: pointer;
  transition: border-color 0.16s, background 0.16s, color 0.16s;
}

/* 悬停 = 即将解除绑定, 用危险色提示这一步是破坏性的。 */
.key-chip:hover {
  border-color: color-mix(in srgb, var(--color-danger, #d14949) 55%, transparent);
  background: color-mix(in srgb, var(--color-danger, #d14949) 12%, transparent);
  color: var(--color-danger, #d14949);
}

.key-chip-remove {
  font-size: 12px;
  opacity: 0.4;
}

.key-chip:hover .key-chip-remove {
  opacity: 1;
}

/* 冲突标记: 该键在当前模式下有多个动作, 只有第一个会执行。 */
.key-chip.conflict {
  border-color: color-mix(in srgb, var(--color-warning, #d4972d) 62%, transparent);
  background: color-mix(in srgb, var(--color-warning, #d4972d) 15%, transparent);
}

.key-chip-warn {
  display: inline-block;
  min-width: 12px;
  border-radius: 6px;
  background: var(--color-warning, #d4972d);
  color: var(--color-text-inverse, #fff);
  font-size: 9px;
  font-weight: 700;
  line-height: 12px;
  text-align: center;
}

.conflict-notice {
  margin-top: 8px;
  padding: 8px 10px;
  border-radius: 10px;
  background: color-mix(in srgb, var(--color-warning, #d4972d) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--color-warning, #d4972d) 34%, transparent);
  font-size: 12px;
  line-height: 1.6;
}

.conflict-line + .conflict-line {
  margin-top: 3px;
}

.conflict-key {
  padding: 0 4px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--settings-font, currentColor) 12%, transparent);
  font-weight: 600;
}

/* 录入按键: 点开后按任意键即可绑定。
   始终可见 —— 未绑定的功能全靠它作为入口, 藏起来会让那些行变成死角;
   但默认不画虚线框、只用低透明度, 因此不会像十四个虚线框那样连成噪音。 */
.key-capture {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 22px;
  height: 22px;
  padding: 0 5px;
  border-radius: 7px;
  border: 1px dashed transparent;
  background: transparent;
  color: inherit;
  font-size: 13px;
  line-height: 1;
  opacity: 0.42;
  cursor: pointer;
  white-space: nowrap;
  transition: opacity 0.16s, border-color 0.16s, background 0.16s;
}

.function-item:hover .key-capture,
.function-item:focus-within .key-capture {
  opacity: 0.75;
}

.key-capture:hover,
.key-capture:focus-visible {
  opacity: 1;
  border-color: color-mix(in srgb, var(--color-primary, #c97f3a) 50%, transparent);
  background: color-mix(in srgb, var(--color-primary, #c97f3a) 10%, transparent);
}

.key-capturing {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 3px 8px;
  border-radius: 7px;
  border: 1px solid color-mix(in srgb, var(--color-primary, #c97f3a) 55%, transparent);
  background: color-mix(in srgb, var(--color-primary, #c97f3a) 12%, transparent);
  font-size: 12px;
  animation: capture-pulse 1.4s ease-in-out infinite;
}

@keyframes capture-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.62; }
}

.key-capture-cancel {
  border: none;
  background: transparent;
  color: inherit;
  font-size: 11px;
  opacity: 0.6;
  cursor: pointer;
  padding: 0;
  text-decoration: underline;
}

.key-capture-cancel:hover {
  opacity: 1;
}

.voice-select {
  flex: 1;
  /* 长音色名（「Microsoft Yunxi Online (Natural) - Chinese (Mainland)」）在
     flex 项里会被 min-width:auto 顶到内容宽度，把设置面板顶出横向滚动条。
     这三条一起才能兜住：允许收缩 / 不超出父容器 / 溢出用省略号。 */
  min-width: 0;
  max-width: 100%;
  box-sizing: border-box;
  text-overflow: ellipsis;
  padding: 10px 12px;
  border-radius: 12px;
  border: 1px solid var(--settings-field-border);
  outline: none;
  background: var(--settings-field-bg);
  color: inherit;
  caret-color: var(--color-primary, #c97f3a);
  transition: background 0.18s, border-color 0.18s, box-shadow 0.18s;
}

.voice-select:hover {
  background: var(--settings-field-bg-hover);
  border-color: color-mix(in srgb, var(--settings-font) 28%, transparent);
}

.voice-select:focus {
  background: var(--settings-field-bg-hover);
  border-color: var(--color-primary, #c97f3a);
  box-shadow: 0 0 0 3px rgba(201, 127, 58, 0.16);
}

.voice-select::placeholder {
  color: var(--settings-field-placeholder);
}

.voice-select option {
  background: var(--settings-popup);
  color: var(--settings-font);
}

.read-settings.is-dark .voice-select {
  color-scheme: dark;
}

/* 在线朗读引擎 / Edge：下拉占满剩余宽度，右侧按钮靠右同排 */
.http-tts-picker,
.edge-voice-picker {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 10px;
}

.setting-hint {
  margin-top: -8px;
  padding-left: 90px;
  font-size: 12px;
  line-height: 1.5;
  opacity: 0.65;
}

/* Theme swatches */
.theme-swatches {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
}

.swatch {
  width: 34px;
  height: 34px;
  border-radius: 50%;
  border: 2px solid transparent;
  cursor: pointer;
  transition: all 0.2s;
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 1px 3px rgba(0,0,0,0.12);
}

.swatch:hover {
  transform: scale(1.1);
}

.swatch.active {
  border-color: var(--color-primary, #c97f3a);
}

.swatch svg {
  width: 16px;
  height: 16px;
  color: var(--color-primary, #c97f3a);
}

/* Button groups */
.btn-group {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.font-picker {
  display: grid;
  flex: 1;
  min-width: 0;
  gap: 10px;
}

.font-actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.custom-font-btn {
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.opt-btn.danger {
  border-color: color-mix(in srgb, #d14b45 55%, transparent);
  color: #c43f3a;
}

.opt-btn {
  min-height: 36px;
  padding: 6px 16px;
  border-radius: 20px;
  font-size: 13px;
  font-weight: 500;
  border: 1px solid rgba(0,0,0,0.12);
  background: transparent;
  color: inherit;
  cursor: pointer;
  transition: all 0.2s;
  white-space: nowrap;
}

.opt-btn:hover {
  border-color: var(--color-primary, #c97f3a);
}

.opt-btn:active:not(:disabled) {
  transform: scale(0.97);
}

.opt-btn:disabled {
  cursor: not-allowed;
  opacity: 0.45;
}

/* 选中态用淡主色底 + 主色文字与描边, 而不是整块纯橙 —— 纯色块在设置面板
   里一屏会出现十来处, 连成一片非常刺眼, 也压掉了文字的层次。 */
.opt-btn.active {
  background: color-mix(in srgb, var(--color-primary) 14%, transparent);
  /* 文字必须用 --settings-font(本组件跟随阅读主题的 fontColor), 不能写主色:
   * 主色 #c97f3a 与阅读主题的浅色底亮度太接近, 默认主题只有 2.4:1、暗灰主题
   * 更是 1.2:1, 字与底几乎同亮。改成跟随文字色后升到 6.4~10.9:1。 */
  color: var(--settings-font, currentColor);
  border-color: color-mix(in srgb, var(--color-primary) 34%, transparent);
  font-weight: 600;
}

.opt-btn.active:hover {
  background: color-mix(in srgb, var(--color-primary) 20%, transparent);
}

/* Steppers */
.stepper {
  display: flex;
  align-items: center;
  border-radius: 20px;
  border: 1px solid rgba(0,0,0,0.12);
  overflow: hidden;
  /* 段落行高/间距、朗读语速等都是「按内容的窄胶囊」。
     .setting-row 是 flex 行，不写这两条时 stepper 会被拉伸成整行宽的空框
     （表现就是「+ 按钮拉得老长」）。 */
  flex: 0 0 auto;
  width: fit-content;
}

/* Sliders
 *
 * 行结构是「.slider-head(标签 + 数值 + 可选回正按钮) + input[range]」，用 grid
 * 排成一行：标签 / 滑块 / 数值 / 回正，整组居中。
 * `display: contents` 把 .slider-head 拆掉，label、value、回正直接成为 grid item。
 *
 * **必须给每项写死 `grid-row: 1`**：grid 的自动放置是「稀疏」的 —— 按 DOM 顺序
 * 排完 label(列1)、value(列3)、回正(列4) 后，光标已停在第 1 行末尾，此时再给
 * 滑块指定 `grid-column: 2` 只会从光标后开始找，于是它被挤到**第 2 行**，
 * 表现为「标签和数值一行、滑块单独一行」。写死行号后四项才真正同排。
 */
.slider-row {
  display: grid;
  /* 第 1 列与 .setting-row 完全一致（104px）+ 同样的 14px 列间距，
     滑块左端才能和上面的按钮组、下拉框严丝合缝地对齐。 */
  grid-template-columns: 104px minmax(120px, 240px) 56px 64px;
  align-items: center;
  justify-content: start;
  column-gap: 14px;
  row-gap: 4px;
}

.slider-head {
  display: contents;
}

.slider-row .slider-head > label {
  grid-column: 1;
  grid-row: 1;
  /* 与普通行一致：左对齐，保证面板内只有一条文本竖线。 */
  text-align: left;
}

.slider-row > .settings-slider {
  grid-column: 2;
  grid-row: 1;
  width: 100%;
}

.slider-row .slider-head > .slider-value {
  grid-column: 3;
  grid-row: 1;
  /* 定宽 + 右对齐：数值位数变化时滑块不会左右抖动。 */
  min-width: 52px;
  text-align: right;
  justify-self: start;
}

.slider-row .slider-head > .link-btn {
  grid-column: 4;
  grid-row: 1;
  justify-self: start;
}

.slider-value {
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  opacity: 0.75;
}

.settings-slider {
  /* 宽面板上 100% 会拉到整行、拖动手感差；统一收窄到 240px（窄屏仍可缩）。
     数值由上面的 grid 紧跟滑块右侧，不再靠整行 right。 */
  width: min(240px, 100%);
  appearance: none;
  -webkit-appearance: none;
  height: 18px;
  margin: 0;
  background: transparent;
  cursor: pointer;
}

.settings-slider::-webkit-slider-runnable-track {
  height: 4px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--settings-font, currentColor) 20%, transparent);
}

.settings-slider::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 14px;
  height: 14px;
  margin-top: -5px;
  border-radius: 50%;
  background: var(--settings-font, currentColor);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.25);
}

.settings-slider::-moz-range-track {
  height: 4px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--settings-font, currentColor) 20%, transparent);
}

.settings-slider::-moz-range-thumb {
  width: 14px;
  height: 14px;
  border: none;
  border-radius: 50%;
  background: var(--settings-font, currentColor);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.25);
}

.step-btn {
  padding: 6px 14px;
  font-size: 13px;
  font-weight: 600;
  color: inherit;
  background: transparent;
  cursor: pointer;
  transition: background 0.15s;
  border: none;
  min-width: 40px;
}

.step-btn:hover {
  background: rgba(0,0,0,0.06);
}

.step-btn:active {
  background: rgba(0,0,0,0.1);
}

.step-val {
  min-width: 50px;
  text-align: center;
  font-size: 14px;
  font-variant-numeric: tabular-nums;
  border-left: 1px solid rgba(0,0,0,0.08);
  border-right: 1px solid rgba(0,0,0,0.08);
  padding: 6px 0;
}

@media (max-width: 420px) {
  .read-settings {
    padding: 16px;
  }

  .reader-settings-tabs {
    top: -16px;
  }

  .settings-header {
    align-items: center;
    gap: 8px;
  }

  .settings-title {
    font-size: 18px;
  }

  .reset-btn {
    padding: 6px 12px;
    font-size: 12px;
  }

  /* 窄屏：标签独占一行、控件整行铺开（88px + 控件在 360px 屏上太挤）。 */
  .setting-row,
  .slider-row {
    grid-template-columns: minmax(0, 1fr);
    row-gap: 8px;
  }

  .setting-row > label,
  .setting-row .slider-head > label {
    grid-column: 1 / -1;
    text-align: left;
    font-size: 13px;
  }

  .setting-row > .btn-group,
  .setting-row > .stepper,
  .setting-row > .font-picker,
  .setting-row > .voice-select,
  .setting-row > .opt-btn.wide,
  .setting-row > .image-picker-row,
  .setting-row > .inline-slider,
  .setting-row > .theme-swatches {
    grid-column: 1 / -1;
  }

  .btn-group {
    gap: 6px;
    flex: 0 1 auto;
  }

  .stepper,
  .voice-select,
  .theme-swatches {
    flex: 0 1 auto;
    min-width: 0;
    max-width: 100%;
  }

  .stepper {
    width: auto;
    display: inline-flex;
  }

  .opt-btn {
    padding: 6px 12px;
    font-size: 12px;
  }

  .step-btn {
    min-width: 36px;
    padding: 6px 10px;
  }

  .step-val {
    min-width: 42px;
    font-size: 13px;
  }

  .voice-select {
    width: auto;
  }

  .http-tts-picker,
  .edge-voice-picker {
    flex-wrap: wrap;
  }

  .setting-hint {
    padding-left: 0;
    margin-top: -12px;
  }
}
</style>
