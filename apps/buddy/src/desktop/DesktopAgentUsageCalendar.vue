<script setup lang="ts">
import type { Component } from 'vue'
import type { LocalUsageSnapshot } from '../../electron/shared/localChatApi'
import type { BuddyLocale } from '@/i18n/buddyI18n'
import {
  ArrowDownload20Regular,
  ArrowSyncCircle20Regular,
  ArrowUpload20Regular,
  DataUsage20Regular,
} from '@vicons/fluent'
import { NCalendar, NEmpty, NIcon, NTooltip } from 'naive-ui'
import { computed, shallowRef, watch } from 'vue'
import { useBuddyI18n } from '@/i18n/buddyI18n'
import { createDesktopAgentUsage } from './desktopAgentUsage'

type UsageMetricKey = 'total' | 'input' | 'output' | 'cache'

interface UsageMetric {
  exactValue: string
  icon: Component
  key: UsageMetricKey
  label: string
  value: string
}

const props = defineProps<{
  language: BuddyLocale
  snapshot: LocalUsageSnapshot | null
}>()

const { t } = useBuddyI18n(() => props.language)
const calendarValue = shallowRef(Date.now())
const usage = computed(() => createDesktopAgentUsage(props.snapshot, 'codex'))
let hasCenteredCalendar = false

watch(
  () => usage.value.latestDate,
  (date) => {
    if (!date || hasCenteredCalendar)
      return

    const timestamp = new Date(`${date}T12:00:00`).getTime()
    if (!Number.isNaN(timestamp)) {
      calendarValue.value = timestamp
      hasCenteredCalendar = true
    }
  },
  { immediate: true },
)

function resolveDateKey(year: number, month: number, date: number) {
  return [
    String(year).padStart(4, '0'),
    String(month).padStart(2, '0'),
    String(date).padStart(2, '0'),
  ].join('-')
}

function resolveDayMetrics(year: number, month: number, date: number): UsageMetric[] {
  const totals = usage.value.daily.get(resolveDateKey(year, month, date))
  if (!totals)
    return []

  return [
    createMetric('total', t('usage.totalTokens'), totals.totalTokens, DataUsage20Regular),
    createMetric('input', t('usage.inputTokens'), totals.inputTokens, ArrowUpload20Regular),
    createMetric('output', t('usage.outputTokens'), totals.outputTokens, ArrowDownload20Regular),
    createMetric(
      'cache',
      t('usage.cacheTokens'),
      totals.cacheCreationTokens + totals.cacheReadTokens,
      ArrowSyncCircle20Regular,
    ),
  ]
}

function createMetric(key: UsageMetricKey, label: string, value: number, icon: Component): UsageMetric {
  return {
    exactValue: `${new Intl.NumberFormat(props.language).format(value)} tokens`,
    icon,
    key,
    label,
    value: formatCompactTokens(value),
  }
}

function formatCompactTokens(value: number) {
  if (value < 1000)
    return new Intl.NumberFormat(props.language).format(value)

  return new Intl.NumberFormat(props.language, {
    maximumFractionDigits: 1,
    notation: 'compact',
  }).format(value)
}
</script>

<template>
  <section class="desktop-agent-usage-calendar" aria-label="Codex token calendar">
    <NEmpty
      v-if="usage.records.length === 0"
      size="small"
      :description="t('usage.noTokenRecords')"
    />
    <NCalendar v-else v-model:value="calendarValue" class="desktop-agent-usage-calendar__grid">
      <template #default="{ year, month, date }">
        <NTooltip v-if="usage.daily.has(resolveDateKey(year, month, date))" placement="top">
          <template #trigger>
            <div
              class="desktop-agent-usage-calendar__cell"
              :aria-label="resolveDateKey(year, month, date)"
            >
              <ul>
                <li v-for="metric in resolveDayMetrics(year, month, date)" :key="metric.key">
                  <NIcon :aria-label="metric.label" :component="metric.icon" :size="13" />
                  <strong>{{ metric.value }}</strong>
                </li>
              </ul>
            </div>
          </template>
          <div class="desktop-agent-usage-calendar__tooltip">
            <strong>{{ resolveDateKey(year, month, date) }}</strong>
            <span v-for="metric in resolveDayMetrics(year, month, date)" :key="metric.key">
              {{ metric.label }}：{{ metric.exactValue }}
            </span>
          </div>
        </NTooltip>
      </template>
    </NCalendar>
  </section>
</template>

<style scoped lang="scss">
.desktop-agent-usage-calendar {
  display: grid;
  min-height: 36rem;
  border: 1px solid var(--buddy-border-light);
  border-radius: 0.85rem;
  background: var(--buddy-bg-surface-raised);
  padding: 1rem;

  > :deep(.n-empty) {
    place-self: center;
  }
}

.desktop-agent-usage-calendar__grid {
  min-height: 38rem;

  --n-border-color: var(--buddy-border-light);
  --n-title-font-size: 1rem;
  --n-date-color-current: var(--buddy-accent-primary);
  --n-bar-color: transparent;
  --n-cell-color: transparent;
  --n-cell-color-hover: color-mix(in srgb, var(--buddy-accent-primary) 8%, transparent);

  :deep(.n-calendar-cell--selected) {
    background: color-mix(in srgb, var(--buddy-accent-primary) 13%, transparent);
  }

  :deep(.n-calendar-cell__bar) {
    display: none;
  }
}

.desktop-agent-usage-calendar__cell {
  position: absolute;
  inset: 2.25rem 0.5rem 0.45rem;
  display: grid;
  min-height: 0;
  place-items: center;

  ul {
    display: grid;
    gap: 0.28rem;
    margin: 0;
    padding: 0;
  }

  li {
    display: grid;
    min-width: 0;
    grid-template-columns: 0.9rem minmax(0, max-content);
    align-items: center;
    gap: 0.3rem;
    color: var(--buddy-accent-primary);
    list-style: none;
  }

  strong {
    overflow: hidden;
    min-width: 0;
    color: var(--buddy-accent-primary);
    font-family: var(--buddy-font-mono);
    font-size: 0.68rem;
    font-weight: 650;
    line-height: 1.1;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
}

.desktop-agent-usage-calendar__tooltip {
  display: grid;
  gap: 0.28rem;
  font-size: 0.76rem;

  strong {
    margin-bottom: 0.18rem;
  }
}
</style>
