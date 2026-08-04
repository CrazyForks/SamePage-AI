import type { BuddyLocale, BuddyTranslate } from '@/i18n/buddyI18n'
import type {
  BuddyActionLogPlanDetail,
  BuddyActionLogPlanStatus,
  BuddyActionLogPlanSummary,
  BuddyActionLogResultKind,
  BuddyActionLogStepDetail,
} from '@/lib/tauriRuntime'

const ACTION_LOG_REASON_LABEL_KEYS = {
  'devFixture.completed': 'actionLog.reason.devFixtureCompleted',
  'devFixture.failed': 'actionLog.reason.devFixtureFailed',
  'devFixture.started': 'actionLog.reason.devFixtureStarted',
  'devFixture.stepCompleted': 'actionLog.reason.devFixtureStepCompleted',
  'devFixture.stepFailed': 'actionLog.reason.devFixtureStepFailed',
  'devFixture.stepResolved': 'actionLog.reason.devFixtureStepResolved',
  'devFixture.yieldedToPendingPlan': 'actionLog.reason.devFixtureYieldedToPendingPlan',
  'timeline.completed': 'actionLog.reason.timelineCompleted',
  'timeline.failed': 'actionLog.reason.timelineFailed',
  'timeline.started': 'actionLog.reason.timelineStarted',
  'timeline.stepCompleted': 'actionLog.reason.timelineStepCompleted',
  'timeline.stepFailed': 'actionLog.reason.timelineStepFailed',
  'timeline.stepResolved': 'actionLog.reason.timelineStepResolved',
  'timeline.yieldedToPendingPlan': 'actionLog.reason.timelineYieldedToPendingPlan',
  'executor.accepted': 'actionLog.reason.executorAccepted',
  'executor.busy': 'actionLog.reason.executorBusy',
  'admission.waitingForActiveStepToFinish': 'actionLog.reason.admissionWaitingForActiveStepToFinish',
  'priority.tooLow': 'actionLog.reason.priorityTooLow',
  'fallback.registrySelected': 'actionLog.reason.fallbackRegistrySelected',
  'runtime.restarted': 'actionLog.reason.runtimeRestarted',
  'presetBehavior.completed': 'actionLog.reason.presetBehaviorCompleted',
  'presetBehavior.resolved': 'actionLog.reason.presetBehaviorResolved',
  'presetBehavior.started': 'actionLog.reason.presetBehaviorStarted',
  'presetBehavior.stepCompleted': 'actionLog.reason.presetBehaviorStepCompleted',
  'run.hostAction.completed': 'actionLog.reason.runHostActionCompleted',
  'run.hostAction.failed': 'actionLog.reason.runHostActionFailed',
  'run.hostAction.resolved': 'actionLog.reason.runHostActionResolved',
  'run.hostAction.started': 'actionLog.reason.runHostActionStarted',
  'run.hostAction.stepCompleted': 'actionLog.reason.runHostActionStepCompleted',
  'run.hostAction.stepFailed': 'actionLog.reason.runHostActionStepFailed',
  'sidecar.stepInterrupted': 'actionLog.reason.sidecarStepInterrupted',
  'systemRecovery.completed': 'actionLog.reason.systemRecoveryCompleted',
  'systemRecovery.failed': 'actionLog.reason.systemRecoveryFailed',
  'systemRecovery.started': 'actionLog.reason.systemRecoveryStarted',
  'systemRecovery.stepCompleted': 'actionLog.reason.systemRecoveryStepCompleted',
  'systemRecovery.stepFailed': 'actionLog.reason.systemRecoveryStepFailed',
  'systemRecovery.stepResolved': 'actionLog.reason.systemRecoveryStepResolved',
} as const satisfies Record<string, Parameters<BuddyTranslate>[0]>

export interface BuddyActionLogPlanRow {
  animationLabel: string
  completedAtLabel: string
  id: string
  key: string
  reasonLabel: string
  sourceDetailLabel: string
  sourceLabel: string
  startedAtLabel: string
  status: string
  statusLabel: string
  statusType: 'default' | 'error' | 'success' | 'warning'
  title: string
}

export interface BuddyActionLogStepRow {
  actionLabel: string
  clipLabel: string
  durationLabel: string
  id: string
  key: string
  reasonLabel: string
  status: string
  statusLabel: string
  statusType: 'default' | 'error' | 'success' | 'warning'
  timeLabel: string
}

export function createActionLogPlanRows(
  plans: ReadonlyArray<BuddyActionLogPlanSummary>,
  t: BuddyTranslate,
  locale: BuddyLocale,
): ReadonlyArray<BuddyActionLogPlanRow> {
  return plans.map(plan => ({
    animationLabel: plan.resolvedAnimationRef || t('common.missing'),
    completedAtLabel: formatActionLogDateTime(plan.completedAt, t, locale),
    id: plan.planId,
    key: plan.planId,
    reasonLabel: resolveActionLogReasonLabel(
      plan.detailReasonCode || plan.lastReasonCode,
      t,
    ),
    sourceDetailLabel: plan.sourceDisplay?.subtitle ?? '',
    sourceLabel: plan.sourceDisplay?.title || resolveActionLogSourceLabel(plan.sourceRefKind, t),
    startedAtLabel: formatActionLogDateTime(plan.startedAt, t, locale),
    status: plan.status,
    statusLabel: resolveActionLogStatusLabel(plan.status, t),
    statusType: resolveActionLogStatusType(plan.status),
    title: plan.resolvedActionId || plan.planId,
  }))
}

export function createActionLogStepRows(
  detail: BuddyActionLogPlanDetail | null,
  t: BuddyTranslate,
  locale: BuddyLocale,
): ReadonlyArray<BuddyActionLogStepRow> {
  if (!detail)
    return []

  const rows = detail.steps.map(step =>
    createActionLogStepRow(step, detail.plan.planId, t, locale),
  )
  const recoveryRows = detail.recoveryPlans.flatMap(recovery =>
    recovery.steps.map(step =>
      createActionLogStepRow(
        step,
        recovery.plan.planId,
        t,
        locale,
        t('actionLog.section.systemRecovery'),
      ),
    ),
  )

  return [...rows, ...recoveryRows]
}

export function resolveActionLogStatusLabel(status: string, t: BuddyTranslate) {
  if (status === 'completed')
    return t('actionLog.status.completed')

  if (status === 'failed')
    return t('actionLog.status.failed')

  if (status === 'rejected')
    return t('actionLog.status.rejected')

  if (status === 'running')
    return t('actionLog.status.running')

  if (status === 'deferred')
    return t('actionLog.status.deferred')

  if (status === 'interrupted')
    return t('actionLog.status.interrupted')

  if (status === 'skipped')
    return t('actionLog.status.skipped')

  return status || t('common.missing')
}

export function resolveActionLogSourceLabel(sourceRefKind: string, t: BuddyTranslate) {
  if (sourceRefKind === 'conversationMessage')
    return t('actionLog.source.conversationMessage')

  if (sourceRefKind === 'run')
    return t('actionLog.source.run')

  if (sourceRefKind === 'approval')
    return t('actionLog.source.approval')

  if (sourceRefKind === 'presetBehavior')
    return t('actionLog.source.presetBehavior')

  if (sourceRefKind === 'systemRecovery')
    return t('actionLog.source.systemRecovery')

  if (sourceRefKind === 'macroFallback')
    return t('actionLog.source.macroFallback')

  if (sourceRefKind === 'startupSystem')
    return t('actionLog.source.startupSystem')

  if (sourceRefKind === 'choreographyScheduler')
    return t('actionLog.source.choreographyScheduler')

  if (sourceRefKind === 'devFixture')
    return t('actionLog.source.devFixture')

  return sourceRefKind || t('common.missing')
}

export function resolveActionLogResultKindLabel(resultKind: string, t: BuddyTranslate) {
  if (resultKind === 'normal')
    return t('actionLog.result.normal')

  if (resultKind === 'fallback')
    return t('actionLog.result.fallback')

  if (resultKind === 'degraded')
    return t('actionLog.result.degraded')

  if (resultKind === 'interrupted')
    return t('actionLog.result.interrupted')

  return resultKind || t('common.missing')
}

export function resolveActionLogReasonLabel(reasonCode: string, t: BuddyTranslate) {
  const labelKey = ACTION_LOG_REASON_LABEL_KEYS[
    reasonCode as keyof typeof ACTION_LOG_REASON_LABEL_KEYS
  ]
  if (labelKey)
    return t(labelKey)

  return reasonCode || t('common.missing')
}

export function resolveActionLogStatusType(
  status: string,
): BuddyActionLogPlanRow['statusType'] {
  if (status === 'completed')
    return 'success'

  if (status === 'failed')
    return 'error'

  if (status === 'rejected')
    return 'error'

  if (status === 'running')
    return 'warning'

  if (status === 'deferred')
    return 'warning'

  if (status === 'interrupted')
    return 'warning'

  if (status === 'skipped')
    return 'default'

  return 'default'
}

export function createActionLogStatusOptions(t: BuddyTranslate) {
  return [
    {
      label: t('actionLog.filter.allStatuses'),
      value: 'all',
    },
    {
      label: resolveActionLogStatusLabel('completed', t),
      value: 'completed' satisfies BuddyActionLogPlanStatus,
    },
    {
      label: resolveActionLogStatusLabel('running', t),
      value: 'running' satisfies BuddyActionLogPlanStatus,
    },
    {
      label: resolveActionLogStatusLabel('rejected', t),
      value: 'rejected' satisfies BuddyActionLogPlanStatus,
    },
    {
      label: resolveActionLogStatusLabel('deferred', t),
      value: 'deferred' satisfies BuddyActionLogPlanStatus,
    },
    {
      label: resolveActionLogStatusLabel('interrupted', t),
      value: 'interrupted' satisfies BuddyActionLogPlanStatus,
    },
    {
      label: resolveActionLogStatusLabel('skipped', t),
      value: 'skipped' satisfies BuddyActionLogPlanStatus,
    },
    {
      label: resolveActionLogStatusLabel('failed', t),
      value: 'failed' satisfies BuddyActionLogPlanStatus,
    },
  ]
}

export function createActionLogSourceOptions(t: BuddyTranslate) {
  return [
    {
      label: t('actionLog.filter.allSources'),
      value: 'all',
    },
    {
      label: resolveActionLogSourceLabel('conversationMessage', t),
      value: 'conversationMessage',
    },
    {
      label: resolveActionLogSourceLabel('run', t),
      value: 'run',
    },
    {
      label: resolveActionLogSourceLabel('approval', t),
      value: 'approval',
    },
    {
      label: resolveActionLogSourceLabel('presetBehavior', t),
      value: 'presetBehavior',
    },
    {
      label: resolveActionLogSourceLabel('systemRecovery', t),
      value: 'systemRecovery',
    },
    {
      label: resolveActionLogSourceLabel('macroFallback', t),
      value: 'macroFallback',
    },
    {
      label: resolveActionLogSourceLabel('devFixture', t),
      value: 'devFixture',
    },
  ]
}

export function createActionLogResultKindOptions(t: BuddyTranslate) {
  return [
    {
      label: t('actionLog.filter.allResults'),
      value: 'all',
    },
    {
      label: resolveActionLogResultKindLabel('normal', t),
      value: 'normal' satisfies BuddyActionLogResultKind,
    },
    {
      label: resolveActionLogResultKindLabel('fallback', t),
      value: 'fallback' satisfies BuddyActionLogResultKind,
    },
    {
      label: resolveActionLogResultKindLabel('degraded', t),
      value: 'degraded' satisfies BuddyActionLogResultKind,
    },
    {
      label: resolveActionLogResultKindLabel('interrupted', t),
      value: 'interrupted' satisfies BuddyActionLogResultKind,
    },
  ]
}

function createActionLogStepRow(
  step: BuddyActionLogStepDetail,
  planId: string,
  t: BuddyTranslate,
  locale: BuddyLocale,
  sectionLabel?: string,
): BuddyActionLogStepRow {
  const actionLabel = step.resolvedActionId || step.stepKind || t('common.missing')

  return {
    actionLabel: sectionLabel ? `${sectionLabel} · ${actionLabel}` : actionLabel,
    clipLabel: step.resolvedAnimationRef || step.targetLabel || t('common.missing'),
    durationLabel: formatActionLogDuration(step.elapsedMs ?? step.durationMs, t),
    id: step.stepId,
    key: `${planId}:${step.stepId}`,
    reasonLabel: resolveActionLogReasonLabel(step.reasonCode, t),
    status: step.status,
    statusLabel: resolveActionLogStatusLabel(step.status, t),
    statusType: resolveActionLogStatusType(step.status),
    timeLabel: formatActionLogDateTime(
      step.completedAt ?? step.failedAt ?? step.resolvedAt,
      t,
      locale,
    ),
  }
}

function formatActionLogDateTime(
  value: string | null | undefined,
  t: BuddyTranslate,
  locale: BuddyLocale,
) {
  if (!value)
    return t('common.missing')

  const date = new Date(value)
  if (Number.isNaN(date.getTime()))
    return value

  return new Intl.DateTimeFormat(locale, {
    day: '2-digit',
    hour: '2-digit',
    hourCycle: 'h23',
    minute: '2-digit',
    month: '2-digit',
  }).format(date)
}

function formatActionLogDuration(
  value: number | null | undefined,
  t: BuddyTranslate,
) {
  if (value === null || value === undefined)
    return t('common.missing')

  return `${Math.max(0, Math.round(value))}ms`
}
