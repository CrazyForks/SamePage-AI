<script setup lang="ts">
import type { BuddyLocale } from '@/i18n/buddyI18n'
import type { BuddyActionLogPlanStatus, BuddyActionLogResultKind } from '@/lib/tauriRuntime'
import { NButton, NSelect, NTag, NVirtualList } from 'naive-ui'
import { computed, onMounted, watch } from 'vue'
import { useBuddyI18n } from '@/i18n/buddyI18n'
import {
  createActionLogPlanRows,
  createActionLogResultKindOptions,
  createActionLogSourceOptions,
  createActionLogStatusOptions,
  createActionLogStepRows,
} from '@/panel/actionLogView'
import { useBuddyActionLog } from '@/panel/useBuddyActionLog'

const props = defineProps<{
  language: BuddyLocale
}>()

const emit = defineEmits<{
  updatePlanCount: [count: number]
}>()

const {
  errorMessage,
  hasMore,
  isLoadingDetail,
  isLoadingMore,
  isLoadingPlans,
  loadFirstPage,
  loadNextPage,
  plans,
  resultKindFilter,
  selectedPlanDetail,
  selectedPlanId,
  selectPlan,
  sourceRefKindFilter,
  statusFilter,
} = useBuddyActionLog()

const { locale, t } = useBuddyI18n(() => props.language)
const statusOptions = computed(() => createActionLogStatusOptions(t))
const sourceOptions = computed(() => createActionLogSourceOptions(t))
const resultKindOptions = computed(() => createActionLogResultKindOptions(t))
const planRows = computed(() => createActionLogPlanRows(plans.value, t, locale.value))
const stepRows = computed(() => createActionLogStepRows(selectedPlanDetail.value, t, locale.value))
const virtualPlanRows = computed(() => [...planRows.value])
const virtualStepRows = computed(() => [...stepRows.value])
const activePlanRow = computed(() =>
  selectedPlanId.value
    ? planRows.value.find(row => row.id === selectedPlanId.value) ?? null
    : null,
)
const isInitialLoading = computed(() => isLoadingPlans.value && planRows.value.length === 0)
const statusSelectValue = computed({
  get: () => statusFilter.value ?? 'all',
  set: (value: string) => {
    statusFilter.value = value === 'all' ? null : value as BuddyActionLogPlanStatus
  },
})
const sourceSelectValue = computed({
  get: () => sourceRefKindFilter.value ?? 'all',
  set: (value: string) => {
    sourceRefKindFilter.value = value === 'all' ? null : value
  },
})
const resultKindSelectValue = computed({
  get: () => resultKindFilter.value ?? 'all',
  set: (value: string) => {
    resultKindFilter.value = value === 'all' ? null : value as BuddyActionLogResultKind
  },
})

onMounted(() => {
  void loadFirstPage()
})

watch(
  () => [
    planRows.value.length,
    resultKindFilter.value,
    sourceRefKindFilter.value,
    statusFilter.value,
  ] as const,
  ([count, resultKind, sourceRefKind, status]) => {
    if (resultKind === null && sourceRefKind === null && status === null)
      emit('updatePlanCount', count)
  },
  { immediate: true },
)
</script>

<template>
  <section class="buddy-action-log">
    <header class="buddy-action-log__head">
      <div class="buddy-action-log__filters">
        <NSelect
          v-model:value="statusSelectValue"
          class="buddy-action-log__filter"
          :options="statusOptions"
          size="small"
        />
        <NSelect
          v-model:value="sourceSelectValue"
          class="buddy-action-log__filter"
          :options="sourceOptions"
          size="small"
        />
        <NSelect
          v-model:value="resultKindSelectValue"
          class="buddy-action-log__filter"
          :options="resultKindOptions"
          size="small"
        />
      </div>
    </header>

    <p
      v-if="errorMessage"
      class="buddy-action-log__error"
    >
      {{ errorMessage }}
    </p>

    <div
      v-if="planRows.length > 0"
      class="buddy-action-log__layout"
    >
      <aside class="buddy-action-log__plans">
        <NVirtualList
          class="buddy-action-log__virtual-list"
          :items="virtualPlanRows"
          :item-size="98"
          key-field="key"
        >
          <template #default="{ item }">
            <button
              class="buddy-action-log__plan"
              :class="{ 'is-active': selectedPlanId === item.id }"
              type="button"
              @click="selectPlan(item.id)"
            >
              <span class="buddy-action-log__plan-head">
                <strong>{{ item.title }}</strong>
                <NTag
                  round
                  size="small"
                  :type="item.statusType"
                >
                  {{ item.statusLabel }}
                </NTag>
              </span>
              <span class="buddy-action-log__plan-meta">
                {{ item.sourceLabel }}
                <template v-if="item.sourceDetailLabel">
                  ·
                  {{ item.sourceDetailLabel }}
                </template>
                /
                {{ item.startedAtLabel }}
              </span>
              <span class="buddy-action-log__plan-foot">
                <span>{{ t('actionLog.actionClip') }}</span>
                <span>{{ item.animationLabel }}</span>
              </span>
            </button>
          </template>
        </NVirtualList>

        <footer
          v-if="hasMore"
          class="buddy-action-log__footer"
        >
          <NButton
            block
            secondary
            size="small"
            :loading="isLoadingMore"
            @click="loadNextPage"
          >
            {{ t('actionLog.loadMore') }}
          </NButton>
        </footer>
      </aside>

      <article class="buddy-action-log__steps">
        <header
          v-if="activePlanRow"
          class="buddy-action-log__detail-head"
        >
          <div class="buddy-action-log__detail-title">
            <strong>{{ t('actionLog.stepTimeline') }}</strong>
            <small>
              {{ t('actionLog.reasonCode') }}
              {{ activePlanRow.reasonLabel }}
            </small>
          </div>
          <NTag
            round
            size="small"
            :type="activePlanRow.statusType"
          >
            {{ activePlanRow.statusLabel }}
          </NTag>
        </header>

        <NVirtualList
          v-if="stepRows.length > 0"
          class="buddy-action-log__virtual-list"
          :items="virtualStepRows"
          :item-size="98"
          key-field="key"
        >
          <template #default="{ item }">
            <section class="buddy-action-log__step">
              <span class="buddy-action-log__step-head">
                <strong>{{ item.actionLabel }}</strong>
                <NTag
                  round
                  size="small"
                  :type="item.statusType"
                >
                  {{ item.statusLabel }}
                </NTag>
              </span>
              <span class="buddy-action-log__step-meta">
                {{ item.clipLabel }}
                /
                {{ item.durationLabel }}
              </span>
              <span class="buddy-action-log__step-foot">
                {{ item.timeLabel }}
                ·
                {{ item.reasonLabel }}
              </span>
            </section>
          </template>
        </NVirtualList>

        <p
          v-else-if="isLoadingDetail"
          class="buddy-action-log__empty"
        >
          {{ t('actionLog.loading') }}
        </p>

        <p
          v-else
          class="buddy-action-log__empty"
        >
          {{ activePlanRow ? t('actionLog.noSteps') : t('actionLog.detailEmpty') }}
        </p>
      </article>
    </div>

    <p
      v-else
      class="buddy-action-log__empty buddy-action-log__empty--full"
    >
      {{ isInitialLoading ? t('actionLog.loading') : t('actionLog.empty') }}
    </p>
  </section>
</template>

<style scoped lang="scss">
.buddy-action-log {
  display: grid;
  grid-template-rows: auto auto minmax(0, 1fr);
  min-width: 0;
  height: 100%;
  min-height: 420px;
  overflow: hidden;
  border: 1px solid var(--buddy-border-light);
  border-radius: 8px;
  background: var(--buddy-bg-surface);
}

.buddy-action-log__head,
.buddy-action-log__detail-head {
  display: flex;
  gap: 14px;
  min-width: 0;
  border-bottom: 1px solid var(--buddy-border-light);
  padding: 14px 16px;
}

.buddy-action-log__head {
  align-items: center;
  justify-content: flex-end;
}

.buddy-action-log__detail-head {
  align-items: flex-start;
  justify-content: space-between;
}

.buddy-action-log__detail-title {
  display: grid;
  gap: 4px;
  min-width: 0;
}

.buddy-action-log__head strong,
.buddy-action-log__detail-head strong {
  color: var(--buddy-text-primary);
  font-size: 15px;
  font-weight: 600;
  line-height: 1.25;
}

.buddy-action-log__head small,
.buddy-action-log__detail-head small {
  color: var(--buddy-text-secondary);
  font-size: 12px;
  line-height: 1.4;
}

.buddy-action-log__filters {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  flex: 0 0 auto;
  flex-wrap: nowrap;
  gap: 8px;
}

.buddy-action-log__filter {
  flex: 0 0 156px;
  width: 156px;
}

.buddy-action-log__layout {
  grid-row: 3;
  display: grid;
  grid-template-columns: minmax(240px, 320px) minmax(0, 1fr);
  gap: 0;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.buddy-action-log__plans,
.buddy-action-log__steps {
  display: grid;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.buddy-action-log__plans {
  grid-template-rows: minmax(0, 1fr) auto;
  border-right: 1px solid var(--buddy-border-light);
}

.buddy-action-log__steps {
  grid-template-rows: auto minmax(0, 1fr);
}

.buddy-action-log__virtual-list {
  height: 100%;
  min-height: 0;
}

.buddy-action-log__plan {
  display: grid;
  gap: 7px;
  width: calc(100% - 16px);
  height: 86px;
  margin: 6px 8px;
  border: 1px solid transparent;
  border-radius: 7px;
  background: transparent;
  color: var(--buddy-text-primary);
  cursor: pointer;
  padding: 10px 11px;
  text-align: left;
}

.buddy-action-log__plan:hover,
.buddy-action-log__plan.is-active {
  border-color: color-mix(in srgb, var(--buddy-accent-primary) 42%, var(--buddy-border-light));
  background: color-mix(in srgb, var(--buddy-accent-primary) 8%, var(--buddy-bg-surface));
}

.buddy-action-log__plan-head,
.buddy-action-log__plan-foot,
.buddy-action-log__step-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  min-width: 0;
}

.buddy-action-log__plan-head strong,
.buddy-action-log__plan-foot span:last-child,
.buddy-action-log__step-head strong {
  overflow: hidden;
  min-width: 0;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.buddy-action-log__plan-head strong,
.buddy-action-log__step-head strong {
  font-size: 14px;
  font-weight: 600;
  line-height: 1.25;
}

.buddy-action-log__plan-meta,
.buddy-action-log__plan-foot,
.buddy-action-log__step-meta,
.buddy-action-log__step-foot {
  color: var(--buddy-text-secondary);
  font-size: 12px;
  line-height: 1.35;
}

.buddy-action-log__plan-meta {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.buddy-action-log__plan-foot span:last-child,
.buddy-action-log__step-meta,
.buddy-action-log__step-foot {
  font-family: var(--buddy-font-mono);
}

.buddy-action-log__footer {
  border-top: 1px solid var(--buddy-border-light);
  padding: 10px;
}

.buddy-action-log__step {
  display: grid;
  gap: 7px;
  height: 86px;
  margin: 6px 16px;
  border-bottom: 1px solid var(--buddy-border-light);
  padding: 10px 0 12px;
}

.buddy-action-log__error,
.buddy-action-log__empty {
  margin: 0;
  color: var(--buddy-text-secondary);
  font-size: 12px;
  line-height: 1.55;
}

.buddy-action-log__error {
  border-bottom: 1px solid color-mix(in srgb, var(--buddy-accent-danger) 24%, var(--buddy-border-light));
  background: color-mix(in srgb, var(--buddy-accent-danger) 7%, var(--buddy-bg-surface));
  color: var(--buddy-accent-danger);
  padding: 10px 16px;
}

.buddy-action-log__empty {
  display: grid;
  place-items: center;
  min-height: 100%;
  padding: 20px;
  text-align: center;
}

.buddy-action-log__empty--full {
  grid-row: 3;
  border-top: 1px solid var(--buddy-border-light);
}

@media (max-width: 980px) {
  .buddy-action-log__layout {
    grid-template-columns: 1fr;
    grid-template-rows: minmax(180px, 42%) minmax(0, 1fr);
  }

  .buddy-action-log__plans {
    border-right: 0;
    border-bottom: 1px solid var(--buddy-border-light);
  }
}
</style>
