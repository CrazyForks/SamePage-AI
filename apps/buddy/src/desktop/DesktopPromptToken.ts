import { mergeAttributes, Node } from '@tiptap/core'
import {
  createDesktopPromptTokenText,
  DESKTOP_PROMPT_TOKEN_NODE_NAME,
} from './desktopComposerInput'

export const DesktopPromptToken = Node.create({
  name: DESKTOP_PROMPT_TOKEN_NODE_NAME,
  group: 'inline',
  inline: true,
  atom: true,
  selectable: true,

  addAttributes() {
    return {
      kind: { default: 'file' },
      label: { default: '' },
      value: { default: '' },
      path: { default: null },
      description: { default: null },
    }
  },

  parseHTML() {
    return [{ tag: 'span[data-type="desktop-prompt-token"]' }]
  },

  renderHTML({ node, HTMLAttributes }) {
    return [
      'span',
      mergeAttributes({
        'class': 'desktop-prompt-token-node',
        'contenteditable': 'false',
        'data-type': 'desktop-prompt-token',
      }, HTMLAttributes),
      createDesktopPromptTokenText({
        description: typeof node.attrs.description === 'string' ? node.attrs.description : null,
        kind: node.attrs.kind,
        label: node.attrs.label,
        path: typeof node.attrs.path === 'string' ? node.attrs.path : null,
        value: node.attrs.value,
      }),
    ]
  },
})
