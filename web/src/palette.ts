// Opening the command palette from anywhere: the sidebar's Search, a phone's header.

export const OPEN_PALETTE = 'silt-palette'

export function openPalette() {
  window.dispatchEvent(new Event(OPEN_PALETTE))
}
