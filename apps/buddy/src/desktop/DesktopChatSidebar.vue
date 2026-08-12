<script setup lang="ts">
import type { LocalConversation, LocalProject } from '../../electron/shared/localChatApi'
import type { BuddyLocale } from '@/i18n/buddyI18n'
import { computed } from 'vue'
import { useBuddyI18n } from '@/i18n/buddyI18n'

const props = defineProps<{
  activeConversationId: string | null
  conversations: ReadonlyArray<LocalConversation>
  language: BuddyLocale
  projectRoot: string | null
  projects: ReadonlyArray<LocalProject>
}>()
const emit = defineEmits<{
  addProject: []
  collapse: []
  deleteConversation: [conversationId: string]
  newGlobal: []
  newProject: [root: string]
  openConversation: [conversationId: string]
}>()

const { t } = useBuddyI18n(() => props.language)

const globalConversations = computed(() =>
  props.conversations.filter(conversation => conversation.scope === 'global'),
)

function projectConversations(root: string) {
  return props.conversations.filter(conversation => conversation.projectRoot === root)
}

function formatConversationTitle(conversation: LocalConversation) {
  return conversation.title?.trim() || t('desktop.chat.untitled')
}

function deleteConversationLabel(conversation: LocalConversation) {
  return `${t('chat.deleteConversation')}: ${formatConversationTitle(conversation)}`
}
</script>

<template>
  <nav class="desktop-chat-sidebar" :aria-label="t('desktop.chat.localConversations')">
    <header class="desktop-chat-sidebar__header">
      <span class="desktop-chat-sidebar__brand">Lexora</span>
      <button type="button" :title="t('desktop.sidebar.collapse')" @click="emit('collapse')">
        ‹
      </button>
    </header>

    <button class="desktop-chat-sidebar__new" type="button" @click="emit('newGlobal')">
      <span>＋</span>
      {{ t('chat.newConversation') }}
    </button>

    <section class="desktop-chat-sidebar__section">
      <h2>{{ t('desktop.chat.personalConversations') }}</h2>
      <ul>
        <li v-for="conversation in globalConversations" :key="conversation.id">
          <button
            class="desktop-chat-sidebar__conversation"
            :class="{ 'is-active': conversation.id === activeConversationId }"
            type="button"
            @click="emit('openConversation', conversation.id)"
          >
            <span>{{ formatConversationTitle(conversation) }}</span>
          </button>
          <button
            class="desktop-chat-sidebar__delete"
            type="button"
            :aria-label="deleteConversationLabel(conversation)"
            :title="t('chat.deleteConversationConfirmTitle')"
            @click="emit('deleteConversation', conversation.id)"
          >
            ×
          </button>
        </li>
      </ul>
    </section>

    <section class="desktop-chat-sidebar__section desktop-chat-sidebar__section--projects">
      <div class="desktop-chat-sidebar__section-heading">
        <h2>{{ t('desktop.sidebar.localProjects') }}</h2>
        <button type="button" :title="t('desktop.sidebar.authorizeProject')" @click="emit('addProject')">
          ＋
        </button>
      </div>
      <div v-for="project in projects" :key="project.root" class="desktop-chat-sidebar__project">
        <button
          class="desktop-chat-sidebar__project-name"
          :class="{ 'is-active': project.root === projectRoot && !activeConversationId }"
          type="button"
          :title="project.root"
          @click="emit('newProject', project.root)"
        >
          <span class="desktop-chat-sidebar__project-mark">⌁</span>
          <span>{{ project.name }}</span>
        </button>
        <ul>
          <li v-for="conversation in projectConversations(project.root)" :key="conversation.id">
            <button
              class="desktop-chat-sidebar__conversation"
              :class="{ 'is-active': conversation.id === activeConversationId }"
              type="button"
              @click="emit('openConversation', conversation.id)"
            >
              <span>{{ formatConversationTitle(conversation) }}</span>
            </button>
            <button
              class="desktop-chat-sidebar__delete"
              type="button"
              :aria-label="deleteConversationLabel(conversation)"
              :title="t('chat.deleteConversationConfirmTitle')"
              @click="emit('deleteConversation', conversation.id)"
            >
              ×
            </button>
          </li>
        </ul>
      </div>
    </section>
  </nav>
</template>

<style scoped>
.desktop-chat-sidebar {
  display: flex;
  width: 17rem;
  height: 100%;
  min-height: 0;
  box-sizing: border-box;
  flex-direction: column;
  gap: 1rem;
  border-right: 1px solid var(--buddy-border-light);
  background: var(--buddy-fill-light);
  padding: 1rem 0.75rem;
  overflow-y: auto;
}

.desktop-chat-sidebar__header,
.desktop-chat-sidebar__section-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.desktop-chat-sidebar__brand {
  color: var(--buddy-text-primary);
  font-size: 1.05rem;
  font-weight: 700;
  letter-spacing: -0.02em;
}

.desktop-chat-sidebar button {
  border: 0;
  background: transparent;
  color: inherit;
  cursor: pointer;
}

.desktop-chat-sidebar__header button,
.desktop-chat-sidebar__section-heading button {
  display: grid;
  width: 1.75rem;
  height: 1.75rem;
  place-items: center;
  border-radius: 0.45rem;
  color: var(--buddy-text-secondary);
}

.desktop-chat-sidebar__header button:hover,
.desktop-chat-sidebar__section-heading button:hover {
  background: var(--buddy-fill-base);
}

.desktop-chat-sidebar__new {
  display: flex;
  align-items: center;
  gap: 0.55rem;
  width: 100%;
  border: 1px solid var(--buddy-border-base) !important;
  border-radius: 0.65rem;
  background: var(--buddy-bg-surface) !important;
  color: var(--buddy-text-regular) !important;
  font-weight: 600;
  padding: 0.65rem 0.75rem;
  box-shadow: 0 0.35rem 1rem rgb(23 33 28 / 4%);
}

.desktop-chat-sidebar__section {
  display: grid;
  gap: 0.4rem;
}

.desktop-chat-sidebar__section--projects {
  min-height: 0;
}

.desktop-chat-sidebar__section h2 {
  margin: 0;
  color: var(--buddy-text-secondary);
  font-size: 0.72rem;
  font-weight: 650;
  letter-spacing: 0.05em;
  text-transform: uppercase;
}

.desktop-chat-sidebar ul {
  display: grid;
  gap: 0.15rem;
  margin: 0;
  padding: 0;
  list-style: none;
}

.desktop-chat-sidebar li {
  position: relative;
  display: flex;
  min-width: 0;
}

.desktop-chat-sidebar__conversation,
.desktop-chat-sidebar__project-name {
  min-width: 0;
  width: 100%;
  border-radius: 0.5rem;
  color: var(--buddy-text-regular) !important;
  overflow: hidden;
  padding: 0.5rem 1.8rem 0.5rem 0.65rem;
  text-align: left;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.desktop-chat-sidebar__conversation:hover,
.desktop-chat-sidebar__conversation.is-active,
.desktop-chat-sidebar__project-name:hover,
.desktop-chat-sidebar__project-name.is-active {
  background: color-mix(in srgb, var(--buddy-accent-primary) 9%, transparent);
  color: var(--buddy-accent-primary) !important;
}

.desktop-chat-sidebar__delete {
  position: absolute;
  top: 50%;
  right: 0.35rem;
  display: grid;
  width: 1.4rem;
  height: 1.4rem;
  transform: translateY(-50%);
  place-items: center;
  border-radius: 0.35rem;
  color: var(--buddy-text-placeholder) !important;
  opacity: 0;
  pointer-events: none;
}

.desktop-chat-sidebar li:hover .desktop-chat-sidebar__delete,
.desktop-chat-sidebar li:focus-within .desktop-chat-sidebar__delete {
  opacity: 1;
  pointer-events: auto;
}

.desktop-chat-sidebar__delete:focus-visible {
  outline: 2px solid var(--buddy-accent-primary);
  outline-offset: 1px;
}

.desktop-chat-sidebar__project {
  display: grid;
  gap: 0.15rem;
}

.desktop-chat-sidebar__project-name {
  display: flex;
  align-items: center;
  gap: 0.45rem;
  padding-right: 0.65rem;
  font-weight: 600;
}

.desktop-chat-sidebar__project-mark {
  color: var(--buddy-accent-primary);
}

.desktop-chat-sidebar__project ul {
  margin-left: 1rem;
}
</style>
