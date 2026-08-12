<script setup lang="ts">
import type { GlobalThemeOverrides } from 'naive-ui'
import { darkTheme, NConfigProvider } from 'naive-ui'
import { computed, onBeforeUnmount, onMounted, shallowRef } from 'vue'
import DesktopShell from '@/desktop/DesktopShell.vue'

type DesktopThemePreference = 'system' | 'light' | 'dark'

const systemPrefersDark = shallowRef(false)
const themePreference = shallowRef<DesktopThemePreference>('system')
const prefersDark = computed(() =>
  themePreference.value === 'dark'
  || (themePreference.value === 'system' && systemPrefersDark.value),
)
const themeOverrides = computed<GlobalThemeOverrides>(() => ({
  common: {
    borderRadius: '10px',
    borderRadiusSmall: '7px',
    fontFamily: '"Noto Sans CJK SC", "Source Han Sans SC", "Microsoft YaHei", system-ui, sans-serif',
    fontFamilyMono: '"JetBrains Mono", "SFMono-Regular", Consolas, monospace',
    primaryColor: prefersDark.value ? '#55a98e' : '#2f7d66',
    primaryColorHover: prefersDark.value ? '#68b99f' : '#3d8f76',
    primaryColorPressed: prefersDark.value ? '#428d75' : '#276b58',
    primaryColorSuppl: prefersDark.value ? '#68b99f' : '#3d8f76',
  },
}))

let colorScheme: MediaQueryList | null = null

function syncColorScheme(event: Pick<MediaQueryListEvent, 'matches'> | MediaQueryList) {
  systemPrefersDark.value = event.matches
}

onMounted(() => {
  colorScheme = window.matchMedia('(prefers-color-scheme: dark)')
  syncColorScheme(colorScheme)
  colorScheme.addEventListener('change', syncColorScheme)
})

onBeforeUnmount(() => {
  colorScheme?.removeEventListener('change', syncColorScheme)
  colorScheme = null
})
</script>

<template>
  <NConfigProvider
    :theme="prefersDark ? darkTheme : null"
    :theme-overrides="themeOverrides"
  >
    <DesktopShell @theme-change="themePreference = $event" />
  </NConfigProvider>
</template>
