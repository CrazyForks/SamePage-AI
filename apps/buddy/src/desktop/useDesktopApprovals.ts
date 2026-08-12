import type { Ref } from 'vue'
import type {
  LocalApproval,
  LocalChatApi,
} from '../../electron/shared/localChatApi'
import { computed, readonly, shallowRef } from 'vue'
import { projectDesktopApproval } from './desktopChatState'

interface UseDesktopApprovalsOptions {
  api: LocalChatApi
  approvals: Ref<ReadonlyArray<LocalApproval>>
  onError: (error: unknown) => void
  refresh: () => Promise<void>
}

export function useDesktopApprovals(options: UseDesktopApprovalsOptions) {
  const resolvingApprovalIds = shallowRef<ReadonlySet<string>>(new Set())
  const approvalViews = computed(() => options.approvals.value.map(projectDesktopApproval))

  async function resolveApproval(approvalId: string, decision: 'approve' | 'deny') {
    if (resolvingApprovalIds.value.has(approvalId))
      return

    const approval = options.approvals.value.find(item => item.id === approvalId)
    if (!approval || (decision === 'approve' && approval.kind !== 'run.codex_app_server_request'))
      return

    resolvingApprovalIds.value = new Set([...resolvingApprovalIds.value, approvalId])
    try {
      if (decision === 'deny')
        await options.api.approvals.deny(approvalId)
      else
        await options.api.approvals.approveCodex(approvalId)
      await options.refresh()
    }
    catch (error) {
      options.onError(error)
    }
    finally {
      const next = new Set(resolvingApprovalIds.value)
      next.delete(approvalId)
      resolvingApprovalIds.value = next
    }
  }

  return {
    approvalViews: readonly(approvalViews),
    resolveApproval,
    resolvingApprovalIds: readonly(resolvingApprovalIds),
  }
}
