<script setup lang="ts">
import type { DesktopAgentId, DesktopRoute, DesktopSettingsTab, DesktopView } from './desktopViewState'
import { onBeforeUnmount, onMounted, shallowRef, watch } from 'vue'
import DesktopAgentPage from './DesktopAgentPage.vue'
import DesktopChatPage from './DesktopChatPage.vue'
import DesktopNavigationRail from './DesktopNavigationRail.vue'
import DesktopSettingsPage from './DesktopSettingsPage.vue'
import DesktopTitleBar from './DesktopTitleBar.vue'
import { resolveDesktopRoute, toDesktopRouteHash } from './desktopViewState'
import { useDesktopChat } from './useDesktopChat'

const emit = defineEmits<{
  themeChange: [theme: 'system' | 'light' | 'dark']
}>()
const chat = useDesktopChat()
const activeRoute = shallowRef<DesktopRoute>(resolveDesktopRoute(window.location))
let initializePromise: Promise<void> | null = null

watch(
  () => chat.config.value?.desktop.theme,
  (theme) => {
    if (theme)
      emit('themeChange', theme)
  },
  { immediate: true },
)

function navigate(view: DesktopView) {
  const currentRoute = activeRoute.value
  if (currentRoute.view === view) {
    if (currentRoute.view === 'agent' && currentRoute.agentId !== null)
      navigateAgent(null)
    return
  }

  navigateRoute(view === 'settings'
    ? { settingsTab: 'general', view }
    : view === 'agent'
      ? { agentId: null, view }
      : { view })
}

function navigateAgent(agentId: DesktopAgentId | null) {
  navigateRoute({ agentId, view: 'agent' })
}

function navigateSettingsTab(settingsTab: DesktopSettingsTab) {
  navigateRoute({ settingsTab, view: 'settings' })
}

function navigateRoute(route: DesktopRoute) {
  activeRoute.value = route
  const hash = toDesktopRouteHash(route)
  if (window.location.hash !== hash) {
    window.location.hash = hash
    return
  }
  activateRoute(route)
}

function syncViewFromLocation() {
  activeRoute.value = resolveDesktopRoute(window.location)
  activateRoute(activeRoute.value)
}

function activateRoute(route: DesktopRoute) {
  void (initializePromise ?? Promise.resolve()).then(async () => {
    if (route.view === 'agent') {
      await Promise.all([chat.loadAgent(), chat.loadUsage()])
      return
    }
    if (route.view === 'settings' && route.settingsTab === 'directories')
      await chat.loadLocalState()
  })
}

onMounted(() => {
  window.addEventListener('hashchange', syncViewFromLocation)
  initializePromise = chat.initialize()
  void initializePromise.then(() => activateRoute(activeRoute.value))
})

onBeforeUnmount(() => {
  window.removeEventListener('hashchange', syncViewFromLocation)
})
</script>

<template>
  <div class="desktop-shell">
    <DesktopTitleBar :language="chat.language.value" />
    <div class="desktop-shell__body">
      <DesktopNavigationRail
        :active-view="activeRoute.view"
        :language="chat.language.value"
        @navigate="navigate"
      />
      <main class="desktop-shell__workspace">
        <DesktopChatPage
          v-show="activeRoute.view === 'chat'"
          :chat="chat"
          @open-agent="navigateAgent('codex')"
        />
        <DesktopAgentPage
          v-if="activeRoute.view === 'agent'"
          :agent-id="activeRoute.agentId"
          :chat="chat"
          @navigate-agent="navigateAgent"
        />
        <DesktopSettingsPage
          v-if="activeRoute.view === 'settings'"
          :active-tab="activeRoute.settingsTab"
          :chat="chat"
          @navigate-tab="navigateSettingsTab"
        />
      </main>
    </div>
  </div>
</template>

<style scoped>
.desktop-shell {
  display: flex;
  width: 100dvw;
  height: 100dvh;
  min-width: 0;
  min-height: 0;
  flex-direction: column;
  border: 1px solid var(--buddy-window-border);
  background: var(--buddy-bg-body);
  box-shadow: var(--buddy-shadow-window);
}

.desktop-shell__body {
  display: flex;
  min-width: 0;
  min-height: 0;
  flex: 1;
}

.desktop-shell__workspace {
  min-width: 0;
  min-height: 0;
  flex: 1;
}

.desktop-shell__workspace > * {
  width: 100%;
  height: 100%;
}
</style>
