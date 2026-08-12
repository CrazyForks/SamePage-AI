<script setup lang="ts">
import type { DesktopComposerSubmitPayload } from './desktopComposerInput'
import type { DesktopChatController } from './useDesktopChat'
import { NButton } from 'naive-ui'
import { computed, onBeforeUnmount, onMounted, shallowRef } from 'vue'
import BuddyChatMessageList from '@/chat/BuddyChatMessageList.vue'
import DesktopApprovalCard from '@/desktop/DesktopApprovalCard.vue'
import DesktopChatComposer from '@/desktop/DesktopChatComposer.vue'
import DesktopChatSidebar from '@/desktop/DesktopChatSidebar.vue'
import { useModalFocusTrap } from '@/desktop/useModalFocusTrap'
import { useBuddyI18n } from '@/i18n/buddyI18n'

const props = defineProps<{
  chat: DesktopChatController
}>()
const emit = defineEmits<{
  openAgent: []
}>()

const chat = props.chat
const { t } = useBuddyI18n(chat.language)
const pendingConversationDeletion = shallowRef<{ id: string, label: string } | null>(null)
const deletionDialog = shallowRef<HTMLElement | null>(null)
const isEmpty = computed(() => chat.activeConversationId.value === null)
useModalFocusTrap(() => pendingConversationDeletion.value !== null, deletionDialog)

const runtimeLabel = computed(() => {
  if (chat.runtimeState.value.status === 'ready') {
    return chat.codexStatus.value?.loginStatus === 'logged_in'
      ? t('desktop.chat.codexConnected')
      : t('desktop.chat.codexLoginRequired')
  }

  if (chat.runtimeState.value.status === 'offline')
    return t('desktop.chat.runtimeOffline')

  return t('desktop.chat.runtimeStarting')
})

onMounted(() => {
  document.addEventListener('keydown', handleDocumentKeydown)
})

onBeforeUnmount(() => {
  document.removeEventListener('keydown', handleDocumentKeydown)
})

async function sendMessage(payload: DesktopComposerSubmitPayload) {
  await chat.send(payload)
}

function requestConversationDeletion(conversationId: string) {
  const conversation = chat.conversations.value.find(item => item.id === conversationId)
  pendingConversationDeletion.value = {
    id: conversationId,
    label: conversation?.title?.trim() || t('desktop.chat.untitled'),
  }
}

async function confirmConversationDeletion() {
  const pending = pendingConversationDeletion.value
  if (!pending)
    return

  pendingConversationDeletion.value = null
  await chat.deleteConversation(pending.id)
}

function handleDocumentKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape')
    pendingConversationDeletion.value = null
}
</script>

<template>
  <section class="desktop-chat-page">
    <header class="desktop-chat-page__header">
      <div class="desktop-chat-page__title">
        <strong>{{ chat.currentTitle.value }}</strong>
        <span v-if="chat.currentCwd.value" :title="chat.currentCwd.value">
          {{ chat.currentCwd.value }}
        </span>
      </div>
      <button
        class="desktop-chat-page__runtime"
        :class="`is-${chat.runtimeState.value.status}`"
        type="button"
        :title="runtimeLabel"
        @click="emit('openAgent')"
      >
        <i />
        {{ runtimeLabel }}
      </button>
    </header>

    <div v-if="chat.errorMessage.value" class="desktop-chat-page__error" role="alert">
      <span>{{ chat.errorMessage.value }}</span>
      <button
        v-if="chat.runtimeState.value.status === 'offline'"
        type="button"
        @click="chat.restartChatRuntime"
      >
        {{ t('desktop.chat.runtimeRestart') }}
      </button>
    </div>

    <div
      class="desktop-chat-page__workspace"
      :class="{ 'is-empty': isEmpty, 'is-sidebar-collapsed': chat.sidebarCollapsed.value }"
    >
      <aside v-if="!chat.sidebarCollapsed.value" class="desktop-chat-page__sidebar">
        <DesktopChatSidebar
          :active-conversation-id="chat.activeConversationId.value"
          :conversations="chat.conversations.value"
          :project-root="chat.projectRoot.value"
          :projects="chat.projects.value"
          :language="chat.language.value"
          @add-project="chat.authorizeProject"
          @collapse="chat.setSidebarCollapsed(true)"
          @delete-conversation="requestConversationDeletion"
          @new-global="chat.startGlobalConversation"
          @new-project="chat.startProjectConversation"
          @open-conversation="chat.openConversation"
        />
      </aside>

      <main class="desktop-chat-page__main">
        <NButton
          v-if="chat.sidebarCollapsed.value"
          class="desktop-chat-page__show-sidebar"
          circle
          quaternary
          :title="t('desktop.sidebar.expand')"
          :aria-label="t('desktop.sidebar.expand')"
          @click="chat.setSidebarCollapsed(false)"
        >
          ☰
        </NButton>

        <section v-if="isEmpty" class="desktop-chat-page__hero">
          <span class="desktop-chat-page__hero-mark">L</span>
          <h1>{{ chat.currentScope.value === 'project' ? t('desktop.chat.projectHero') : t('desktop.chat.globalHero') }}</h1>
          <p v-if="chat.currentCwd.value">
            {{ t('desktop.chat.projectDescription') }}
          </p>
          <p v-else>
            {{ t('desktop.chat.globalDescription') }}
          </p>
          <DesktopChatComposer
            :attachments="chat.attachments.value"
            :can-send="chat.canSend.value"
            :composer-content="chat.composerContent.value"
            :draft="chat.draft.value"
            :is-running="Boolean(chat.activeRun.value)"
            :is-selecting-files="chat.isSelectingFiles.value"
            :is-sending="chat.isSending.value"
            :language="chat.language.value"
            :load-context-options="chat.listContextOptions"
            :models="chat.models.value"
            :selected-effort="chat.selectedEffort.value"
            :selected-model="chat.selectedModel.value"
            :selected-model-id="chat.selectedModelId.value"
            :selected-service-tier="chat.selectedServiceTier.value"
            @attach="chat.selectAttachments"
            @remove-attachment="chat.removeAttachment"
            @send="sendMessage"
            @stop="chat.cancelActiveRun"
            @update-content="chat.updateComposerContent"
            @update-effort="chat.selectedEffort.value = $event"
            @update-model="chat.selectModel"
            @update-service-tier="chat.selectedServiceTier.value = $event"
          />
        </section>

        <template v-else>
          <div class="desktop-chat-page__messages">
            <div v-if="chat.isLoading.value" class="desktop-chat-page__loading">
              {{ t('desktop.chat.loading') }}
            </div>
            <BuddyChatMessageList
              v-else
              :language="chat.language.value"
              :messages="chat.messages.value"
              :run-events="chat.runEvents.value"
            />
          </div>
          <footer class="desktop-chat-page__composer-dock">
            <div v-if="chat.approvalViews.value.length" class="desktop-chat-page__approvals">
              <DesktopApprovalCard
                v-for="approval in chat.approvalViews.value"
                :key="approval.approval.id"
                :approval="approval"
                :language="chat.language.value"
                :resolving="chat.resolvingApprovalIds.value.has(approval.approval.id)"
                @approve="chat.resolveApproval(approval.approval.id, 'approve')"
                @deny="chat.resolveApproval(approval.approval.id, 'deny')"
              />
            </div>
            <DesktopChatComposer
              :attachments="chat.attachments.value"
              :can-send="chat.canSend.value"
              :composer-content="chat.composerContent.value"
              :draft="chat.draft.value"
              :is-running="Boolean(chat.activeRun.value)"
              :is-selecting-files="chat.isSelectingFiles.value"
              :is-sending="chat.isSending.value"
              :language="chat.language.value"
              :load-context-options="chat.listContextOptions"
              :models="chat.models.value"
              :selected-effort="chat.selectedEffort.value"
              :selected-model="chat.selectedModel.value"
              :selected-model-id="chat.selectedModelId.value"
              :selected-service-tier="chat.selectedServiceTier.value"
              @attach="chat.selectAttachments"
              @remove-attachment="chat.removeAttachment"
              @send="sendMessage"
              @stop="chat.cancelActiveRun"
              @update-content="chat.updateComposerContent"
              @update-effort="chat.selectedEffort.value = $event"
              @update-model="chat.selectModel"
              @update-service-tier="chat.selectedServiceTier.value = $event"
            />
          </footer>
        </template>
      </main>
    </div>

    <div
      v-if="pendingConversationDeletion"
      class="desktop-chat-page__dialog-backdrop"
      role="presentation"
      @click.self="pendingConversationDeletion = null"
    >
      <section
        ref="deletionDialog"
        aria-labelledby="delete-conversation-title"
        aria-describedby="delete-conversation-description"
        aria-modal="true"
        class="desktop-chat-page__dialog"
        role="alertdialog"
        tabindex="-1"
      >
        <h2 id="delete-conversation-title">
          {{ t('chat.deleteConversationConfirmTitle') }}
        </h2>
        <p id="delete-conversation-description">
          {{ t('chat.deleteConversationConfirmMessage', { title: pendingConversationDeletion.label }) }}
        </p>
        <div>
          <NButton @click="pendingConversationDeletion = null">
            {{ t('common.cancel') }}
          </NButton>
          <NButton type="error" autofocus @click="confirmConversationDeletion">
            {{ t('chat.deleteConversation') }}
          </NButton>
        </div>
      </section>
    </div>
  </section>
</template>

<style scoped lang="scss">
.desktop-chat-page {
  display: flex;
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  flex-direction: column;
  background: var(--buddy-bg-surface);
  overflow: hidden;
}

.desktop-chat-page__header {
  display: flex;
  min-height: 3.75rem;
  flex: none;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  border-bottom: 1px solid var(--buddy-border-light);
  padding: 0.65rem 1rem;
}

.desktop-chat-page__title {
  display: grid;
  min-width: 0;
  gap: 0.15rem;

  strong {
    color: var(--buddy-text-primary);
    font-size: 0.92rem;
  }

  span {
    max-width: min(42rem, 60vw);
    overflow: hidden;
    color: var(--buddy-text-placeholder);
    font-family: var(--buddy-font-mono);
    font-size: 0.66rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
}

.desktop-chat-page__runtime {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  border: 0;
  background: transparent;
  color: var(--buddy-text-secondary);
  font-size: 0.72rem;

  i {
    width: 0.48rem;
    height: 0.48rem;
    border-radius: 50%;
    background: var(--buddy-accent-warning);
  }

  &.is-ready i {
    background: var(--buddy-accent-success);
  }

  &.is-offline {
    cursor: pointer;

    i {
      background: var(--buddy-accent-danger);
    }
  }
}

.desktop-chat-page__error {
  display: flex;
  flex: none;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  border-bottom: 1px solid color-mix(in srgb, var(--buddy-accent-danger) 28%, transparent);
  background: color-mix(in srgb, var(--buddy-accent-danger) 7%, var(--buddy-bg-surface));
  color: var(--buddy-accent-danger);
  font-size: 0.78rem;
  padding: 0.5rem 1rem;

  button {
    border: 0;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font-weight: 650;
  }
}

.desktop-chat-page__workspace {
  display: flex;
  min-width: 0;
  min-height: 0;
  flex: 1;
  overflow: hidden;
}

.desktop-chat-page__sidebar {
  width: 17rem;
  height: 100%;
  min-width: 13rem;
  min-height: 0;
  flex: none;
  border-right: 1px solid var(--buddy-border-light);
  background: var(--buddy-fill-light);
  overflow: hidden;
}

.desktop-chat-page__main {
  position: relative;
  display: flex;
  min-width: 0;
  min-height: 0;
  flex: 1;
  flex-direction: column;
}

.desktop-chat-page__show-sidebar {
  position: absolute;
  z-index: 4;
  top: 0.75rem;
  left: 0.75rem;
  color: var(--buddy-text-secondary);
}

.desktop-chat-page__hero {
  display: grid;
  width: 100%;
  min-height: 0;
  flex: 1;
  align-content: center;
  justify-items: center;
  padding: 2rem 1rem 4rem;

  h1 {
    margin: 0;
    color: var(--buddy-text-primary);
    font-size: clamp(1.45rem, 3vw, 2rem);
    letter-spacing: -0.035em;
  }

  > p {
    margin: 0.65rem 0 1.25rem;
    color: var(--buddy-text-secondary);
    font-size: 0.9rem;
  }

  :deep(.desktop-chat-composer-wrap) {
    width: min(52rem, calc(100% - 2rem));
  }
}

.desktop-chat-page__hero-mark {
  display: grid;
  width: 2.65rem;
  height: 2.65rem;
  place-items: center;
  margin-bottom: 1rem;
  border-radius: 0.85rem;
  background: var(--buddy-accent-primary);
  color: var(--buddy-text-on-accent);
  font-size: 1.1rem;
  font-weight: 750;
  box-shadow: 0 0.65rem 1.6rem color-mix(in srgb, var(--buddy-accent-primary) 24%, transparent);
}

.desktop-chat-page__messages {
  min-height: 0;
  flex: 1;
  overflow: hidden;
  padding-top: 1rem;
}

.desktop-chat-page__loading {
  display: grid;
  height: 100%;
  place-items: center;
  color: var(--buddy-text-placeholder);
  font-size: 0.82rem;
}

.desktop-chat-page__composer-dock {
  flex: none;
  background: linear-gradient(180deg, transparent, var(--buddy-bg-surface) 1.5rem);
  padding: 1rem 1.5rem 1.25rem;

  > :deep(.desktop-chat-composer-wrap) {
    width: min(52rem, 100%);
    margin: 0 auto;
  }
}

.desktop-chat-page__approvals {
  display: grid;
  width: min(52rem, 100%);
  gap: 0.35rem;
  margin: 0 auto 0.5rem;
}

.desktop-chat-page__dialog-backdrop {
  position: fixed;
  z-index: 100;
  display: grid;
  background: rgb(20 27 24 / 38%);
  inset: 0;
  place-items: center;
}

.desktop-chat-page__dialog {
  width: min(26rem, calc(100vw - 2rem));
  border: 1px solid var(--buddy-border-light);
  border-radius: 0.9rem;
  background: var(--buddy-bg-surface-raised);
  box-shadow: 0 1.2rem 4rem rgb(20 27 24 / 20%);
  padding: 1.1rem;

  h2 {
    margin: 0;
    color: var(--buddy-text-primary);
    font-size: 1rem;
  }

  p {
    margin: 0.65rem 0 1rem;
    color: var(--buddy-text-secondary);
    font-size: 0.82rem;
    line-height: 1.6;
  }

  > div {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }
}
</style>
