<script setup lang="ts">
import type { DesktopAppInfo } from '../../electron/shared/desktopApi'
import type { DesktopSettingsTab } from './desktopViewState'
import type { DesktopChatController } from './useDesktopChat'
import { NTabPane, NTabs } from 'naive-ui'
import { onMounted, shallowRef } from 'vue'
import { useBuddyI18n } from '@/i18n/buddyI18n'
import DesktopDirectoriesSettingsTab from './DesktopDirectoriesSettingsTab.vue'
import DesktopGeneralSettingsTab from './DesktopGeneralSettingsTab.vue'
import DesktopRunLogSection from './DesktopRunLogSection.vue'

const props = defineProps<{
  activeTab: DesktopSettingsTab
  chat: DesktopChatController
}>()
const emit = defineEmits<{
  navigateTab: [tab: DesktopSettingsTab]
}>()

const chat = props.chat
const { t } = useBuddyI18n(chat.language)
const appInfo = shallowRef<DesktopAppInfo | null>(null)

onMounted(async () => {
  try {
    appInfo.value = await chat.getAppInfo()
  }
  catch {
    appInfo.value = null
  }
})

function navigateTab(value: string | number) {
  if (value === 'general' || value === 'directories' || value === 'logs')
    emit('navigateTab', value)
}
</script>

<template>
  <section class="desktop-settings-page">
    <header class="desktop-settings-page__header">
      <h1>{{ t('page.settings') }}</h1>
    </header>

    <NTabs
      class="desktop-settings-page__tabs"
      :value="activeTab"
      type="line"
      :animated="false"
      @update:value="navigateTab"
    >
      <NTabPane display-directive="if" name="general" :tab="t('desktop.settings.tabGeneral')">
        <DesktopGeneralSettingsTab :app-info="appInfo" :chat="chat" />
      </NTabPane>
      <NTabPane display-directive="if" name="directories" :tab="t('desktop.settings.tabDirectories')">
        <DesktopDirectoriesSettingsTab :app-info="appInfo" :chat="chat" />
      </NTabPane>
      <NTabPane display-directive="if" name="logs" :tab="t('desktop.settings.tabLogs')">
        <DesktopRunLogSection :chat="chat" />
      </NTabPane>
    </NTabs>
  </section>
</template>

<style scoped lang="scss">
.desktop-settings-page {
  width: 100%;
  height: 100%;
  overflow: auto;
  background: var(--buddy-bg-body);
  padding: clamp(1.5rem, 3vw, 3rem);
}

.desktop-settings-page__header,
.desktop-settings-page__tabs {
  max-width: 68rem;
  margin-right: auto;
  margin-left: auto;
}

.desktop-settings-page__header {
  margin-bottom: 1.35rem;
}

.desktop-settings-page__header h1 {
  margin: 0;
  color: var(--buddy-text-primary);
  font-size: clamp(1.75rem, 3vw, 2.35rem);
  letter-spacing: -0.045em;
}

.desktop-settings-page__tabs {
  --n-tab-gap: 1.5rem;
}

.desktop-settings-page__tabs :deep(.n-tabs-nav) {
  position: sticky;
  z-index: 3;
  top: -1px;
  background: var(--buddy-bg-body);
  padding-top: 0.15rem;
}

.desktop-settings-page__tabs :deep(.n-tab-pane) {
  padding-top: 1.4rem;
  padding-bottom: 3rem;
}
</style>
