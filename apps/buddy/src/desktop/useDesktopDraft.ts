import type { ComputedRef } from 'vue'
import type { LocalAttachment, LocalWorkspaceDraft } from '../../electron/shared/localChatApi'
import { shallowRef } from 'vue'
import {
  createDesktopDraftStore,
  reconcileDesktopDraftAfterSend,
} from './desktopChatState'

const ATTACHMENT_COUNT_LIMIT = 16
const ATTACHMENT_TOTAL_BYTES_LIMIT = 32 * 1024 * 1024

interface UseDesktopDraftOptions {
  cleanupDraftAttachments: (retainedAttachmentIds: ReadonlyArray<string>) => Promise<unknown>
  onChange: () => void
  releaseAttachments: (attachmentIds: ReadonlyArray<string>) => Promise<unknown>
  targetKey: ComputedRef<string>
}

export function useDesktopDraft(options: UseDesktopDraftOptions) {
  const store = createDesktopDraftStore()
  const attachments = shallowRef<ReadonlyArray<LocalAttachment>>([])
  const composerContent = shallowRef<LocalWorkspaceDraft['composerContent']>(null)
  const draft = shallowRef('')

  async function appendAttachments(incoming: ReadonlyArray<LocalAttachment>): Promise<number> {
    const accepted = [...attachments.value]
    const rejectedIds: string[] = []
    let totalBytes = accepted.reduce((total, attachment) => total + attachment.sizeBytes, 0)
    for (const attachment of incoming) {
      if (
        accepted.length >= ATTACHMENT_COUNT_LIMIT
        || totalBytes + attachment.sizeBytes > ATTACHMENT_TOTAL_BYTES_LIMIT
      ) {
        rejectedIds.push(attachment.attachmentId)
        continue
      }
      accepted.push(attachment)
      totalBytes += attachment.sizeBytes
    }
    attachments.value = accepted
    saveCurrentDraft(true)
    if (rejectedIds.length)
      await options.releaseAttachments(rejectedIds)

    return rejectedIds.length
  }

  async function removeAttachment(index: number) {
    const removed = attachments.value[index]
    if (!removed)
      return

    attachments.value = attachments.value.filter((_attachment, itemIndex) => itemIndex !== index)
    saveCurrentDraft(true)
    await options.releaseAttachments([removed.attachmentId])
  }

  function updateDraft(content: string) {
    draft.value = content
    composerContent.value = null
    saveCurrentDraft(true)
  }

  function updateComposerContent(content: string, value: LocalWorkspaceDraft['composerContent']) {
    draft.value = content
    composerContent.value = value
    saveCurrentDraft(true)
  }

  function saveCurrentDraft(resetRequestId = false) {
    const current = store.load(options.targetKey.value)
    store.save(options.targetKey.value, {
      attachments: attachments.value,
      composerContent: composerContent.value,
      content: draft.value,
      requestFingerprint: resetRequestId ? null : current.requestFingerprint,
      requestId: resetRequestId ? null : current.requestId,
    })
    options.onChange()
  }

  function restoreCurrentDraft() {
    const storedDraft = store.load(options.targetKey.value)
    draft.value = storedDraft.content
    composerContent.value = storedDraft.composerContent
    attachments.value = storedDraft.attachments
  }

  return {
    appendAttachments,
    attachments,
    composerContent,
    clear(key: string) {
      store.clear(key)
      options.onChange()
    },
    cleanupAbandonedAttachments: () => options.cleanupDraftAttachments(store.attachmentIds()),
    async discard(key: string) {
      const discarded = store.load(key)
      store.clear(key)
      options.onChange()
      if (discarded.attachments.length) {
        await options.releaseAttachments(
          discarded.attachments.map(attachment => attachment.attachmentId),
        )
      }
    },
    draft,
    exportDrafts: () => store.exportDrafts(),
    hydrate: (drafts: Parameters<typeof store.hydrate>[0]) => store.hydrate(drafts),
    load: (key: string) => store.load(key),
    prepareSend(requestId: string, requestFingerprint: string) {
      const prepared = store.prepareSend(options.targetKey.value, requestId, requestFingerprint)
      options.onChange()
      return prepared
    },
    reconcileAfterSend: (
      sourceKey: string,
      conversationKey: string,
      sentDraft: {
        attachments: ReadonlyArray<LocalAttachment>
        composerContent?: LocalWorkspaceDraft['composerContent']
        content: string
      },
    ) => {
      reconcileDesktopDraftAfterSend(store, sourceKey, conversationKey, sentDraft)
      options.onChange()
    },
    removeAttachment,
    restoreCurrentDraft,
    saveCurrentDraft,
    updateDraft,
    updateComposerContent,
  }
}
