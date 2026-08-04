import type { MaybeRefOrGetter } from 'vue'
import type { BuddyChoreographyMacroIntentRunner } from './useBuddyChoreographyMacroIntentSync'
import type { BuddyApproval, BuddyChoreographyMacroIntent } from '@/lib/tauriRuntime'
import type { BuddyChoreographyMacroIntentApprovalSourceRef } from '@/pet/buddyHostAction'
import { computed, toValue } from 'vue'
import { useBuddyChoreographyMacroIntentSync } from './useBuddyChoreographyMacroIntentSync'

const AWAIT_APPROVAL_MACRO_INTENT: BuddyChoreographyMacroIntent = {
  macroId: 'awaitApproval',
  params: {},
}

export interface UseBuddyApprovalChoreographyMacroIntentSyncOptions {
  approvals: MaybeRefOrGetter<ReadonlyArray<BuddyApproval>>
  enabled?: MaybeRefOrGetter<boolean>
  runMacroIntent?: BuddyChoreographyMacroIntentRunner
  startedAtUnixMs?: MaybeRefOrGetter<number | null | undefined>
}

export function useBuddyApprovalChoreographyMacroIntentSync(
  options: UseBuddyApprovalChoreographyMacroIntentSyncOptions,
) {
  const target = computed(() =>
    resolvePendingApprovalChoreographyTarget(
      toValue(options.approvals),
      toValue(options.startedAtUnixMs),
    ),
  )

  return useBuddyChoreographyMacroIntentSync({
    enabled: () => toValue(options.enabled) !== false,
    intent: () => target.value ? AWAIT_APPROVAL_MACRO_INTENT : null,
    playbackKey: () => createPendingApprovalChoreographyPlaybackKey(
      toValue(options.approvals),
      toValue(options.startedAtUnixMs),
    ),
    runMacroIntent: options.runMacroIntent,
    sourceRef: () => target.value
      ? createPendingApprovalChoreographySourceRef(target.value)
      : null,
  })
}

export function createPendingApprovalChoreographySourceRef(
  approval: BuddyApproval,
): BuddyChoreographyMacroIntentApprovalSourceRef {
  const sourceRef: BuddyChoreographyMacroIntentApprovalSourceRef = {
    approvalId: approval.id,
    kind: 'approval',
  }
  if (approval.runId)
    sourceRef.runId = approval.runId

  return sourceRef
}

export function createPendingApprovalChoreographyPlaybackKey(
  approvals: ReadonlyArray<BuddyApproval>,
  startedAtUnixMs?: number | null,
): string | null {
  const ids = filterPendingApprovalsForChoreography(approvals, startedAtUnixMs)
    .map(approval => approval.id)
    .sort((left, right) => left.localeCompare(right))

  return ids.length > 0 ? `approval:${ids.join(',')}` : null
}

export function resolvePendingApprovalChoreographyTarget(
  approvals: ReadonlyArray<BuddyApproval>,
  startedAtUnixMs?: number | null,
): BuddyApproval | null {
  return filterPendingApprovalsForChoreography(approvals, startedAtUnixMs)
    .sort(comparePendingApprovalChoreographyPriority)[0] ?? null
}

function filterPendingApprovalsForChoreography(
  approvals: ReadonlyArray<BuddyApproval>,
  startedAtUnixMs?: number | null,
): BuddyApproval[] {
  return approvals.filter((approval) => {
    if (approval.status !== 'pending' || !approval.id)
      return false
    if (typeof startedAtUnixMs !== 'number' || !Number.isFinite(startedAtUnixMs))
      return true

    const createdAtUnixMs = Date.parse(approval.createdAt)
    return Number.isFinite(createdAtUnixMs) && createdAtUnixMs >= startedAtUnixMs
  })
}

function comparePendingApprovalChoreographyPriority(
  left: BuddyApproval,
  right: BuddyApproval,
): number {
  const leftCreatedAt = Date.parse(left.createdAt)
  const rightCreatedAt = Date.parse(right.createdAt)

  if (Number.isFinite(leftCreatedAt) && Number.isFinite(rightCreatedAt) && leftCreatedAt !== rightCreatedAt)
    return rightCreatedAt - leftCreatedAt

  return left.id.localeCompare(right.id)
}
