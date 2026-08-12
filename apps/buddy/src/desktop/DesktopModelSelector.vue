<script setup lang="ts">
import type { LocalRuntimeModelOption } from '../../electron/shared/localChatApi'
import type { BuddyLocale } from '@/i18n/buddyI18n'
import {
  Checkmark16Regular,
  ChevronDown16Regular,
  ChevronRight16Regular,
  Flash20Filled,
} from '@vicons/fluent'
import { NIcon } from 'naive-ui'
import { computed, onBeforeUnmount, onMounted, shallowRef, useTemplateRef } from 'vue'
import { useBuddyI18n } from '@/i18n/buddyI18n'

interface SelectorOption {
  description: string | null
  label: string
  value: string | null
}

const props = defineProps<{
  disabled: boolean
  language: BuddyLocale
  models: ReadonlyArray<LocalRuntimeModelOption>
  selectedEffort: string | null
  selectedModel: LocalRuntimeModelOption | null
  selectedModelId: string | null
  selectedServiceTier: string | null
}>()

const emit = defineEmits<{
  updateEffort: [value: string | null]
  updateModel: [value: string]
  updateServiceTier: [value: string | null]
}>()

const { t } = useBuddyI18n(() => props.language)
const root = useTemplateRef<HTMLElement>('root')
const isOpen = shallowRef(false)
const activePanel = shallowRef<'main' | 'model' | 'service'>('main')

const reasoningOptions = computed<ReadonlyArray<SelectorOption>>(() => {
  const model = props.selectedModel
  if (!model)
    return []

  const options = model.supportedReasoningEfforts.map(option => ({
    description: option.description,
    label: formatOptionLabel(option.reasoningEffort),
    value: option.reasoningEffort,
  }))

  return model.defaultReasoningEffort === null
    ? [{ description: null, label: t('desktop.chat.defaultEffort'), value: null }, ...options]
    : options
})

const serviceTierOptions = computed<ReadonlyArray<SelectorOption>>(() => {
  const model = props.selectedModel
  if (!model)
    return []

  const options = model.serviceTiers.map(option => ({
    description: option.description,
    label: option.name,
    value: option.id,
  }))

  return model.defaultServiceTier === null
    ? [{ description: null, label: t('desktop.chat.defaultServiceTier'), value: null }, ...options]
    : options
})

const selectedEffortLabel = computed(() => {
  if (!props.selectedModel)
    return ''

  return props.selectedEffort
    ? formatOptionLabel(props.selectedEffort)
    : t('desktop.chat.defaultEffort')
})

const selectedServiceTierLabel = computed(() => {
  if (!props.selectedModel)
    return t('desktop.chat.defaultServiceTier')

  return props.selectedModel.serviceTiers.find(option => option.id === props.selectedServiceTier)?.name
    ?? t('desktop.chat.defaultServiceTier')
})

const isFastMode = computed(() => {
  const serviceTier = props.selectedModel?.serviceTiers.find(
    option => option.id === props.selectedServiceTier,
  )
  return serviceTier
    ? /\b(?:fast|speed|quick|turbo)\b/i.test(`${serviceTier.id} ${serviceTier.name}`)
    : false
})

const triggerLabel = computed(() => [
  props.selectedModel?.displayName ?? t('desktop.chat.noModels'),
  selectedEffortLabel.value,
].filter(Boolean).join(' · '))

onMounted(() => {
  document.addEventListener('pointerdown', handleDocumentPointerDown)
  document.addEventListener('keydown', handleDocumentKeydown)
})

onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', handleDocumentPointerDown)
  document.removeEventListener('keydown', handleDocumentKeydown)
})

function toggle() {
  if (props.disabled || props.models.length === 0)
    return

  isOpen.value = !isOpen.value
  activePanel.value = 'main'
}

function close() {
  isOpen.value = false
  activePanel.value = 'main'
}

function selectEffort(value: string | null) {
  emit('updateEffort', value)
  close()
}

function selectModel(modelId: string) {
  emit('updateModel', modelId)
  activePanel.value = 'main'
}

function selectServiceTier(value: string | null) {
  emit('updateServiceTier', value)
  close()
}

function handleDocumentPointerDown(event: PointerEvent) {
  if (!isOpen.value || !(event.target instanceof Node) || root.value?.contains(event.target))
    return

  close()
}

function handleDocumentKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape')
    close()
}

function formatOptionLabel(value: string) {
  const normalized = value.trim()
  if (!normalized)
    return ''

  return `${normalized.slice(0, 1).toUpperCase()}${normalized.slice(1)}`
}
</script>

<template>
  <div ref="root" class="desktop-model-selector">
    <button
      class="desktop-model-selector__trigger"
      :class="{ 'is-fast': isFastMode }"
      type="button"
      aria-haspopup="menu"
      :aria-expanded="isOpen"
      :aria-label="`${t('desktop.chat.model')}: ${triggerLabel}`"
      :disabled="disabled || models.length === 0"
      @click="toggle"
    >
      <NIcon v-if="isFastMode" class="desktop-model-selector__flash" :component="Flash20Filled" />
      <span class="desktop-model-selector__model">{{ selectedModel?.displayName ?? t('desktop.chat.noModels') }}</span>
      <span v-if="selectedEffortLabel" class="desktop-model-selector__effort">
        {{ selectedEffortLabel }}
      </span>
      <NIcon class="desktop-model-selector__chevron" :component="ChevronDown16Regular" />
    </button>

    <div
      v-if="isOpen"
      class="desktop-model-selector__popover"
      role="menu"
      @pointerdown.stop
    >
      <section class="desktop-model-selector__menu desktop-model-selector__menu--main">
        <span class="desktop-model-selector__title">{{ t('desktop.chat.effort') }}</span>
        <button
          v-for="option in reasoningOptions"
          :key="option.value ?? 'default'"
          class="desktop-model-selector__item"
          type="button"
          role="menuitemradio"
          :aria-checked="selectedEffort === option.value"
          @click="selectEffort(option.value)"
        >
          <span class="desktop-model-selector__item-copy">
            <strong>{{ option.label }}</strong>
            <small v-if="option.description">{{ option.description }}</small>
          </span>
          <NIcon v-if="selectedEffort === option.value" :component="Checkmark16Regular" />
        </button>
        <span v-if="reasoningOptions.length === 0" class="desktop-model-selector__empty">
          {{ t('desktop.chat.noReasoningOptions') }}
        </span>

        <span class="desktop-model-selector__divider" />
        <button
          class="desktop-model-selector__item"
          :class="{ 'is-active': activePanel === 'model' }"
          type="button"
          @click="activePanel = 'model'"
        >
          <span class="desktop-model-selector__item-copy">
            <small>{{ t('desktop.chat.model') }}</small>
            <strong>{{ selectedModel?.displayName ?? t('desktop.chat.noModels') }}</strong>
          </span>
          <NIcon :component="ChevronRight16Regular" />
        </button>
        <button
          v-if="serviceTierOptions.length"
          class="desktop-model-selector__item"
          :class="{ 'is-active': activePanel === 'service' }"
          type="button"
          @click="activePanel = 'service'"
        >
          <span class="desktop-model-selector__item-copy">
            <small>{{ t('desktop.chat.serviceTier') }}</small>
            <strong>{{ selectedServiceTierLabel }}</strong>
          </span>
          <NIcon :component="ChevronRight16Regular" />
        </button>
      </section>

      <section
        v-if="activePanel === 'model'"
        class="desktop-model-selector__menu desktop-model-selector__menu--secondary"
      >
        <span class="desktop-model-selector__title">{{ t('desktop.chat.model') }}</span>
        <button
          v-for="model in models"
          :key="model.id"
          class="desktop-model-selector__item"
          type="button"
          role="menuitemradio"
          :aria-checked="selectedModelId === model.id"
          @click="selectModel(model.id)"
        >
          <span class="desktop-model-selector__item-copy">
            <strong>{{ model.displayName }}</strong>
            <small v-if="model.description">{{ model.description }}</small>
          </span>
          <NIcon v-if="selectedModelId === model.id" :component="Checkmark16Regular" />
        </button>
      </section>

      <section
        v-else-if="activePanel === 'service'"
        class="desktop-model-selector__menu desktop-model-selector__menu--secondary desktop-model-selector__menu--service"
      >
        <span class="desktop-model-selector__title">{{ t('desktop.chat.serviceTier') }}</span>
        <button
          v-for="option in serviceTierOptions"
          :key="option.value ?? 'default'"
          class="desktop-model-selector__item"
          type="button"
          role="menuitemradio"
          :aria-checked="selectedServiceTier === option.value"
          @click="selectServiceTier(option.value)"
        >
          <span class="desktop-model-selector__item-copy">
            <span class="desktop-model-selector__service-name">
              <NIcon
                v-if="/\b(?:fast|speed|quick|turbo)\b/i.test(`${option.value ?? ''} ${option.label}`)"
                class="desktop-model-selector__flash"
                :component="Flash20Filled"
              />
              <strong>{{ option.label }}</strong>
            </span>
            <small v-if="option.description">{{ option.description }}</small>
          </span>
          <NIcon v-if="selectedServiceTier === option.value" :component="Checkmark16Regular" />
        </button>
      </section>
    </div>
  </div>
</template>

<style scoped lang="scss">
.desktop-model-selector {
  position: relative;
  min-width: 0;
}

.desktop-model-selector__trigger {
  display: inline-flex;
  min-width: 0;
  max-width: min(22rem, 44vw);
  height: 2rem;
  align-items: center;
  gap: 0.35rem;
  border: 0;
  border-radius: 0.55rem;
  background: transparent;
  color: var(--buddy-text-secondary);
  cursor: pointer;
  font: inherit;
  font-size: 0.78rem;
  padding: 0 0.45rem 0 0.55rem;

  &:hover,
  &:focus-visible,
  &[aria-expanded='true'] {
    background: color-mix(in srgb, var(--buddy-accent-primary) 9%, transparent);
    color: var(--buddy-text-primary);
    outline: 0;
  }

  &:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  &.is-fast {
    color: color-mix(in srgb, var(--buddy-accent-primary) 82%, var(--buddy-text-primary));
  }
}

.desktop-model-selector__model {
  min-width: 0;
  overflow: hidden;
  font-weight: 650;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.desktop-model-selector__effort {
  flex: none;
  color: var(--buddy-text-placeholder);
}

.desktop-model-selector__chevron,
.desktop-model-selector__flash,
.desktop-model-selector__item > :deep(.n-icon) {
  flex: none;
}

.desktop-model-selector__flash {
  color: color-mix(in srgb, #c99a2e 84%, var(--buddy-accent-primary));
}

.desktop-model-selector__popover {
  position: absolute;
  right: 0;
  bottom: calc(100% + 0.65rem);
  z-index: 32;
  display: flex;
  max-width: calc(100vw - 3rem);
  flex-direction: row-reverse;
  align-items: flex-end;
  gap: 0.5rem;
}

.desktop-model-selector__menu {
  display: grid;
  width: 13.5rem;
  max-height: min(24rem, 58vh);
  gap: 0.15rem;
  overflow: hidden auto;
  border: 1px solid var(--buddy-border-light);
  border-radius: 0.8rem;
  background: color-mix(in srgb, var(--buddy-bg-surface-raised) 97%, transparent);
  box-shadow: 0 1rem 2.5rem rgb(23 33 28 / 16%);
  padding: 0.5rem;
}

.desktop-model-selector__menu--secondary {
  width: 15.5rem;
}

.desktop-model-selector__menu--service {
  width: 14rem;
}

.desktop-model-selector__title,
.desktop-model-selector__empty {
  color: var(--buddy-text-placeholder);
  font-size: 0.7rem;
  line-height: 1.35;
  padding: 0.15rem 0.55rem 0.3rem;
}

.desktop-model-selector__empty {
  padding-block: 0.45rem;
}

.desktop-model-selector__divider {
  height: 1px;
  margin: 0.25rem 0.15rem;
  background: var(--buddy-border-light);
}

.desktop-model-selector__item {
  display: flex;
  min-width: 0;
  min-height: 2.15rem;
  align-items: center;
  justify-content: space-between;
  gap: 0.65rem;
  border: 0;
  border-radius: 0.55rem;
  background: transparent;
  color: var(--buddy-text-primary);
  cursor: pointer;
  font: inherit;
  padding: 0.4rem 0.55rem;
  text-align: left;

  &:hover,
  &:focus-visible,
  &.is-active {
    background: var(--buddy-fill-base);
    outline: 0;
  }
}

.desktop-model-selector__item-copy {
  display: grid;
  min-width: 0;
  flex: 1;
  gap: 0.1rem;

  strong,
  small {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  strong {
    font-size: 0.78rem;
    font-weight: 650;
  }

  small {
    color: var(--buddy-text-placeholder);
    font-size: 0.68rem;
    line-height: 1.3;
  }
}

.desktop-model-selector__service-name {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 0.35rem;
}

@media (max-width: 680px) {
  .desktop-model-selector__trigger {
    max-width: 50vw;
  }

  .desktop-model-selector__popover {
    max-width: calc(100vw - 1.5rem);
    overflow-x: auto;
  }
}
</style>
