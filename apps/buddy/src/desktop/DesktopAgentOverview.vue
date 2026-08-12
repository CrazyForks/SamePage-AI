<script setup lang="ts">
import type { DesktopChatController } from './useDesktopChat'
import { ArrowClockwise20Regular, ChevronRight20Regular } from '@vicons/fluent'
import { NAlert, NButton, NIcon, NSkeleton, NTag, NTooltip } from 'naive-ui'
import { computed, shallowRef } from 'vue'
import codexIconUrl from '@/assets/brand/codex.svg'
import { useBuddyI18n } from '@/i18n/buddyI18n'
import {
  createDesktopAgentUsage,
  resolveDesktopUsagePresentation,
} from './desktopAgentUsage'

const props = defineProps<{
  chat: DesktopChatController
}>()

const emit = defineEmits<{
  openCodex: []
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

function formatDate(value: string | null) {
  if (!value)
    return t('desktop.agent.neverUsed')

  const date = new Date(`${value}T12:00:00`)
  return Number.isNaN(date.getTime())
    ? value
    : new Intl.DateTimeFormat(chat.language.value, { dateStyle: 'medium' }).format(date)
}
</script>

<template>
  <div class="desktop-agent-overview">
    <header class="desktop-agent-overview__header">
      <div>
        <h1>{{ t('page.agent') }}</h1>
        <p>{{ t('desktop.agent.description') }}</p>
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

    <section class="desktop-agent-overview__usage" aria-labelledby="agent-usage-title">
      <div class="desktop-agent-overview__section-title">
        <div>
          <h2 id="agent-usage-title">
            {{ t('desktop.agent.usageTitle') }}
          </h2>
          <p>{{ t('desktop.agent.usageDescription') }}</p>
        </div>
        <div
          v-if="usageStatus"
          class="desktop-agent-overview__usage-status"
          :class="{ 'is-error': usageStatus.error }"
          role="status"
          aria-live="polite"
        >
          <i v-if="!usageStatus.error" aria-hidden="true" />
          <span>{{ usageStatus.label }}</span>
          <button
            v-if="usageStatus.error"
            class="desktop-agent-overview__retry"
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
      <dl class="desktop-agent-overview__metrics">
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
    </section>

    <section class="desktop-agent-overview__agents" aria-labelledby="agent-list-title">
      <div class="desktop-agent-overview__section-title">
        <div>
          <h2 id="agent-list-title">
            {{ t('desktop.agent.agentsTitle') }}
          </h2>
          <p>{{ t('desktop.agent.agentsDescription') }}</p>
        </div>
      </div>

      <NAlert v-if="chat.agentError.value" type="error" :show-icon="false">
        {{ chat.agentError.value }}
      </NAlert>

      <button
        class="desktop-agent-card"
        type="button"
        :aria-label="t('desktop.agent.openCodex')"
        @click="emit('openCodex')"
      >
        <div class="desktop-agent-card__identity">
          <img :src="codexIconUrl" alt="">
          <div>
            <strong>Codex</strong>
            <span>{{ t('runtime.codexDescription') }}</span>
          </div>
        </div>
        <NTag :bordered="false" size="small" :type="isAvailable ? 'success' : 'warning'">
          {{ isAvailable ? t('common.available') : t('common.unavailable') }}
        </NTag>
        <dl class="desktop-agent-card__facts">
          <div>
            <dt>{{ t('desktop.agent.cliVersion') }}</dt>
            <dd v-if="chat.isLoadingAgent.value && !chat.codexStatus.value">
              <NSkeleton text />
            </dd>
            <dd v-else>
              {{ chat.codexStatus.value?.version ?? '-' }}
            </dd>
          </div>
          <div>
            <dt>{{ t('desktop.agent.lastUsed') }}</dt>
            <dd v-if="isInitialUsageLoading">
              <NSkeleton text />
            </dd>
            <dd v-else-if="!hasUsageSnapshot">
              -
            </dd>
            <dd v-else>
              {{ formatDate(codexUsage.latestDate) }}
            </dd>
          </div>
          <div>
            <dt>{{ t('usage.totalTokens') }}</dt>
            <dd v-if="isInitialUsageLoading">
              <NSkeleton text />
            </dd>
            <dd v-else-if="!hasUsageSnapshot">
              -
            </dd>
            <dd v-else>
              {{ formatTokens(codexUsage.totals.totalTokens) }}
            </dd>
          </div>
        </dl>
        <NIcon class="desktop-agent-card__arrow" :component="ChevronRight20Regular" />
      </button>
    </section>
  </div>
</template>

<style scoped lang="scss">
.desktop-agent-overview {
  display: grid;
  max-width: 68rem;
  gap: 2rem;
  margin: 0 auto;
  padding-bottom: 3rem;
}

.desktop-agent-overview__header,
.desktop-agent-overview__section-title {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 2rem;
}

.desktop-agent-overview__header h1,
.desktop-agent-overview__header p,
.desktop-agent-overview__section-title h2,
.desktop-agent-overview__section-title p {
  margin: 0;
}

.desktop-agent-overview__usage-status {
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
    animation: desktop-agent-usage-pulse 1.4s ease-in-out infinite;
  }

  &.is-error {
    color: var(--buddy-accent-danger);
  }
}

.desktop-agent-overview__retry {
  border: 0;
  border-bottom: 1px solid currentcolor;
  background: transparent;
  color: inherit;
  cursor: pointer;
  padding: 0;
  font: inherit;
}

.desktop-agent-overview__header h1 {
  font-size: clamp(1.75rem, 3vw, 2.35rem);
  letter-spacing: -0.045em;
}

.desktop-agent-overview__header p {
  max-width: 40rem;
  margin-top: 0.5rem;
  color: var(--buddy-text-secondary);
  font-size: 0.86rem;
  line-height: 1.65;
}

.desktop-agent-overview__usage,
.desktop-agent-overview__agents {
  display: grid;
  gap: 1rem;
}

.desktop-agent-overview__section-title h2 {
  font-size: 1.05rem;
}

.desktop-agent-overview__section-title p {
  margin-top: 0.3rem;
  color: var(--buddy-text-secondary);
  font-size: 0.76rem;
}

.desktop-agent-overview__metrics {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 0.75rem;
  margin: 0;

  > div {
    display: grid;
    gap: 0.45rem;
    border: 1px solid var(--buddy-border-light);
    border-radius: 0.8rem;
    background: var(--buddy-bg-surface-raised);
    padding: 1rem;
  }

  dt {
    color: var(--buddy-text-secondary);
    font-size: 0.72rem;
  }

  dd {
    margin: 0;
    font-family: var(--buddy-font-mono);
    font-size: 1.35rem;
    font-weight: 650;
    letter-spacing: -0.04em;
  }

  :deep(.n-skeleton) {
    width: min(6rem, 70%);
    height: 1.35rem;
  }
}

.desktop-agent-card {
  display: grid;
  width: 100%;
  grid-template-columns: minmax(14rem, 1.15fr) auto minmax(25rem, 1.85fr) auto;
  align-items: center;
  gap: 1.25rem;
  border: 1px solid var(--buddy-border-light);
  border-radius: 0.9rem;
  background: var(--buddy-bg-surface-raised);
  color: var(--buddy-text-regular);
  cursor: pointer;
  padding: 1.15rem 1.25rem;
  text-align: left;
  transition: border-color 150ms ease, box-shadow 150ms ease, transform 150ms ease;

  &:hover {
    border-color: var(--buddy-border-base);
    box-shadow: var(--buddy-shadow-raised);
    transform: translateY(-1px);
  }

  &:focus-visible {
    outline: 2px solid var(--buddy-accent-primary);
    outline-offset: 2px;
  }
}

.desktop-agent-card__identity {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 0.85rem;

  img {
    width: 2.5rem;
    height: 2.5rem;
    border-radius: 0.7rem;
    background: #111614;
    object-fit: contain;
    padding: 0.45rem;
  }

  div {
    display: grid;
    min-width: 0;
    gap: 0.2rem;
  }

  strong {
    color: var(--buddy-text-primary);
    font-size: 0.95rem;
  }

  span {
    overflow: hidden;
    color: var(--buddy-text-secondary);
    font-size: 0.7rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
}

.desktop-agent-card__facts {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 1rem;
  margin: 0;

  div {
    min-width: 0;
  }

  dt {
    color: var(--buddy-text-secondary);
    font-size: 0.66rem;
  }

  dd {
    min-height: 1.15rem;
    overflow: hidden;
    margin: 0.25rem 0 0;
    color: var(--buddy-text-primary);
    font-family: var(--buddy-font-mono);
    font-size: 0.73rem;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  :deep(.n-skeleton) {
    width: 4.5rem;
  }
}

.desktop-agent-card__arrow {
  color: var(--buddy-text-placeholder);
}

@media (max-width: 940px) {
  .desktop-agent-card {
    grid-template-columns: minmax(0, 1fr) auto;
  }

  .desktop-agent-card__facts {
    grid-column: 1 / -1;
    grid-row: 2;
  }

  .desktop-agent-card__arrow {
    grid-column: 2;
    grid-row: 1;
  }
}

@media (max-width: 760px) {
  .desktop-agent-overview__metrics {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (prefers-reduced-motion: reduce) {
  .desktop-agent-overview__usage-status > i {
    animation: none;
  }
}

@keyframes desktop-agent-usage-pulse {
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
