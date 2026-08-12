<script setup lang="ts">
import type { DesktopAppInfo } from '../../electron/shared/desktopApi'
import type { DesktopChatController } from './useDesktopChat'
import { NAlert, NCard, NSpin } from 'naive-ui'
import { computed } from 'vue'
import { useBuddyI18n } from '@/i18n/buddyI18n'

const props = defineProps<{
  appInfo: DesktopAppInfo | null
  chat: DesktopChatController
}>()

const chat = props.chat
const { t } = useBuddyI18n(chat.language)
const pathRows = computed(() => {
  const paths = chat.localState.value?.paths
  if (!paths)
    return []

  return [
    { label: t('settings.configDir'), value: props.appInfo?.configPath ?? '-' },
    { label: t('settings.dataDir'), value: paths.dataDir },
    { label: t('settings.conversationsDir'), value: paths.conversationsDir },
    { label: t('settings.runsDir'), value: paths.runsDir },
    { label: t('settings.memoriesDir'), value: paths.memoriesDir },
    { label: t('settings.sqliteDir'), value: paths.sqliteDir },
    { label: t('settings.logDir'), value: paths.logDir },
  ]
})
</script>

<template>
  <section class="desktop-directories-settings">
    <NAlert v-if="chat.localStateError.value" type="error" :show-icon="false">
      {{ chat.localStateError.value }}
    </NAlert>

    <NCard size="small">
      <div v-if="chat.isLoadingLocalState.value && !chat.localState.value" class="desktop-directories-settings__loading">
        <NSpin size="small" />
      </div>
      <template v-else>
        <div v-for="row in pathRows" :key="row.label" class="desktop-directories-settings__row">
          <span>{{ row.label }}</span>
          <code :title="row.value">{{ row.value }}</code>
        </div>
        <div v-if="chat.localState.value" class="desktop-directories-settings__row">
          <span>{{ t('desktop.settings.schemaVersion') }}</span>
          <code>{{ chat.localState.value.storage.schemaVersion }}</code>
        </div>
      </template>
    </NCard>
  </section>
</template>

<style scoped lang="scss">
.desktop-directories-settings {
  display: grid;
  gap: 1rem;

  :deep(.n-card) {
    border-color: var(--buddy-border-light);
    background: var(--buddy-bg-surface-raised);
  }
}

.desktop-directories-settings__loading {
  display: grid;
  min-height: 6rem;
  place-items: center;
}

.desktop-directories-settings__row {
  display: grid;
  min-height: 3.2rem;
  grid-template-columns: minmax(9rem, 12rem) minmax(0, 1fr);
  align-items: center;
  gap: 1rem;
  border-bottom: 1px solid var(--buddy-border-light);
  color: var(--buddy-text-regular);
  font-size: 0.78rem;

  &:last-child {
    border-bottom: 0;
  }

  code {
    overflow: hidden;
    color: var(--buddy-text-secondary);
    font-family: var(--buddy-font-mono);
    font-size: 0.7rem;
    text-align: right;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
}
</style>
