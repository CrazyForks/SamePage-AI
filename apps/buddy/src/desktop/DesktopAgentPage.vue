<script setup lang="ts">
import type { DesktopAgentId } from './desktopViewState'
import type { DesktopChatController } from './useDesktopChat'
import DesktopAgentOverview from './DesktopAgentOverview.vue'
import DesktopCodexAgentPage from './DesktopCodexAgentPage.vue'

defineProps<{
  agentId: DesktopAgentId | null
  chat: DesktopChatController
}>()

const emit = defineEmits<{
  navigateAgent: [agentId: DesktopAgentId | null]
}>()
</script>

<template>
  <section class="desktop-agent-page">
    <DesktopAgentOverview
      v-if="agentId === null"
      :chat="chat"
      @open-codex="emit('navigateAgent', 'codex')"
    />
    <DesktopCodexAgentPage
      v-else
      :chat="chat"
      @back="emit('navigateAgent', null)"
    />
  </section>
</template>

<style scoped>
.desktop-agent-page {
  width: 100%;
  height: 100%;
  overflow: auto;
  background:
    radial-gradient(circle at 82% 0%, color-mix(in srgb, var(--buddy-accent-primary) 7%, transparent), transparent 25rem),
    var(--buddy-bg-body);
  padding: clamp(1.5rem, 3vw, 3rem);
}
</style>
