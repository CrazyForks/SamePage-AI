<script setup lang="ts">
defineProps<{
  open: boolean
}>()
</script>

<template>
  <svg
    aria-hidden="true"
    viewBox="0 0 16 16"
    :class="{ 'is-open': open }"
  >
    <path d="M5.75 3.5 10.25 8l-4.5 4.5" />
  </svg>
</template>

<style scoped>
svg {
  fill: none;
  stroke: currentcolor;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 1.5;
  transition: transform 120ms ease;
}

svg.is-open {
  transform: rotate(90deg);
}
</style>
