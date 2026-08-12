import type { Ref } from 'vue'
import { nextTick, onBeforeUnmount, watch } from 'vue'

const FOCUSABLE_SELECTOR = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(',')

export function useModalFocusTrap(
  isOpen: () => boolean,
  dialog: Ref<HTMLElement | null>,
): void {
  let restoreTarget: HTMLElement | null = null

  function focusableElements(): HTMLElement[] {
    return dialog.value
      ? [...dialog.value.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)]
      : []
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key !== 'Tab')
      return

    const elements = focusableElements()
    if (elements.length === 0) {
      event.preventDefault()
      dialog.value?.focus()
      return
    }

    const first = elements[0]!
    const last = elements.at(-1)!
    const activeElement = document.activeElement
    const activeIndex = activeElement instanceof HTMLElement
      ? elements.indexOf(activeElement)
      : -1
    if (!dialog.value?.contains(activeElement) || activeIndex < 0) {
      event.preventDefault()
      const focusTarget = event.shiftKey ? last : first
      focusTarget.focus()
    }
    else if (event.shiftKey && activeIndex === 0) {
      event.preventDefault()
      last.focus()
    }
    else if (!event.shiftKey && activeIndex === elements.length - 1) {
      event.preventDefault()
      first.focus()
    }
  }

  function restoreFocus() {
    document.removeEventListener('keydown', handleKeydown)
    if (restoreTarget?.isConnected)
      restoreTarget.focus()
    restoreTarget = null
  }

  watch(isOpen, async (open) => {
    if (!open) {
      restoreFocus()
      return
    }

    restoreTarget = document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null
    document.addEventListener('keydown', handleKeydown)
    await nextTick()
    const elements = focusableElements()
    const autofocus = dialog.value?.querySelector<HTMLElement>('[autofocus]')
    const focusTarget = autofocus ?? elements[0] ?? dialog.value
    focusTarget?.focus()
  }, { flush: 'post' })

  onBeforeUnmount(restoreFocus)
}
