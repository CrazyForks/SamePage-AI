import type { JSONContent } from '@tiptap/core'
import type {
  LocalCodexInput,
  LocalPromptContextOption,
} from '../../electron/shared/localChatApi'

export const DESKTOP_PROMPT_TOKEN_NODE_NAME = 'desktopPromptToken'

export interface DesktopComposerContextOptions {
  files: ReadonlyArray<LocalPromptContextOption>
  plugins: ReadonlyArray<LocalPromptContextOption>
  skills: ReadonlyArray<LocalPromptContextOption>
}

export interface DesktopComposerTrigger {
  kind: 'slash' | 'skill' | 'mention'
  query: string
}

export interface DesktopComposerSubmitPayload {
  content: string
  contextItems: ReadonlyArray<LocalPromptContextOption>
  inputs: ReadonlyArray<LocalCodexInput>
}

export interface DesktopPromptTokenAttrs {
  description: string | null
  kind: LocalPromptContextOption['kind']
  label: string
  path: string | null
  value: string
}

const SLASH_COMMANDS: ReadonlyArray<LocalPromptContextOption> = [
  {
    description: '先拆计划，再进入执行。',
    kind: 'slashCommand',
    label: '/plan',
    path: null,
    value: '/plan',
  },
  {
    description: '按代码审查方式优先找风险、缺陷和测试缺口。',
    kind: 'slashCommand',
    label: '/review',
    path: null,
    value: '/review',
  },
  {
    description: '查看当前任务、运行状态和关键上下文。',
    kind: 'slashCommand',
    label: '/status',
    path: null,
    value: '/status',
  },
  {
    description: '请求 Codex 进入技能选择上下文。',
    kind: 'slashCommand',
    label: '/skills',
    path: null,
    value: '/skills',
  },
  {
    description: '请求 Codex 进入插件选择上下文。',
    kind: 'slashCommand',
    label: '/plugins',
    path: null,
    value: '/plugins',
  },
]

const TRIGGER_BOUNDARY_PATTERN = /[\s([{，。！？；：、"'`]$/u

export function createEmptyDesktopComposerContent(): JSONContent {
  return {
    type: 'doc',
    content: [{ type: 'paragraph' }],
  }
}

export function createDesktopComposerContentFromText(text: string): JSONContent {
  return {
    type: 'doc',
    content: text.split('\n').map(line => ({
      type: 'paragraph',
      content: line ? [{ type: 'text', text: line }] : undefined,
    })),
  }
}

export function serializeDesktopComposerContent(content: JSONContent): DesktopComposerSubmitPayload {
  const state: SerializeState = {
    contextItems: [],
    inputContextItems: [],
    text: '',
    textElements: [],
  }
  serializeNode(content, state)

  const trimmed = trimSerializedText(state.text, state.textElements)
  const inputs: LocalCodexInput[] = []
  if (trimmed.text) {
    inputs.push({
      type: 'text',
      text: trimmed.text,
      text_elements: trimmed.textElements,
    })
  }

  for (const item of state.inputContextItems) {
    if (item.kind === 'skill' && item.path) {
      inputs.push({ type: 'skill', name: item.value, path: item.path })
    }
    else if (item.kind === 'file' && item.path) {
      inputs.push({ type: 'mention', name: item.label, path: item.path })
    }
  }

  return {
    content: trimmed.text,
    contextItems: state.contextItems,
    inputs,
  }
}

export function findDesktopComposerTrigger(textBeforeCursor: string): DesktopComposerTrigger | null {
  const slashIndex = findTriggerStart(textBeforeCursor, '/')
  const skillIndex = findTriggerStart(textBeforeCursor, '$')
  const mentionIndex = findTriggerStart(textBeforeCursor, '@')
  const triggerIndex = Math.max(slashIndex, skillIndex, mentionIndex)
  if (triggerIndex < 0)
    return null

  const query = textBeforeCursor.slice(triggerIndex + 1)
  if (query.includes('\n'))
    return null

  const trigger = textBeforeCursor[triggerIndex]
  return {
    kind: trigger === '/' ? 'slash' : trigger === '$' ? 'skill' : 'mention',
    query,
  }
}

export function createDesktopComposerSuggestions(
  trigger: DesktopComposerTrigger | null,
  options: DesktopComposerContextOptions,
) {
  if (!trigger)
    return []

  const candidates = trigger.kind === 'slash'
    ? SLASH_COMMANDS
    : trigger.kind === 'skill'
      ? options.skills
      : [...options.plugins, ...options.files]
  const query = normalizeSearchText(trigger.query)

  return candidates
    .map(option => ({ option, searchText: createOptionSearchText(option) }))
    .filter(suggestion => normalizeSearchText(suggestion.searchText).includes(query))
    .slice(0, 8)
}

export function shouldSubmitDesktopComposerKey(
  event: Pick<KeyboardEvent, 'altKey' | 'ctrlKey' | 'isComposing' | 'key' | 'metaKey' | 'shiftKey'>,
) {
  return event.key === 'Enter'
    && !event.isComposing
    && !event.shiftKey
    && !event.altKey
    && !event.ctrlKey
    && !event.metaKey
}

export function createDesktopPromptTokenAttrs(option: LocalPromptContextOption): DesktopPromptTokenAttrs {
  return {
    description: option.description,
    kind: option.kind,
    label: option.label,
    path: option.path,
    value: option.value,
  }
}

export function createDesktopPromptTokenText(attrs: DesktopPromptTokenAttrs) {
  if (attrs.kind === 'skill')
    return `$${attrs.label || attrs.value}`
  if (attrs.kind === 'slashCommand')
    return attrs.value

  return `@${attrs.label || attrs.value}`
}

interface SerializeState {
  contextItems: LocalPromptContextOption[]
  inputContextItems: LocalPromptContextOption[]
  text: string
  textElements: Array<{
    byteRange: { start: number, end: number }
    placeholder: string | null
  }>
}

function serializeNode(node: JSONContent, state: SerializeState) {
  if (node.type === 'text') {
    state.text += node.text ?? ''
    return
  }
  if (node.type === 'hardBreak') {
    state.text += '\n'
    return
  }
  if (node.type === DESKTOP_PROMPT_TOKEN_NODE_NAME) {
    appendPromptToken(state, readPromptTokenAttrs(node.attrs))
    return
  }
  if (!node.content?.length)
    return

  node.content.forEach((child, index) => {
    serializeNode(child, state)
    if (node.type === 'doc' && index < node.content!.length - 1)
      state.text += '\n'
  })
}

function appendPromptToken(state: SerializeState, attrs: DesktopPromptTokenAttrs) {
  const placeholder = createDesktopPromptTokenText(attrs)
  const start = byteLength(state.text)
  state.text += placeholder
  state.textElements.push({
    byteRange: { start, end: byteLength(state.text) },
    placeholder,
  })

  const item: LocalPromptContextOption = { ...attrs }
  state.contextItems.push(item)
  if (attrs.kind === 'skill' || attrs.kind === 'file')
    state.inputContextItems.push(item)
}

function trimSerializedText(text: string, textElements: SerializeState['textElements']) {
  const leadingLength = text.length - text.trimStart().length
  const trailingStart = text.trimEnd().length
  const leadingBytes = byteLength(text.slice(0, leadingLength))
  const trailingBytes = byteLength(text.slice(0, trailingStart))

  return {
    text: text.trim(),
    textElements: textElements
      .map(element => ({
        ...element,
        byteRange: {
          start: element.byteRange.start - leadingBytes,
          end: element.byteRange.end - leadingBytes,
        },
      }))
      .filter((element) => {
        const originalStart = element.byteRange.start + leadingBytes
        return originalStart >= leadingBytes && originalStart < trailingBytes
      }),
  }
}

function readPromptTokenAttrs(attrs: JSONContent['attrs']): DesktopPromptTokenAttrs {
  const value = readString(attrs?.value)
  return {
    description: readNullableString(attrs?.description),
    kind: readPromptTokenKind(attrs?.kind),
    label: readString(attrs?.label) || value,
    path: readNullableString(attrs?.path),
    value,
  }
}

function readPromptTokenKind(value: unknown): LocalPromptContextOption['kind'] {
  return value === 'skill' || value === 'plugin' || value === 'file' || value === 'slashCommand'
    ? value
    : 'file'
}

function findTriggerStart(value: string, trigger: '/' | '$' | '@') {
  const index = value.lastIndexOf(trigger)
  if (index < 0 || (index > 0 && !TRIGGER_BOUNDARY_PATTERN.test(value[index - 1] ?? '')))
    return -1

  return index
}

function createOptionSearchText(option: LocalPromptContextOption) {
  return [option.label, option.value, option.path, option.description].filter(Boolean).join(' ')
}

function normalizeSearchText(value: string) {
  return value.trim().toLowerCase()
}

function readString(value: unknown) {
  return typeof value === 'string' ? value : ''
}

function readNullableString(value: unknown) {
  return typeof value === 'string' && value ? value : null
}

function byteLength(value: string) {
  return new TextEncoder().encode(value).length
}
