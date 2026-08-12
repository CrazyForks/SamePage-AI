<script setup lang="ts">
import type { LexoraConfigPatch } from '../../electron/shared/desktopApi'
import type { DesktopChatController } from './useDesktopChat'
import { NButton, NCard, NSelect, NSpin } from 'naive-ui'
import { computed, shallowRef } from 'vue'
import { useBuddyI18n } from '@/i18n/buddyI18n'

type AgentSettingField = 'model' | 'effort'

const props = defineProps<{
  chat: DesktopChatController
}>()

const chat = props.chat
const { t } = useBuddyI18n(chat.language)
const pendingFields = shallowRef<ReadonlySet<AgentSettingField>>(new Set())
const failedField = shallowRef<AgentSettingField | null>(null)
const isRestarting = computed(() =>
  chat.runtimeState.value.status === 'restarting'
  || chat.runtimeState.value.status === 'starting',
)
const modelOptions = computed(() => chat.models.value.map(model => ({
  label: model.displayName,
  value: model.model,
})))
const configuredModel = computed(() => {
  const modelName = chat.config.value?.agent.codex.defaultModel
  return chat.models.value.find(model => model.model === modelName)
    ?? chat.models.value.find(model => model.isDefault)
    ?? chat.models.value[0]
    ?? null
})
const effortOptions = computed(() => configuredModel.value?.supportedReasoningEfforts.map(option => ({
  label: option.reasoningEffort,
  value: option.reasoningEffort,
})) ?? [])

async function updateSetting(field: AgentSettingField, patch: LexoraConfigPatch) {
  pendingFields.value = new Set([...pendingFields.value, field])
  const succeeded = await chat.updateSettings(patch)
  pendingFields.value = new Set([...pendingFields.value].filter(item => item !== field))
  failedField.value = succeeded ? null : field
}
</script>

<template>
  <section class="desktop-codex-settings" aria-labelledby="codex-settings-title">
    <header>
      <h2 id="codex-settings-title">
        {{ t('desktop.agent.configurationTitle') }}
      </h2>
      <p>{{ t('desktop.agent.configurationDescription') }}</p>
    </header>

    <NCard v-if="chat.config.value" size="small">
      <div class="desktop-codex-settings__row">
        <div>
          <strong>{{ t('desktop.settings.defaultModel') }}</strong>
        </div>
        <div class="desktop-codex-settings__control">
          <NSelect
            :aria-label="t('desktop.settings.defaultModel')"
            :options="modelOptions"
            :value="chat.config.value.agent.codex.defaultModel || configuredModel?.model || null"
            @update:value="updateSetting('model', { agent: { codex: { defaultModel: $event } } })"
          />
          <NSpin v-if="pendingFields.has('model')" size="small" />
          <small v-else-if="failedField === 'model'" class="is-error">
            {{ chat.settingsError.value ?? t('desktop.settings.saveFailed') }}
          </small>
        </div>
      </div>
      <div class="desktop-codex-settings__row">
        <div>
          <strong>{{ t('desktop.settings.defaultReasoningEffort') }}</strong>
        </div>
        <div class="desktop-codex-settings__control">
          <NSelect
            :aria-label="t('desktop.settings.defaultReasoningEffort')"
            :options="effortOptions"
            :value="chat.config.value.agent.codex.reasoningEffort"
            @update:value="updateSetting('effort', { agent: { codex: { reasoningEffort: $event } } })"
          />
          <NSpin v-if="pendingFields.has('effort')" size="small" />
          <small v-else-if="failedField === 'effort'" class="is-error">
            {{ chat.settingsError.value ?? t('desktop.settings.saveFailed') }}
          </small>
        </div>
      </div>

      <footer>
        <span>{{ t('desktop.settings.restartDescription') }}</span>
        <NButton :loading="isRestarting" @click="chat.restartRuntime">
          {{ t('desktop.settings.restartRuntime') }}
        </NButton>
      </footer>
    </NCard>
  </section>
</template>

<style scoped lang="scss">
.desktop-codex-settings {
  display: grid;
  gap: 1rem;

  > header h2,
  > header p {
    margin: 0;
  }

  > header h2 {
    font-size: 1.05rem;
  }

  > header p {
    margin-top: 0.3rem;
    color: var(--buddy-text-secondary);
    font-size: 0.76rem;
  }

  :deep(.n-card) {
    border-color: var(--buddy-border-light);
    background: var(--buddy-bg-surface-raised);
  }
}

.desktop-codex-settings__row {
  display: grid;
  min-height: 4rem;
  grid-template-columns: minmax(9rem, 1fr) minmax(13rem, 19rem);
  align-items: center;
  gap: 2rem;
  border-bottom: 1px solid var(--buddy-border-light);

  strong {
    color: var(--buddy-text-regular);
    font-size: 0.8rem;
  }
}

.desktop-codex-settings__control {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: 0.55rem;

  .is-error {
    grid-column: 1 / -1;
    color: var(--buddy-accent-danger);
    font-size: 0.7rem;
    text-align: right;
  }
}

.desktop-codex-settings footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  padding-top: 1rem;

  span {
    color: var(--buddy-text-secondary);
    font-size: 0.72rem;
  }
}

@media (max-width: 760px) {
  .desktop-codex-settings__row {
    grid-template-columns: minmax(0, 1fr);
    gap: 0.7rem;
    padding: 0.9rem 0;
  }
}
</style>
