import type { BuddyMessageAttachment } from '@/desktop/localChatTypes'

const ATTACHMENT_PREVIEW_PROTOCOL = 'lexora-attachment:'

export function resolveBuddyAttachmentPreviewUrl(
  attachment: BuddyMessageAttachment,
): string | undefined {
  if (attachment.dataUrl)
    return attachment.dataUrl

  if (attachment.kind !== 'image' || !attachment.attachmentId)
    return undefined

  const url = new URL(`${ATTACHMENT_PREVIEW_PROTOCOL}//preview/`)
  url.pathname = `/${encodeURIComponent(attachment.attachmentId)}`
  return url.toString()
}
