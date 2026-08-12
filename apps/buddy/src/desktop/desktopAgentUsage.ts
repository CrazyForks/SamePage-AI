import type { LocalUsageSnapshot } from '../../electron/shared/localChatApi'

type LocalUsageRecord = LocalUsageSnapshot['records'][number]
type LocalUsageRuntime = LocalUsageRecord['runtime']
export interface DesktopUsageTotals {
  cacheCreationTokens: number
  cacheReadTokens: number
  inputTokens: number
  outputTokens: number
  recordCount: number
  totalTokens: number
}

export interface DesktopAgentUsage {
  daily: ReadonlyMap<string, DesktopUsageTotals>
  latestDate: string | null
  records: ReadonlyArray<LocalUsageRecord>
  totals: DesktopUsageTotals
}

export type DesktopUsagePresentation
  = | 'empty'
    | 'empty-error'
    | 'initial-loading'
    | 'ready'
    | 'refreshing'
    | 'stale-error'

interface DesktopUsagePresentationOptions {
  hasError: boolean
  hasSnapshot: boolean
  isLoading: boolean
}

export function resolveDesktopUsagePresentation(
  options: DesktopUsagePresentationOptions,
): DesktopUsagePresentation {
  if (options.isLoading)
    return options.hasSnapshot ? 'refreshing' : 'initial-loading'
  if (options.hasError)
    return options.hasSnapshot ? 'stale-error' : 'empty-error'

  return options.hasSnapshot ? 'ready' : 'empty'
}

export function createDesktopAgentUsage(
  snapshot: LocalUsageSnapshot | null,
  runtime: LocalUsageRuntime,
): DesktopAgentUsage {
  const records = snapshot?.records.filter(record => record.runtime === runtime) ?? []
  const totals = createEmptyUsageTotals()
  const daily = new Map<string, DesktopUsageTotals>()
  let latestDate: string | null = null

  for (const record of records) {
    addUsageRecord(totals, record)
    if (!record.date)
      continue

    const aggregate = daily.get(record.date) ?? createEmptyUsageTotals()
    addUsageRecord(aggregate, record)
    daily.set(record.date, aggregate)
    if (latestDate === null || record.date > latestDate)
      latestDate = record.date
  }

  return {
    daily,
    latestDate,
    records,
    totals,
  }
}

function createEmptyUsageTotals(): DesktopUsageTotals {
  return {
    cacheCreationTokens: 0,
    cacheReadTokens: 0,
    inputTokens: 0,
    outputTokens: 0,
    recordCount: 0,
    totalTokens: 0,
  }
}

function addUsageRecord(totals: DesktopUsageTotals, record: LocalUsageRecord) {
  totals.cacheCreationTokens += record.cacheCreationTokens
  totals.cacheReadTokens += record.cacheReadTokens
  totals.inputTokens += record.inputTokens
  totals.outputTokens += record.outputTokens
  totals.recordCount += 1
  totals.totalTokens += record.totalTokens
}
