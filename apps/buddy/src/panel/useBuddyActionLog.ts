import type {
  BuddyActionLogPlanDetail,
  BuddyActionLogPlanStatus,
  BuddyActionLogPlanSummary,
  BuddyActionLogResultKind,
} from '@/lib/tauriRuntime'
import { readonly, shallowRef, watch } from 'vue'
import { normalizeBuddyCommandError } from '@/lib/invokeClient'
import {
  getBuddyActionLogPlanDetail,
  listBuddyActionLogPlans,
} from '@/lib/tauriRuntime'

const ACTION_LOG_PAGE_SIZE = 50

export function useBuddyActionLog() {
  const plans = shallowRef<ReadonlyArray<BuddyActionLogPlanSummary>>([])
  const selectedPlanId = shallowRef<string | null>(null)
  const selectedPlanDetail = shallowRef<BuddyActionLogPlanDetail | null>(null)
  const nextPageCursor = shallowRef<string | null>(null)
  const hasMore = shallowRef(false)
  const errorMessage = shallowRef<string | null>(null)
  const isLoadingPlans = shallowRef(false)
  const isLoadingMore = shallowRef(false)
  const isLoadingDetail = shallowRef(false)
  const statusFilter = shallowRef<BuddyActionLogPlanStatus | null>(null)
  const sourceRefKindFilter = shallowRef<string | null>(null)
  const resultKindFilter = shallowRef<BuddyActionLogResultKind | null>(null)
  let listRequestId = 0
  let detailRequestId = 0
  let pendingRequest: Promise<void> | null = null

  async function loadFirstPage() {
    const requestId = ++listRequestId
    isLoadingPlans.value = true
    errorMessage.value = null
    pendingRequest = runListRequest(requestId, null, false)
    await pendingRequest
  }

  async function loadNextPage() {
    if (!hasMore.value || !nextPageCursor.value || isLoadingMore.value)
      return

    const requestId = ++listRequestId
    isLoadingMore.value = true
    errorMessage.value = null
    pendingRequest = runListRequest(requestId, nextPageCursor.value, true)
    await pendingRequest
  }

  async function runListRequest(
    requestId: number,
    pageCursor: string | null,
    append: boolean,
  ) {
    try {
      const list = await listBuddyActionLogPlans({
        limit: ACTION_LOG_PAGE_SIZE,
        pageCursor,
        resultKind: resultKindFilter.value,
        sourceRefKind: sourceRefKindFilter.value,
        status: statusFilter.value,
      })
      if (requestId !== listRequestId)
        return

      plans.value = append
        ? [...plans.value, ...list.items]
        : list.items
      nextPageCursor.value = list.nextPageCursor
      hasMore.value = list.hasMore
      if (!append) {
        const nextSelectedPlanId = list.items[0]?.planId ?? null
        selectedPlanId.value = nextSelectedPlanId
        if (nextSelectedPlanId) {
          await selectPlan(nextSelectedPlanId)
        }
        else {
          detailRequestId++
          selectedPlanDetail.value = null
          isLoadingDetail.value = false
        }
      }
    }
    catch (error) {
      if (requestId === listRequestId)
        errorMessage.value = normalizeBuddyCommandError(error).message
    }
    finally {
      if (requestId === listRequestId) {
        isLoadingPlans.value = false
        isLoadingMore.value = false
      }
    }
  }

  async function selectPlan(planId: string) {
    if (!planId)
      return

    selectedPlanId.value = planId
    selectedPlanDetail.value = null
    const requestId = ++detailRequestId
    isLoadingDetail.value = true
    errorMessage.value = null
    try {
      const detail = await getBuddyActionLogPlanDetail(planId)
      if (requestId !== detailRequestId || selectedPlanId.value !== planId)
        return

      selectedPlanDetail.value = detail
    }
    catch (error) {
      if (requestId === detailRequestId)
        errorMessage.value = normalizeBuddyCommandError(error).message
    }
    finally {
      if (requestId === detailRequestId)
        isLoadingDetail.value = false
    }
  }

  async function flushPendingRequests() {
    await pendingRequest
  }

  watch(
    () => [statusFilter.value, sourceRefKindFilter.value, resultKindFilter.value] as const,
    () => {
      void loadFirstPage()
    },
  )

  return {
    errorMessage: readonly(errorMessage),
    flushPendingRequests,
    hasMore: readonly(hasMore),
    isLoadingDetail: readonly(isLoadingDetail),
    isLoadingMore: readonly(isLoadingMore),
    isLoadingPlans: readonly(isLoadingPlans),
    loadFirstPage,
    loadNextPage,
    nextPageCursor: readonly(nextPageCursor),
    plans: readonly(plans),
    resultKindFilter,
    selectedPlanDetail: readonly(selectedPlanDetail),
    selectedPlanId: readonly(selectedPlanId),
    selectPlan,
    sourceRefKindFilter,
    statusFilter,
  }
}
