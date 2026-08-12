<script setup lang="ts">
import type { DesktopChatController } from './useDesktopChat'
import { ArrowClockwise20Regular, ArrowLeft20Regular } from '@vicons/fluent'
import { NAlert, NButton, NIcon, NSkeleton, NTag, NTooltip } from 'naive-ui'
import { computed, shallowRef } from 'vue'
import codexIconUrl from '@/assets/brand/codex.svg'
import { useBuddyI18n } from '@/i18n/buddyI18n'
import {
  createDesktopAgentUsage,
  resolveDesktopUsagePresentation,
} from './desktopAgentUsage'
import DesktopAgentUsageCalendar from './DesktopAgentUsageCalendar.vue'
import DesktopCodexSettingsSection from './DesktopCodexSettingsSection.vue'

const props = defineProps<{
  chat: DesktopChatController
}>()

const emit = defineEmits<{
  back: []
}>()

const chat = props.chat
const { t } = useBuddyI18n(chat.language)
const isManualRefreshing = shallowRef(false)
const codexUsage = computed(() => createDesktopAgentUsage(chat.usageSnapshot.value, 'codex'))
const usagePresentation = computed(() => resolveDesktopUsagePresentation({
  hasError: chat.usageError.value !== null,
  hasSnapshot: chat.usageSnapshot.value !== null,
  isLoading: chat.isLoadingUsage.value,
}))
const usageStatus = computed(() => {
  if (usagePresentation.value === 'initial-loading')
    return { error: false, label: t('desktop.agent.collectingUsage') }
  if (usagePresentation.value === 'refreshing')
    return { error: false, label: t('desktop.agent.updatingUsage') }
  if (usagePresentation.value === 'stale-error' || usagePresentation.value === 'empty-error')
    return { error: true, label: t('desktop.agent.usageUpdateFailed') }

  return null
})
const isInitialUsageLoading = computed(() => usagePresentation.value === 'initial-loading')
const hasUsageSnapshot = computed(() => chat.usageSnapshot.value !== null)
const isAvailable = computed(() =>
  chat.runtimeState.value.status === 'ready'
  && chat.codexStatus.value?.activeProtocol !== 'unavailable',
)
const statusRows = computed(() => {
  const status = chat.codexStatus.value
  return [
    {
      label: t('runtime.codexCli'),
      value: status?.cliAvailable ? status.version ?? t('common.detected') : t('common.undetected'),
    },
    { label: t('runtime.loginStatus'), value: formatLoginStatus(status?.loginStatus) },
    { label: t('runtime.appServer'), value: status?.appServerAvailable ? t('common.available') : t('common.unavailable') },
    { label: t('runtime.execution'), value: formatCodexProtocol(status?.activeProtocol) },
  ]
})
const metrics = computed(() => [
  { label: t('usage.totalTokens'), value: codexUsage.value.totals.totalTokens },
  { label: t('usage.inputTokens'), value: codexUsage.value.totals.inputTokens },
  { label: t('usage.outputTokens'), value: codexUsage.value.totals.outputTokens },
  {
    label: t('usage.cacheTokens'),
    value: codexUsage.value.totals.cacheCreationTokens + codexUsage.value.totals.cacheReadTokens,
  },
])

async function refresh() {
  if (isManualRefreshing.value)
    return

  isManualRefreshing.value = true
  try {
    await Promise.all([
      chat.loadAgent(true),
      chat.loadUsage(true),
    ])
  }
  finally {
    isManualRefreshing.value = false
  }
}

function formatTokens(value: number, exact = false) {
  return new Intl.NumberFormat(chat.language.value, exact
    ? {}
    : {
        maximumFractionDigits: 1,
        notation: value >= 10_000 ? 'compact' : 'standard',
      }).format(value)
}

function formatLoginStatus(status: 'logged_in' | 'logged_out' | 'unknown' | 'unavailable' | undefined) {
  if (status === 'logged_in')
    return t('common.loggedIn')
  if (status === 'logged_out')
    return t('common.loggedOut')
  if (status === 'unavailable')
    return t('common.unavailable')

  return t('common.missing')
}

function formatCodexProtocol(protocol: 'codex_app_server' | 'codex_exec_json_fallback' | 'unavailable' | undefined) {
  if (protocol === 'codex_app_server')
    return 'App Server'
  if (protocol === 'codex_exec_json_fallback')
    return 'exec --json'

  return t('common.unavailable')
}
</script>

<template>
  <div class="desktop-codex-agent">
    <header class="desktop-codex-agent__header">
      <div class="desktop-codex-agent__heading">
        <NButton quaternary circle :aria-label="t('desktop.agent.back')" @click="emit('back')">
          <template #icon>
            <NIcon :component="ArrowLeft20Regular" />
          </template>
        </NButton>
        <img :src="codexIconUrl" alt="">
        <div>
          <h1>Codex</h1>
          <p>{{ t('desktop.agent.codexDescription') }}</p>
        </div>
      </div>
      <NButton
        circle
        quaternary
        :loading="isManualRefreshing"
        :aria-label="t('desktop.agent.refresh')"
        @click="refresh"
      >
        <template #icon>
          <NIcon :component="ArrowClockwise20Regular" />
        </template>
      </NButton>
    </header>

    <section class="desktop-codex-agent__section" aria-labelledby="codex-status-title">
      <div class="desktop-codex-agent__section-title">
        <div>
          <h2 id="codex-status-title">
            {{ t('desktop.agent.statusTitle') }}
          </h2>
          <p>{{ t('desktop.agent.statusDescription') }}</p>
        </div>
        <NTag :bordered="false" size="small" :type="isAvailable ? 'success' : 'warning'">
          {{ isAvailable ? t('common.available') : t('common.unavailable') }}
        </NTag>
      </div>
      <NAlert v-if="chat.agentError.value" type="error" :show-icon="false">
        {{ chat.agentError.value }}
      </NAlert>
      <dl class="desktop-codex-agent__status">
        <div v-for="row in statusRows" :key="row.label">
          <dt>{{ row.label }}</dt>
          <dd :aria-busy="chat.isLoadingAgent.value && !chat.codexStatus.value">
            <NSkeleton v-if="chat.isLoadingAgent.value && !chat.codexStatus.value" text />
            <span v-else>{{ row.value }}</span>
          </dd>
        </div>
      </dl>
    </section>

    <section class="desktop-codex-agent__section" aria-labelledby="codex-usage-title">
      <div class="desktop-codex-agent__section-title">
        <div>
          <h2 id="codex-usage-title">
            {{ t('desktop.agent.usageDetailTitle') }}
          </h2>
          <p>{{ t('desktop.agent.usageDetailDescription') }}</p>
        </div>
        <div
          v-if="usageStatus"
          class="desktop-codex-agent__usage-status"
          :class="{ 'is-error': usageStatus.error }"
          role="status"
          aria-live="polite"
        >
          <i v-if="!usageStatus.error" aria-hidden="true" />
          <span>{{ usageStatus.label }}</span>
          <button
            v-if="usageStatus.error"
            class="desktop-codex-agent__retry"
            type="button"
            :title="chat.usageError.value ?? undefined"
            @click="refresh"
          >
            {{ t('desktop.agent.retry') }}
          </button>
        </div>
      </div>
      <NAlert v-if="usagePresentation === 'empty-error'" type="error" :show-icon="false">
        {{ chat.usageError.value }}
      </NAlert>
      <dl class="desktop-codex-agent__metrics">
        <NTooltip v-for="metric in metrics" :key="metric.label" :disabled="!hasUsageSnapshot">
          <template #trigger>
            <div>
              <dt>{{ metric.label }}</dt>
              <dd :aria-busy="isInitialUsageLoading">
                <NSkeleton v-if="isInitialUsageLoading" text />
                <span v-else-if="!hasUsageSnapshot">-</span>
                <span v-else>{{ formatTokens(metric.value) }}</span>
              </dd>
            </div>
          </template>
          {{ formatTokens(metric.value, true) }} tokens
        </NTooltip>
      </dl>
      <div v-if="isInitialUsageLoading" class="desktop-codex-agent__calendar-loading" aria-busy="true">
        <NSkeleton height="22rem" />
      </div>
      <DesktopAgentUsageCalendar
        v-else
        :language="chat.language.value"
        :snapshot="chat.usageSnapshot.value"
      />
    </section>

    <DesktopCodexSettingsSection :chat="chat" />
  </div>
</template>

<style scoped lang="scss">
.desktop-codex-agent {
  display: grid;
  max-width: 72rem;
  gap: 2rem;
  margin: 0 auto;
  padding-bottom: 3rem;
}

.desktop-codex-agent__header,
.desktop-codex-agent__heading,
.desktop-codex-agent__section-title {
  display: flex;
  align-items: flex-start;
}

.desktop-codex-agent__header,
.desktop-codex-agent__section-title {
  justify-content: space-between;
  gap: 2rem;
}

.desktop-codex-agent__heading {
  align-items: center;
  gap: 0.8rem;

  img {
    width: 2.75rem;
    height: 2.75rem;
    border-radius: 0.75rem;
    background: #111614;
    object-fit: contain;
    padding: 0.5rem;
  }

  h1,
  p {
    margin: 0;
  }

  h1 {
    font-size: clamp(1.75rem, 3vw, 2.35rem);
    letter-spacing: -0.045em;
  }

  p {
    margin-top: 0.25rem;
    color: var(--buddy-text-secondary);
    font-size: 0.8rem;
  }
}

.desktop-codex-agent__section {
  display: grid;
  gap: 1rem;
}

.desktop-codex-agent__section-title h2,
.desktop-codex-agent__section-title p {
  margin: 0;
}

.desktop-codex-agent__usage-status {
  display: inline-flex;
  align-items: center;
  gap: 0.45rem;
  color: var(--buddy-text-secondary);
  font-size: 0.72rem;
  line-height: 1.4;

  > i {
    width: 0.5rem;
    height: 0.5rem;
    flex: none;
    border-radius: 50%;
    background: var(--buddy-accent-primary);
    animation: desktop-codex-usage-pulse 1.4s ease-in-out infinite;
  }

  &.is-error {
    color: var(--buddy-accent-danger);
  }
}

.desktop-codex-agent__retry {
  border: 0;
  border-bottom: 1px solid currentcolor;
  background: transparent;
  color: inherit;
  cursor: pointer;
  padding: 0;
  font: inherit;
}

.desktop-codex-agent__section-title h2 {
  font-size: 1.05rem;
}

.desktop-codex-agent__section-title p {
  margin-top: 0.3rem;
  color: var(--buddy-text-secondary);
  font-size: 0.76rem;
}

.desktop-codex-agent__status,
.desktop-codex-agent__metrics {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 0.75rem;
  margin: 0;

  > div {
    display: grid;
    gap: 0.4rem;
    border: 1px solid var(--buddy-border-light);
    border-radius: 0.8rem;
    background: var(--buddy-bg-surface-raised);
    padding: 0.95rem;
  }

  dt {
    color: var(--buddy-text-secondary);
    font-size: 0.7rem;
  }

  dd {
    overflow: hidden;
    margin: 0;
    color: var(--buddy-text-primary);
    font-family: var(--buddy-font-mono);
    font-size: 0.8rem;
    font-weight: 650;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  :deep(.n-skeleton) {
    width: min(6rem, 70%);
  }
}

.desktop-codex-agent__metrics dd {
  font-size: 1.3rem;
  letter-spacing: -0.04em;
}

.desktop-codex-agent__calendar-loading {
  min-height: 22rem;
  border: 1px solid var(--buddy-border-light);
  border-radius: 0.8rem;
  background: var(--buddy-bg-surface-raised);
  padding: 1rem;
}

@media (max-width: 760px) {
  .desktop-codex-agent__status,
  .desktop-codex-agent__metrics {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (prefers-reduced-motion: reduce) {
  .desktop-codex-agent__usage-status > i {
    animation: none;
  }
}

@keyframes desktop-codex-usage-pulse {
  0%,
  100% {
    opacity: 0.35;
    transform: scale(0.82);
  }

  50% {
    opacity: 1;
    transform: scale(1);
  }
}
</style>
