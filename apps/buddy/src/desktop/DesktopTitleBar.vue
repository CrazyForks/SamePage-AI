<script setup lang="ts">
import type {
  DesktopWindowState,
  LexoraDesktopApi,
} from '../../electron/shared/desktopApi'
import type { BuddyLocale } from '@/i18n/buddyI18n'
import { computed, onBeforeUnmount, onMounted, shallowRef } from 'vue'
import { useBuddyI18n } from '@/i18n/buddyI18n'

const props = defineProps<{
  language: BuddyLocale
}>()

const appIconUrl = new URL('../../runtime/icons/icon.png', import.meta.url).href
const desktopApi = requireDesktopApi()

const isAlwaysOnTop = shallowRef(false)
const isMaximized = shallowRef(false)
const { t } = useBuddyI18n(() => props.language)
const pinLabel = computed(() => isAlwaysOnTop.value ? t('window.unpin') : t('window.pin'))
const maximizeLabel = computed(() => isMaximized.value ? t('window.restore') : t('window.maximize'))
let windowStateVersion = 0

const stopWindowState = desktopApi.window.onStateChanged((state) => {
  windowStateVersion += 1
  applyWindowState(state)
})

onMounted(async () => {
  const snapshotVersion = windowStateVersion
  try {
    const state = await desktopApi.window.getState()
    if (snapshotVersion === windowStateVersion)
      applyWindowState(state)
  }
  catch (error) {
    console.error('Lexora window state is unavailable', error)
  }
})

onBeforeUnmount(stopWindowState)

async function toggleAlwaysOnTop() {
  await runWindowAction(() => desktopApi.window.toggleAlwaysOnTop())
}

async function toggleMaximize() {
  await runWindowAction(() => desktopApi.window.toggleMaximize())
}

async function minimize() {
  await runVoidWindowAction(() => desktopApi.window.minimize())
}

async function hide() {
  await runVoidWindowAction(() => desktopApi.window.hide())
}

async function runWindowAction(action: () => Promise<DesktopWindowState>) {
  try {
    applyWindowState(await action())
  }
  catch (error) {
    console.error('Lexora window action failed', error)
  }
}

async function runVoidWindowAction(action: () => Promise<void>) {
  try {
    await action()
  }
  catch (error) {
    console.error('Lexora window action failed', error)
  }
}

function applyWindowState(state: DesktopWindowState) {
  isAlwaysOnTop.value = state.isAlwaysOnTop
  isMaximized.value = state.isMaximized
}

function requireDesktopApi(): LexoraDesktopApi {
  const api = window.lexoraDesktop
  if (!api)
    throw new Error('Lexora Desktop API is unavailable')

  return api
}
</script>

<template>
  <header class="desktop-title-bar" @dblclick="toggleMaximize">
    <div class="desktop-title-bar__identity">
      <img :src="appIconUrl" alt="" draggable="false">
      <strong>Lexora</strong>
    </div>

    <div
      class="desktop-title-bar__controls"
      @dblclick.stop
      @mousedown.stop
      @pointerdown.stop
    >
      <button
        :aria-label="pinLabel"
        :aria-pressed="isAlwaysOnTop"
        class="desktop-title-bar__control"
        :class="{ 'is-active': isAlwaysOnTop }"
        type="button"
        :title="pinLabel"
        @click="toggleAlwaysOnTop"
      >
        <svg aria-hidden="true" viewBox="0 0 20 20">
          <path d="M7 3.5h6l-1 4 2.25 2.25v1H10.7V16l-.7 1-.7-1v-5.25H5.75v-1L8 7.5l-1-4Z" />
        </svg>
      </button>
      <button
        :aria-label="t('window.minimize')"
        class="desktop-title-bar__control"
        type="button"
        :title="t('window.minimize')"
        @click="minimize"
      >
        <svg aria-hidden="true" viewBox="0 0 20 20">
          <path d="M5 10h10v1H5z" />
        </svg>
      </button>
      <button
        :aria-label="maximizeLabel"
        class="desktop-title-bar__control"
        type="button"
        :title="maximizeLabel"
        @click="toggleMaximize"
      >
        <svg v-if="isMaximized" aria-hidden="true" viewBox="0 0 20 20">
          <path d="M7 5h8v8h-2v2H5V7h2V5Zm1 2v5h5V7H8Zm-2 1v6h6v-1H7V8H6Z" />
        </svg>
        <svg v-else aria-hidden="true" viewBox="0 0 20 20">
          <path d="M5.5 5.5h9v9h-9v-9Zm1 1v7h7v-7h-7Z" />
        </svg>
      </button>
      <button
        :aria-label="t('window.close')"
        class="desktop-title-bar__control is-close"
        type="button"
        :title="t('window.close')"
        @click="hide"
      >
        <svg aria-hidden="true" viewBox="0 0 20 20">
          <path d="m6.15 5.45 3.85 3.85 3.85-3.85.7.7L10.7 10l3.85 3.85-.7.7L10 10.7l-3.85 3.85-.7-.7L9.3 10 5.45 6.15l.7-.7Z" />
        </svg>
      </button>
    </div>
  </header>
</template>

<style scoped lang="scss">
.desktop-title-bar {
  display: flex;
  min-height: 2.5rem;
  flex: none;
  align-items: center;
  justify-content: space-between;
  border-bottom: 1px solid var(--buddy-border-light);
  background: var(--buddy-bg-surface-raised);
  color: var(--buddy-text-primary);
  user-select: none;
  -webkit-app-region: drag;
}

.desktop-title-bar__identity {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 0.5rem;
  padding-left: 0.75rem;
  pointer-events: none;
}

.desktop-title-bar__identity img {
  width: 1.25rem;
  height: 1.25rem;
  border-radius: 0.3rem;
}

.desktop-title-bar__identity strong {
  font-size: 0.78rem;
  font-weight: 600;
  letter-spacing: 0.01em;
}

.desktop-title-bar__controls {
  display: flex;
  align-self: stretch;
  -webkit-app-region: no-drag;
}

.desktop-title-bar__control {
  display: grid;
  width: 2.75rem;
  min-height: 2.5rem;
  place-items: center;
  border: 0;
  background: transparent;
  color: var(--buddy-text-secondary);
  cursor: default;
  -webkit-app-region: no-drag;

  &:hover {
    background: var(--buddy-fill-base);
    color: var(--buddy-text-primary);
  }

  &.is-active {
    color: var(--buddy-accent-primary);
  }

  &.is-close:hover {
    background: var(--buddy-accent-danger);
    color: var(--buddy-text-on-accent);
  }
}

.desktop-title-bar__control svg {
  width: 1rem;
  height: 1rem;
  fill: currentColor;
}
</style>
