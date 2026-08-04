import type { MaybeRefOrGetter, Ref } from 'vue'
import type { BuddyChoreographyMacroIntent } from '@/lib/tauriRuntime'
import type {
  BuddyChoreographyMacroIntentRuntimeFields,
  BuddyChoreographyMacroIntentSourceRef,
} from '@/pet/buddyHostAction'
import { shallowRef, toValue, watch } from 'vue'
import { runBuddyChoreographyMacroIntent } from '@/lib/tauriRuntime'

const CHOREOGRAPHY_MACRO_INTENT_RETRY_DELAYS_MS = [250, 1_000] as const
const CHOREOGRAPHY_MACRO_INTENT_RETRYABLE_ERROR_FRAGMENTS = [
  'native pet sidecar is restarting',
  'native pet sidecar stdin is unavailable',
] as const

export type BuddyChoreographyMacroIntentRunner = (
  intent: BuddyChoreographyMacroIntent,
  sourceRef?: BuddyChoreographyMacroIntentSourceRef | null,
  runtime?: BuddyChoreographyMacroIntentRuntimeFields | null,
) => Promise<unknown> | unknown

export type BuddyChoreographyMacroIntentPlaybackKey = number | string

export interface UseBuddyChoreographyMacroIntentSyncOptions {
  enabled?: MaybeRefOrGetter<boolean>
  intent: MaybeRefOrGetter<BuddyChoreographyMacroIntent | null | undefined>
  playbackKey?: MaybeRefOrGetter<BuddyChoreographyMacroIntentPlaybackKey | null | undefined>
  runMacroIntent?: BuddyChoreographyMacroIntentRunner
  runtime?: MaybeRefOrGetter<BuddyChoreographyMacroIntentRuntimeFields | null | undefined>
  sourceRef?: MaybeRefOrGetter<BuddyChoreographyMacroIntentSourceRef | null | undefined>
}

export interface UseBuddyChoreographyMacroIntentSyncResult {
  lastPlaybackError: Ref<unknown | null>
  lastSyncedPlaybackKey: Ref<string | null>
}

export function useBuddyChoreographyMacroIntentSync(
  options: UseBuddyChoreographyMacroIntentSyncOptions,
): UseBuddyChoreographyMacroIntentSyncResult {
  const inFlightPlaybackKey = shallowRef<string | null>(null)
  const lastPlaybackError = shallowRef<unknown | null>(null)
  const lastSyncedPlaybackKey = shallowRef<string | null>(null)

  watch(
    () => [
      resolveChoreographyMacroIntentSyncKey(options),
      toValue(options.enabled) !== false,
    ] as const,
    ([syncKey, enabled], _previous, onCleanup) => {
      if (
        !enabled
        || !syncKey
        || syncKey === lastSyncedPlaybackKey.value
        || syncKey === inFlightPlaybackKey.value
      ) {
        return
      }

      const intent = toValue(options.intent) ?? null
      if (!intent)
        return

      inFlightPlaybackKey.value = syncKey
      lastPlaybackError.value = null
      let cancelled = false
      let cancelRetryWait: (() => void) | null = null
      onCleanup(() => {
        cancelled = true
        cancelRetryWait?.()
      })

      const runner = options.runMacroIntent ?? runBuddyChoreographyMacroIntent
      const sourceRef = toValue(options.sourceRef) ?? null
      const runtime = toValue(options.runtime) ?? null

      void runChoreographyMacroIntentWithRetry({
        intent,
        isCurrent: () => (
          !cancelled
          && toValue(options.enabled) !== false
          && resolveChoreographyMacroIntentSyncKey(options) === syncKey
        ),
        onError: (error) => {
          lastPlaybackError.value = error
        },
        registerRetryCancellation: (cancel) => {
          cancelRetryWait = cancel
        },
        runner,
        runtime,
        sourceRef,
      })
        .then((synced) => {
          if (!synced)
            return

          lastPlaybackError.value = null
          lastSyncedPlaybackKey.value = syncKey
        })
        .finally(() => {
          if (inFlightPlaybackKey.value === syncKey)
            inFlightPlaybackKey.value = null
        })
    },
    { immediate: true },
  )

  return {
    lastPlaybackError,
    lastSyncedPlaybackKey,
  }
}

interface RunChoreographyMacroIntentWithRetryOptions {
  intent: BuddyChoreographyMacroIntent
  isCurrent: () => boolean
  onError: (error: unknown) => void
  registerRetryCancellation: (cancel: (() => void) | null) => void
  runner: BuddyChoreographyMacroIntentRunner
  runtime: BuddyChoreographyMacroIntentRuntimeFields | null
  sourceRef: BuddyChoreographyMacroIntentSourceRef | null
}

async function runChoreographyMacroIntentWithRetry(
  options: RunChoreographyMacroIntentWithRetryOptions,
): Promise<boolean> {
  for (let attempt = 0; ; attempt += 1) {
    try {
      await options.runner(options.intent, options.sourceRef, options.runtime)
      return options.isCurrent()
    }
    catch (error: unknown) {
      if (!options.isCurrent())
        return false

      options.onError(error)
      if (!isRetryableChoreographyMacroIntentError(error))
        return false

      const retryDelayMs = CHOREOGRAPHY_MACRO_INTENT_RETRY_DELAYS_MS[attempt]
      if (retryDelayMs === undefined)
        return false

      const shouldRetry = await waitForChoreographyMacroIntentRetry(
        retryDelayMs,
        options.registerRetryCancellation,
      )
      if (!shouldRetry || !options.isCurrent())
        return false
    }
  }
}

function isRetryableChoreographyMacroIntentError(error: unknown): boolean {
  return error instanceof Error
    && CHOREOGRAPHY_MACRO_INTENT_RETRYABLE_ERROR_FRAGMENTS.some(fragment => (
      error.message.includes(fragment)
    ))
}

function waitForChoreographyMacroIntentRetry(
  delayMs: number,
  registerCancellation: (cancel: (() => void) | null) => void,
): Promise<boolean> {
  return new Promise((resolve) => {
    let settled = false
    let timer: ReturnType<typeof setTimeout> | null = null
    const settle = (shouldRetry: boolean) => {
      if (settled)
        return

      settled = true
      if (timer !== null)
        clearTimeout(timer)
      registerCancellation(null)
      resolve(shouldRetry)
    }
    timer = setTimeout(settle, delayMs, true)
    registerCancellation(() => settle(false))
  })
}

function resolveChoreographyMacroIntentSyncKey(
  options: UseBuddyChoreographyMacroIntentSyncOptions,
): string | null {
  const intent = toValue(options.intent) ?? null
  if (!intent)
    return null

  const playbackKey = toValue(options.playbackKey)
  if (playbackKey !== null && playbackKey !== undefined)
    return String(playbackKey)

  return JSON.stringify(intent)
}
