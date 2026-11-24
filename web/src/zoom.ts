// The image viewer's zoom and pan (ADR 0121), as plain arithmetic: where an
// image of `width` by `height` px sits in a stage at `scale`, its top-left
// corner `x`, `y` px from the stage's.

export interface Size {
  width: number
  height: number
}

export interface View {
  scale: number
  x: number
  y: number
}

/** The closest look, 800%. */
export const MAX_SCALE = 8
/** The margin round an image fitted to its stage. */
export const FIT_PAD = 24
/** Each step of + and −. */
export const STEP = 1.5

/** The scale that fits the image in the stage with a margin, never above actual size. */
export function fitScale(stage: Size, image: Size, pad = FIT_PAD): number {
  if (image.width <= 0 || image.height <= 0) return 1
  const room = (length: number) => Math.max(1, length - 2 * pad)
  return Math.min(room(stage.width) / image.width, room(stage.height) / image.height, 1)
}

/** The least the image zooms out to: half its fit, or its fit when that's tiny. */
export function minScale(stage: Size, image: Size): number {
  return Math.min(fitScale(stage, image) / 2, 1)
}

/** The image fitted and centred. */
export function fitted(stage: Size, image: Size): View {
  return placed({ scale: fitScale(stage, image), x: 0, y: 0 }, stage, image)
}

/** `view` with the image centred on each axis it doesn't fill, and kept covering the stage on each it does. */
export function placed(view: View, stage: Size, image: Size): View {
  const axis = (offset: number, length: number, room: number) =>
    length <= room ? (room - length) / 2 : Math.min(0, Math.max(room - length, offset))
  return {
    scale: view.scale,
    x: axis(view.x, image.width * view.scale, stage.width),
    y: axis(view.y, image.height * view.scale, stage.height),
  }
}

/** `view` zoomed to `scale` about the stage's point `px`, `py`, which stays over the same part of the image. */
export function zoomedAt(view: View, scale: number, px: number, py: number, stage: Size, image: Size): View {
  const next = Math.min(MAX_SCALE, Math.max(minScale(stage, image), scale))
  const ratio = next / view.scale
  return placed({ scale: next, x: px - (px - view.x) * ratio, y: py - (py - view.y) * ratio }, stage, image)
}

/** `view` moved by `dx`, `dy`. */
export function panned(view: View, dx: number, dy: number, stage: Size, image: Size): View {
  return placed({ ...view, x: view.x + dx, y: view.y + dy }, stage, image)
}

/** Whether the image is larger than the stage, so it pans. */
export function pannable(view: View, stage: Size, image: Size): boolean {
  return image.width * view.scale > stage.width + 0.5 || image.height * view.scale > stage.height + 0.5
}

/** The part of the image on screen, as fractions of it, for the navigator. */
export function visible(view: View, stage: Size, image: Size) {
  const width = image.width * view.scale
  const height = image.height * view.scale
  const clamp = (value: number) => Math.min(1, Math.max(0, value))
  return {
    left: clamp(-view.x / width),
    top: clamp(-view.y / height),
    width: clamp(stage.width / width),
    height: clamp(stage.height / height),
  }
}

/** Whether a preview `shownWidth` px across is drawn past its own pixels, so the original would show more. */
export function pastPreview(view: View, image: Size, shownWidth: number | undefined): boolean {
  return shownWidth !== undefined && shownWidth < image.width && image.width * view.scale > shownWidth * 1.15
}

/** How a zoom reads: `100%`. */
export function percent(scale: number): string {
  return `${Math.round(scale * 100)}%`
}
