import type {
  LocalMessage,
  LocalMessageAttachment,
  LocalRunEvent,
} from '../../electron/shared/localChatApi'

export type BuddyMessage = LocalMessage
export type BuddyMessageAttachment = LocalMessageAttachment
export type BuddyMessageRole = LocalMessage['role']
export type BuddyChatRunEvent = LocalRunEvent
export type BuddyRunEventType = LocalRunEvent['eventType']
