import type { BuddyChatRunEvent, BuddyChoreographyMacroIntent } from '@/lib/tauriRuntime'
import type { BuddyAnimationPriority } from '@/pet/buddyAnimation'

export interface BuddyResolvedChoreographyMacroIntent {
  createdAt?: string
  eventId?: number
  intent: BuddyChoreographyMacroIntent
  runId?: string
  runtime?: BuddyChoreographyMacroIntentRuntimeFields
}

export type BuddyChoreographyMacroIntentSourceRef
  = | BuddyChoreographyMacroIntentConversationMessageSourceRef
    | BuddyChoreographyMacroIntentRunSourceRef
    | BuddyChoreographyMacroIntentApprovalSourceRef

export interface BuddyChoreographyMacroIntentConversationMessageSourceRef {
  conversationId: string
  kind: 'conversationMessage'
  messageId: string
  runId?: string | null
}

export interface BuddyChoreographyMacroIntentRunSourceRef {
  kind: 'run'
  runId: string
}

export interface BuddyChoreographyMacroIntentApprovalSourceRef {
  approvalId: string
  kind: 'approval'
  runId?: string | null
}

export interface BuddyChoreographyMacroIntentRuntimeFields {
  priority?: BuddyAnimationPriority
  reason?: string
}

const BUDDY_NATIVE_PET_HOST_PRIORITIES = new Set<BuddyAnimationPriority>([
  'background',
  'high',
  'normal',
  'urgent',
])
const BUDDY_HOST_ACTION_REASON_MAX_LENGTH = 120
const BUDDY_HOST_ACTION_SOURCE = 'buddy_builtin_host_skill'

export const BUDDY_HOST_ACTION_PUBLIC_MACRO_IDS = [
  'celebrate',
  'dance',
  'lieDown',
  'patrolAroundScreen',
  'reassure',
  'sad',
  'thinking',
  'working',
  'curious',
  'awaitApproval',
  'getUp',
  'peekFromEdge',
  'peekBehindWindow',
  'cast',
] as const satisfies ReadonlyArray<BuddyChoreographyMacroIntent['macroId']>

export const BUDDY_HOST_ACTION_PUBLIC_MACRO_PARAM_BOUNDS = {
  dance: {
    durationMs: {
      max: 30_000,
      min: 1_000,
    },
  },
  patrolAroundScreen: {
    loops: {
      max: 4,
      min: 1,
    },
  },
  peekBehindWindow: {
    durationMs: {
      max: 15_000,
      min: 500,
    },
  },
} as const

const BUDDY_HOST_ACTION_EMPTY_PARAM_MACRO_IDS = new Set<BuddyChoreographyMacroIntent['macroId']>([
  'awaitApproval',
  'cast',
  'celebrate',
  'curious',
  'lieDown',
  'reassure',
  'sad',
  'thinking',
  'working',
])

export function resolveBuddyChoreographyMacroIntentFromRunEvents(
  events: ReadonlyArray<BuddyChatRunEvent>,
): BuddyResolvedChoreographyMacroIntent | null {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index]
    if (event.eventType !== 'host.action')
      continue

    const resolved = normalizeBuddyChoreographyMacroIntent(event.payload)
    if (!resolved)
      continue

    return {
      createdAt: event.createdAt,
      eventId: event.id,
      intent: resolved.intent,
      runId: event.runId,
      runtime: resolved.runtime,
    }
  }

  return null
}

export function createBuddyChoreographyMacroIntentPlaybackKey(
  resolved: BuddyResolvedChoreographyMacroIntent | null,
): string | null {
  if (!resolved)
    return null

  return `host.action.macroIntent:${resolved.runId ?? 'run'}:${resolved.eventId ?? 'event'}`
}

export function resolveBuddyChoreographyMacroIntentSourceRef(
  resolved: BuddyResolvedChoreographyMacroIntent | null,
): BuddyChoreographyMacroIntentSourceRef | null {
  if (!resolved?.runId)
    return null

  return {
    kind: 'run',
    runId: resolved.runId,
  }
}

export function isBuddyChoreographyMacroIntentFresh(
  resolved: BuddyResolvedChoreographyMacroIntent | null,
  shellStartedAtUnixMs: number,
): boolean {
  return isBuddyHostActionEventFresh(resolved, shellStartedAtUnixMs)
}

function isBuddyHostActionEventFresh(
  resolved: { createdAt?: string } | null,
  shellStartedAtUnixMs: number,
): boolean {
  if (!resolved?.createdAt || !Number.isFinite(shellStartedAtUnixMs))
    return false

  const createdAtUnixMs = Date.parse(resolved.createdAt)
  if (!Number.isFinite(createdAtUnixMs))
    return false

  return createdAtUnixMs >= shellStartedAtUnixMs
}

function normalizeBuddyChoreographyMacroIntent(
  payload: unknown,
): { intent: BuddyChoreographyMacroIntent, runtime?: BuddyChoreographyMacroIntentRuntimeFields } | null {
  if (!isRecord(payload))
    return null

  if (!hasOnlyRecordKeys(payload, ['action', 'intent', 'version', 'priority', 'reason', 'source']))
    return null

  if (payload.version !== 1)
    return null

  if (payload.action !== 'macroIntent')
    return null

  const runtime = normalizeBuddyNativePetHostCommonFields(payload)
  if (!runtime)
    return null

  const intent = normalizeBuddyChoreographyMacroIntentValue(payload.intent)
  if (!intent)
    return null

  return hasBuddyChoreographyMacroIntentRuntimeFields(runtime) ? { intent, runtime } : { intent }
}

function normalizeBuddyChoreographyMacroIntentValue(
  value: unknown,
): BuddyChoreographyMacroIntent | null {
  if (
    !isRecord(value)
    || !hasExactRecordKeys(value, ['macroId', 'params'])
    || typeof value.macroId !== 'string'
  ) {
    return null
  }

  const params = isRecord(value.params) ? value.params : null
  if (!params)
    return null

  if (isBuddyHostActionEmptyParamMacroId(value.macroId)) {
    return isEmptyRecord(params)
      ? { macroId: value.macroId, params: {} }
      : null
  }

  switch (value.macroId) {
    case 'dance':
      return hasExactRecordKeys(params, ['durationMs'])
        && isDanceMacroDurationMs(params.durationMs)
        ? { macroId: 'dance', params: { durationMs: params.durationMs } }
        : null
    case 'patrolAroundScreen':
      return hasExactRecordKeys(params, ['loops'])
        && isFiniteLoopCount(params.loops)
        ? { macroId: 'patrolAroundScreen', params: { loops: params.loops } }
        : null
    case 'getUp':
      return hasExactRecordKeys(params, ['side'])
        && isGetUpSide(params.side)
        ? { macroId: 'getUp', params: { side: params.side } }
        : null
    case 'peekFromEdge':
      return hasExactRecordKeys(params, ['edge'])
        && isBuddyNativePetEdge(params.edge)
        ? { macroId: 'peekFromEdge', params: { edge: params.edge } }
        : null
    case 'peekBehindWindow':
      return normalizePeekBehindWindowMacroIntent(params)
    default:
      return null
  }
}

function normalizePeekBehindWindowMacroIntent(
  params: Record<string, unknown>,
): BuddyChoreographyMacroIntent | null {
  if (
    !hasExactRecordKeys(params, ['windowSelector', 'edge', 'reveal', 'durationMs'])
    || !isRecord(params.windowSelector)
    || !hasExactRecordKeys(params.windowSelector, ['kind'])
    || params.windowSelector.kind !== 'activeWindow'
    || !isBuddyNativePetWindowAnchorEdge(params.edge)
    || params.reveal !== 'head'
    || !isPeekBehindWindowMacroDurationMs(params.durationMs)
  ) {
    return null
  }

  return {
    macroId: 'peekBehindWindow',
    params: {
      durationMs: params.durationMs,
      edge: params.edge,
      reveal: 'head',
      windowSelector: {
        kind: 'activeWindow',
      },
    },
  }
}

function normalizeBuddyNativePetHostCommonFields(
  payload: Record<string, unknown>,
): BuddyChoreographyMacroIntentRuntimeFields | null {
  const priority = payload.priority
  if (priority !== undefined && !isBuddyNativePetHostPriority(priority))
    return null

  const reason = payload.reason
  if (reason !== undefined && !isBuddyNativePetHostReason(reason))
    return null

  if (payload.source !== BUDDY_HOST_ACTION_SOURCE)
    return null

  const fields: BuddyChoreographyMacroIntentRuntimeFields = {}
  if (priority !== undefined)
    fields.priority = priority
  if (reason !== undefined)
    fields.reason = reason

  return fields
}

function hasBuddyChoreographyMacroIntentRuntimeFields(
  value: BuddyChoreographyMacroIntentRuntimeFields,
): boolean {
  return value.priority !== undefined || value.reason !== undefined
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isBuddyNativePetHostPriority(value: unknown): value is BuddyAnimationPriority {
  return typeof value === 'string'
    && BUDDY_NATIVE_PET_HOST_PRIORITIES.has(value as BuddyAnimationPriority)
}

function isBuddyNativePetHostReason(value: unknown): value is string {
  return typeof value === 'string'
    && value.length > 0
    && value.length <= BUDDY_HOST_ACTION_REASON_MAX_LENGTH
    && /^[a-z][a-z0-9]*(?:_[a-z0-9]+)*$/.test(value)
}

function isBuddyHostActionEmptyParamMacroId(
  value: string,
): value is 'awaitApproval' | 'cast' | 'celebrate' | 'curious' | 'lieDown' | 'reassure' | 'sad' | 'thinking' | 'working' {
  return BUDDY_HOST_ACTION_EMPTY_PARAM_MACRO_IDS.has(value as BuddyChoreographyMacroIntent['macroId'])
}

function isBuddyNativePetEdge(value: unknown): value is 'left' | 'right' | 'top' | 'bottom' {
  return value === 'left' || value === 'right' || value === 'top' || value === 'bottom'
}

function isBuddyNativePetWindowAnchorEdge(
  value: unknown,
): value is 'auto' | 'left' | 'right' | 'top' | 'bottom' {
  return value === 'auto' || isBuddyNativePetEdge(value)
}

function isGetUpSide(value: unknown): value is 'left' | 'right' {
  return value === 'left' || value === 'right'
}

function isDanceMacroDurationMs(value: unknown): value is number {
  const bounds = BUDDY_HOST_ACTION_PUBLIC_MACRO_PARAM_BOUNDS.dance.durationMs

  return isIntegerDurationMsInRange(
    value,
    bounds.min,
    bounds.max,
  )
}

function isPeekBehindWindowMacroDurationMs(value: unknown): value is number {
  const bounds = BUDDY_HOST_ACTION_PUBLIC_MACRO_PARAM_BOUNDS.peekBehindWindow.durationMs

  return isIntegerDurationMsInRange(
    value,
    bounds.min,
    bounds.max,
  )
}

function isIntegerDurationMsInRange(
  value: unknown,
  minDurationMs: number,
  maxDurationMs: number,
): value is number {
  return typeof value === 'number'
    && Number.isInteger(value)
    && value >= minDurationMs
    && value <= maxDurationMs
}

function isFiniteLoopCount(value: unknown): value is number {
  const bounds = BUDDY_HOST_ACTION_PUBLIC_MACRO_PARAM_BOUNDS.patrolAroundScreen.loops

  return Number.isInteger(value)
    && Number(value) >= bounds.min
    && Number(value) <= bounds.max
}

function isEmptyRecord(value: Record<string, unknown>): boolean {
  return Object.keys(value).length === 0
}

function hasExactRecordKeys(
  value: Record<string, unknown>,
  keys: readonly string[],
): boolean {
  const actualKeys = Object.keys(value)
  return actualKeys.length === keys.length
    && keys.every(key => Object.hasOwn(value, key))
}

function hasOnlyRecordKeys(
  value: Record<string, unknown>,
  keys: readonly string[],
): boolean {
  return Object.keys(value).every(key => keys.includes(key))
}
