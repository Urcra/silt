// What the viewer and the player show (ADR 0121): an image by its previews and its original, a video by its blob,
// and how each reads in a caption.

import { blobUrl, type ArtifactView, type ImageView, type PreviewView } from './api'

/** An image as the viewer opens it: from `src`, which may be a preview `shownWidth` px across, with its original. */
export interface ImageMedia {
  kind: 'image'
  title: string
  /** The line under the title: its format, size and where it came from. */
  meta?: string
  src: string
  /** The width of `src` in px, when it's a preview of an image `width` px across. */
  shownWidth?: number
  /** The original, when `src` is a preview: where it is and how many bytes it is. */
  full?: { src: string; size: number }
  width?: number
  height?: number
  /** Where Download saves it from. */
  download?: string
  /** The blob it reads, to ask how much has arrived when it can't be read yet. */
  state?: { thread: string; blob: string; size: number }
}

export interface VideoMedia {
  kind: 'video'
  title: string
  meta?: string
  src: string
  mediaType: string
  download?: string
  state?: { thread: string; blob: string; size: number }
}

export type Media = ImageMedia | VideoMedia

/** The long edge of the preview the viewer opens on. */
export const VIEWER_EDGE = 2048

/** A size as people read it. */
export function sizeOf(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`
}

/** A time in a video: `0:06`, `12:40`, `1:02:09`. */
export function clock(seconds: number): string {
  const total = Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds) : 0
  const hours = Math.floor(total / 3600)
  const minutes = Math.floor((total % 3600) / 60)
  const secs = String(total % 60).padStart(2, '0')
  return hours > 0 ? `${hours}:${String(minutes).padStart(2, '0')}:${secs}` : `${minutes}:${secs}`
}

/** A media type as a caption names it. */
export function formatOf(mediaType: string): string {
  const named: Record<string, string> = {
    'video/mp4': 'MP4',
    'video/quicktime': 'MOV',
    'video/webm': 'WebM',
    'video/x-matroska': 'MKV',
    'image/svg+xml': 'SVG',
    'image/jpeg': 'JPEG',
  }
  return named[mediaType] ?? mediaType.replace(/^\w+\//, '').toUpperCase()
}

/** What a binary file is, by its name: an image or a video plays on its page (ADR 0121). */
export function binaryKind(path: string): string {
  const extension = path.split('.').pop()?.toLowerCase() ?? ''
  if (['png', 'jpg', 'jpeg', 'gif', 'webp'].includes(extension)) return 'image'
  if (['mp4', 'm4v', 'mov', 'webm', 'mkv'].includes(extension)) return 'video'
  return 'binary'
}

export function isVideo(mediaType: string): boolean {
  return mediaType.startsWith('video/')
}

/** The caption's facts: format, pixels and size. */
export function metaOf(mediaType: string, size: number, width?: number | null, height?: number | null): string {
  const pixels = width && height ? ` · ${width}×${height}` : ''
  return `${formatOf(mediaType)}${pixels} · ${sizeOf(size)}`
}

/** The preview a thumbnail shows: the smallest, else the image itself. */
export function thumbSrc(thread: string, blob: string, previews: PreviewView[] | undefined): string {
  const smallest = previews?.[0]
  return blobUrl(thread, smallest ? smallest.blob : blob)
}

/** An image of `previews` as the viewer opens it: on its largest preview, with Full resolution for the original. */
function viewed(
  thread: string,
  title: string,
  blob: string,
  mediaType: string,
  size: number,
  previews: PreviewView[],
  width?: number | null,
  height?: number | null,
  name?: string,
): ImageMedia {
  const largest = previews.findLast((preview) => Math.max(preview.width, preview.height) >= VIEWER_EDGE * 0.9)
  const original = blobUrl(thread, blob)
  return {
    kind: 'image',
    title,
    meta: metaOf(mediaType, size, width, height),
    src: largest ? blobUrl(thread, largest.blob) : original,
    shownWidth: largest?.width,
    full: largest ? { src: original, size } : undefined,
    width: width ?? undefined,
    height: height ?? undefined,
    download: blobUrl(thread, blob, { download: name ?? title }),
    state: { thread, blob, size },
  }
}

/** An image or a video an agent showed, or an artifact's version of one. */
export function artifactMedia(thread: string, artifact: ArtifactView): Media {
  const title = artifact.caption ?? artifact.title
  if (isVideo(artifact.media_type))
    return {
      kind: 'video',
      title,
      meta: metaOf(artifact.media_type, artifact.size),
      src: blobUrl(thread, artifact.blob),
      mediaType: artifact.media_type,
      download: blobUrl(thread, artifact.blob, { download: title }),
      state: { thread, blob: artifact.blob, size: artifact.size },
    }
  return viewed(
    thread,
    title,
    artifact.blob,
    artifact.media_type,
    artifact.size,
    artifact.previews ?? [],
    artifact.width,
    artifact.height,
  )
}

/** A prompt's image or video, kept as a blob or carried in its line. */
export function entryMedia(thread: string | undefined, image: ImageView, title: string): Media | null {
  const stored = image.stored
  if (stored && thread) {
    const name = stored.name ?? title
    if (isVideo(image.media_type))
      return {
        kind: 'video',
        title: name,
        meta: metaOf(image.media_type, stored.size),
        src: blobUrl(thread, stored.blob),
        mediaType: image.media_type,
        download: blobUrl(thread, stored.blob, { download: name }),
        state: { thread, blob: stored.blob, size: stored.size },
      }
    return viewed(
      thread,
      name,
      stored.blob,
      image.media_type,
      stored.size,
      stored.previews ?? [],
      stored.width,
      stored.height,
      name,
    )
  }
  if (!image.data) return null
  return { kind: 'image', title, src: `data:${image.media_type};base64,${image.data}` }
}

/** Fetches `url` whole as a blob, telling `progress` how many bytes have come. */
export async function fetchWithProgress(url: string, progress: (loaded: number) => void, signal?: AbortSignal): Promise<Blob> {
  const response = await fetch(url, { signal })
  if (!response.ok || !response.body) throw new Error(`couldn't load it (${response.status})`)
  const reader = response.body.getReader()
  const parts: Uint8Array[] = []
  let loaded = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    parts.push(value)
    loaded += value.length
    progress(loaded)
  }
  return new Blob(parts as BlobPart[], { type: response.headers.get('content-type') ?? undefined })
}
