<script setup lang="ts">
import type { DesktopApprovalView } from './desktopChatState'
import type { BuddyLocale } from '@/i18n/buddyI18n'
import { computed } from 'vue'
import { useBuddyI18n } from '@/i18n/buddyI18n'

const props = defineProps<{
  approval: DesktopApprovalView
  language: BuddyLocale
  resolving: boolean
}>()
const emit = defineEmits<{
  approve: []
  deny: []
}>()

const { t } = useBuddyI18n(() => props.language)

const operationLabel = computed(() => {
  if (props.approval.operation === 'command')
    return t('desktop.approval.runCommand')
  if (props.approval.operation === 'file_change')
    return t('desktop.approval.fileChange')
  return t('desktop.approval.localOperation')
})

const scopeLabel = computed(() => {
  if (props.approval.scopeStatus === 'authorized')
    return t('desktop.approval.scopeAuthorized')
  if (props.approval.scopeStatus)
    return t('desktop.approval.scopeCheck', { status: props.approval.scopeStatus })
  return t('desktop.approval.scopeReview')
})

const preview = computed(() => props.approval.preview || operationLabel.value)
</script>

<template>
  <article class="desktop-approval-card">
    <div class="desktop-approval-card__header">
      <div>
        <strong>{{ t('desktop.approval.request', { operation: operationLabel }) }}</strong>
        <span>{{ scopeLabel }}</span>
      </div>
      <span v-if="approval.scopeStatus" class="desktop-approval-card__scope">
        {{ approval.scopeStatus }}
      </span>
    </div>

    <pre>{{ preview }}</pre>

    <dl v-if="approval.cwd || approval.targetRoot || approval.authorizationRoot">
      <template v-if="approval.cwd">
        <dt>{{ t('desktop.approval.workingDirectory') }}</dt>
        <dd :title="approval.cwd">
          {{ approval.cwd }}
        </dd>
      </template>
      <template v-if="approval.targetRoot">
        <dt>{{ t('desktop.approval.target') }}</dt>
        <dd :title="approval.targetRoot">
          {{ approval.targetRoot }}
        </dd>
      </template>
      <template v-if="approval.authorizationRoot">
        <dt>{{ t('desktop.approval.authorizationBoundary') }}</dt>
        <dd :title="approval.authorizationRoot">
          {{ approval.authorizationRoot }}
        </dd>
      </template>
    </dl>

    <p v-if="approval.scopeReason" class="desktop-approval-card__reason">
      {{ approval.scopeReason }}
    </p>

    <div class="desktop-approval-card__actions">
      <button type="button" :disabled="resolving" @click="emit('deny')">
        {{ t('approvalAction.deny') }}
      </button>
      <button
        class="is-primary"
        type="button"
        :disabled="resolving || approval.approvalMode === null"
        :title="approval.approvalMode === null ? t('desktop.approval.unsupported') : t('desktop.approval.allow')"
        @click="emit('approve')"
      >
        {{ resolving ? t('desktop.approval.processing') : t('desktop.approval.allow') }}
      </button>
    </div>
  </article>
</template>

<style scoped>
.desktop-approval-card {
  display: grid;
  gap: 0.65rem;
  border: 1px solid color-mix(in srgb, var(--buddy-accent-warning) 34%, var(--buddy-border-light));
  border-radius: 0.8rem;
  background: color-mix(in srgb, var(--buddy-accent-warning) 6%, var(--buddy-bg-surface-raised));
  color: var(--buddy-text-regular);
  padding: 0.75rem;
}

.desktop-approval-card__header,
.desktop-approval-card__actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
}

.desktop-approval-card__header > div {
  display: grid;
  min-width: 0;
  gap: 0.15rem;
}

.desktop-approval-card__header strong {
  color: var(--buddy-text-primary);
  font-size: 0.82rem;
}

.desktop-approval-card__header span,
.desktop-approval-card__reason {
  color: var(--buddy-text-secondary);
  font-size: 0.72rem;
}

.desktop-approval-card__scope {
  flex: none;
  border-radius: 999px;
  background: var(--buddy-fill-light);
  padding: 0.2rem 0.45rem;
}

.desktop-approval-card pre {
  max-height: 8rem;
  margin: 0;
  overflow: auto;
  border-radius: 0.55rem;
  background: var(--buddy-bg-surface);
  color: var(--buddy-text-primary);
  font-family: var(--buddy-font-mono);
  font-size: 0.72rem;
  line-height: 1.5;
  padding: 0.6rem;
  white-space: pre-wrap;
  word-break: break-word;
}

.desktop-approval-card dl {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  gap: 0.2rem 0.55rem;
  margin: 0;
  font-size: 0.68rem;
}

.desktop-approval-card dt {
  color: var(--buddy-text-placeholder);
}

.desktop-approval-card dd {
  min-width: 0;
  margin: 0;
  overflow: hidden;
  color: var(--buddy-text-secondary);
  font-family: var(--buddy-font-mono);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.desktop-approval-card__reason {
  margin: 0;
}

.desktop-approval-card__actions {
  justify-content: flex-end;
}

.desktop-approval-card__actions button {
  border: 1px solid var(--buddy-border-light);
  border-radius: 0.5rem;
  background: var(--buddy-bg-surface);
  color: var(--buddy-text-regular);
  cursor: pointer;
  font-size: 0.75rem;
  padding: 0.38rem 0.65rem;
}

.desktop-approval-card__actions button.is-primary {
  border-color: var(--buddy-accent-primary);
  background: var(--buddy-accent-primary);
  color: white;
}

.desktop-approval-card__actions button:disabled {
  cursor: default;
  opacity: 0.5;
}
</style>
