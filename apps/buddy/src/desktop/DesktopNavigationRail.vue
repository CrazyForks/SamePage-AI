<script setup lang="ts">
import type { Component } from 'vue'
import type { DesktopView } from './desktopViewState'
import {
  Bot24Filled,
  Bot24Regular,
  Chat24Filled,
  Chat24Regular,
  Settings24Filled,
  Settings24Regular,
} from '@vicons/fluent'
import { NButton, NIcon, NTooltip } from 'naive-ui'
import { computed } from 'vue'
import { useBuddyI18n } from '@/i18n/buddyI18n'

const props = defineProps<{
  activeView: DesktopView
  language: 'zh-CN' | 'en-US'
}>()

const emit = defineEmits<{
  navigate: [view: DesktopView]
}>()

const { t } = useBuddyI18n(() => props.language)
interface NavigationItem {
  activeIcon: Component
  icon: Component
  label: string
  view: DesktopView
}

const mainItems = computed<ReadonlyArray<NavigationItem>>(() => [
  {
    activeIcon: Chat24Filled,
    icon: Chat24Regular,
    label: t('desktop.navigation.chat'),
    view: 'chat',
  },
  {
    activeIcon: Bot24Filled,
    icon: Bot24Regular,
    label: t('desktop.navigation.agent'),
    view: 'agent',
  },
])
const settingsItem = computed<NavigationItem>(() => ({
  activeIcon: Settings24Filled,
  icon: Settings24Regular,
  label: t('desktop.navigation.settings'),
  view: 'settings',
}))
</script>

<template>
  <nav class="desktop-navigation-rail" :aria-label="t('desktop.navigation.primary')">
    <div class="desktop-navigation-rail__mark" aria-hidden="true">
      L
    </div>

    <div class="desktop-navigation-rail__items">
      <NTooltip
        v-for="item in mainItems"
        :key="item.view"
        placement="right"
      >
        <template #trigger>
          <NButton
            :aria-current="activeView === item.view ? 'page' : undefined"
            :aria-label="item.label"
            class="desktop-navigation-rail__button"
            :class="{ 'is-active': activeView === item.view }"
            circle
            quaternary
            @click="emit('navigate', item.view)"
          >
            <template #icon>
              <NIcon :component="activeView === item.view ? item.activeIcon : item.icon" />
            </template>
          </NButton>
        </template>
        {{ item.label }}
      </NTooltip>
    </div>

    <div class="desktop-navigation-rail__footer">
      <NTooltip placement="right">
        <template #trigger>
          <NButton
            :aria-current="activeView === settingsItem.view ? 'page' : undefined"
            :aria-label="settingsItem.label"
            class="desktop-navigation-rail__button"
            :class="{ 'is-active': activeView === settingsItem.view }"
            circle
            quaternary
            @click="emit('navigate', settingsItem.view)"
          >
            <template #icon>
              <NIcon :component="activeView === settingsItem.view ? settingsItem.activeIcon : settingsItem.icon" />
            </template>
          </NButton>
        </template>
        {{ settingsItem.label }}
      </NTooltip>
    </div>
  </nav>
</template>

<style scoped lang="scss">
.desktop-navigation-rail {
  display: flex;
  width: 4rem;
  flex: none;
  flex-direction: column;
  align-items: center;
  gap: 1.35rem;
  border-right: 1px solid var(--buddy-border-light);
  background:
    linear-gradient(180deg, color-mix(in srgb, var(--buddy-accent-primary) 8%, transparent), transparent 24%),
    var(--buddy-bg-surface-raised);
  padding: 0.9rem 0;
}

.desktop-navigation-rail__mark {
  display: grid;
  width: 2.25rem;
  height: 2.25rem;
  place-items: center;
  border-radius: 0.75rem;
  background: var(--buddy-accent-primary);
  color: var(--buddy-text-on-accent);
  font-size: 0.9rem;
  font-weight: 760;
  box-shadow: 0 0.55rem 1.4rem color-mix(in srgb, var(--buddy-accent-primary) 26%, transparent);
}

.desktop-navigation-rail__items {
  display: grid;
  gap: 0.5rem;
}

.desktop-navigation-rail__footer {
  display: grid;
  margin-top: auto;
}

.desktop-navigation-rail__button {
  color: var(--buddy-text-secondary);

  &.is-active {
    background: color-mix(in srgb, var(--buddy-accent-primary) 14%, transparent);
    color: var(--buddy-accent-primary);
  }
}
</style>
