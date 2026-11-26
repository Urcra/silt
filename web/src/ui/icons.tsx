// One inline-SVG icon set: 16 px, 1.4 stroke, drawn in the current color.

import type { ReactNode, SVGProps } from 'react'

const PATHS = {
  thread: <path d="M3 4h10M3 8h10M3 12h6" />,
  merge: (
    <>
      <circle cx="4" cy="3.5" r="1.5" />
      <circle cx="4" cy="12.5" r="1.5" />
      <circle cx="12" cy="8" r="1.5" />
      <path d="M4 5v6M4 5c0 3 2 3 6.5 3" />
    </>
  ),
  branch: (
    <>
      <circle cx="4.5" cy="3.5" r="1.5" />
      <circle cx="4.5" cy="12.5" r="1.5" />
      <circle cx="11.5" cy="5" r="1.5" />
      <path d="M4.5 5v6M11.5 6.5c0 3-7 2-7 4.5" />
    </>
  ),
  inbox: <path d="M2.5 9.5 4 3.5h8l1.5 6v3h-11zM2.5 9.5H6l.5 1.5h3l.5-1.5h3.5" />,
  search: (
    <>
      <circle cx="7" cy="7" r="4" />
      <path d="m10 10 3.5 3.5" />
    </>
  ),
  agent: (
    <>
      <rect x="3" y="5" width="10" height="8" rx="2" />
      <path d="M8 5V2.5M6 9h.01M10 9h.01" />
    </>
  ),
  terminal: <path d="M2.5 3.5h11v9h-11zM5 7l2 1.5L5 10M8.5 10.5H11" />,
  check: <path d="m3.5 8.5 3 3 6-7" />,
  x: <path d="m4 4 8 8M12 4l-8 8" />,
  plus: <path d="M8 3v10M3 8h10" />,
  chevronDown: <path d="m4 6 4 4 4-4" />,
  chevronLeft: <path d="m10 4-4 4 4 4" />,
  expand: <path d="M9.5 2.5h4v4M6.5 13.5h-4v-4M13.5 2.5 9 7M2.5 13.5 7 9" />,
  collapse: <path d="M13 3 9.5 6.5M9.5 3v3.5H13M3 13l3.5-3.5M6.5 13V9.5H3" />,
  download: <path d="M8 2.5v8M4.5 7 8 10.5 11.5 7M3 13.5h10" />,
  reload: <path d="M13 8a5 5 0 1 1-1.5-3.6M13 2.5V5h-2.5" />,
  code: <path d="M5.5 4.5 2 8l3.5 3.5M10.5 4.5 14 8l-3.5 3.5" />,
  chevronRight: <path d="m6 4 4 4-4 4" />,
  play: <path d="M5 3.5v9l7-4.5z" />,
  stop: <rect x="4" y="4" width="8" height="8" rx="1" />,
  alert: <path d="M8 2.5 14 13H2zM8 6.5v3M8 11.5h.01" />,
  info: (
    <>
      <circle cx="8" cy="8" r="5.5" />
      <path d="M8 7.5v3.5M8 5h.01" />
    </>
  ),
  sun: (
    <>
      <circle cx="8" cy="8" r="2.5" />
      <path d="M8 1.5v1.5M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1 1M11.6 11.6l1 1M3.4 12.6l1-1M11.6 4.4l1-1" />
    </>
  ),
  moon: <path d="M13 9.5A5.5 5.5 0 0 1 6.5 3 5.5 5.5 0 1 0 13 9.5z" />,
  monitor: <path d="M2.5 3h11v8h-11zM6 14h4M8 11v3" />,
  copy: <path d="M5.5 5.5h7v8h-7zM3.5 10.5v-8h7" />,
  more: <path d="M4 8h.01M8 8h.01M12 8h.01" />,
  fork: (
    <>
      <circle cx="4" cy="3.5" r="1.5" />
      <circle cx="12" cy="3.5" r="1.5" />
      <circle cx="8" cy="12.5" r="1.5" />
      <path d="M4 5v1.5c0 1.5 1 2 4 2s4-.5 4-2V5M8 8.5V11" />
    </>
  ),
  graph: (
    <>
      <circle cx="5" cy="3.5" r="1.5" />
      <circle cx="5" cy="12.5" r="1.5" />
      <circle cx="11" cy="6" r="1.5" />
      <path d="M5 5v6M11 7.5c0 2.5-6 2-6 3.5" />
    </>
  ),
  laptop: <path d="M3.5 3.5h9v6.5h-9zM1.5 12.5h13" />,
  phone: <path d="M5 1.5h6v13H5zM7.5 12.5h1" />,
  cloud: <path d="M4.5 12.5h7.25a2.75 2.75 0 0 0 .4-5.47A4 4 0 0 0 4.4 6.6 3 3 0 0 0 4.5 12.5z" />,
  clock: (
    <>
      <circle cx="8" cy="8" r="5.5" />
      <path d="M8 5v3l2 1.5" />
    </>
  ),
  lock: <path d="M3 7h10v7H3zM5 7V5a3 3 0 0 1 6 0v2" />,
  shield: <path d="M8 1.75 13 3.5V7.5c0 3-2.1 5.4-5 6.75C5.1 12.9 3 10.5 3 7.5V3.5z" />,
  handoff: <path d="M3 8h10M9 4l4 4-4 4" />,
  link: <path d="M7 9a3 3 0 0 0 4.2 0l2-2a3 3 0 0 0-4.2-4.2l-.6.6M9 7a3 3 0 0 0-4.2 0l-2 2a3 3 0 0 0 4.2 4.2l.6-.6" />,
  comment: <path d="M3 3.5h10v7H8l-3 2.5v-2.5H3z" />,
  eye: (
    <>
      <path d="M1.5 8S4 3.5 8 3.5 14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8z" />
      <circle cx="8" cy="8" r="2" />
    </>
  ),
  revert: <path d="M5.5 3 2.5 6l3 3M2.5 6H10a3.5 3.5 0 0 1 0 7H7" />,
  selector: <path d="M5 6l3-3 3 3M5 10l3 3 3-3" />,
  // A cog, not rays: drawn as rays it read as the sun, the theme's icon.
  settings: (
    <>
      <circle cx="8" cy="8" r="2.2" />
      <path d="M12.8 6.5 14.5 6.9v2.2l-1.7.4-.4.8 1 1.5-1.6 1.6-1.5-1-.8.4-.4 1.7H6.9l-.4-1.7-.8-.4-1.5 1-1.6-1.6 1-1.5-.4-.8-1.7-.4V6.9l1.7-.4.4-.8-1-1.5 1.6-1.6 1.5 1 .8-.4.4-1.7h2.2l.4 1.7.8.4 1.5-1 1.6 1.6-1 1.5z" />
    </>
  ),
  // How a view shows: two sliders, each at its own setting.
  sliders: (
    <>
      <path d="M2.5 4.5h6M11.5 4.5h2M2.5 11.5h2M7.5 11.5h6" />
      <circle cx="10" cy="4.5" r="1.5" />
      <circle cx="6" cy="11.5" r="1.5" />
    </>
  ),
  grid: <path d="M3 3h4v4H3zM9 3h4v4H9zM3 9h4v4H3zM9 9h4v4H9z" />,
  file: <path d="M4 1.5h5l3 3v10H4zM9 1.5v3h3" />,
  panel: <path d="M2.5 2.5h11v11h-11zM10 2.5v11" />,
  bell: <path d="M4 11V7a4 4 0 0 1 8 0v4l1 1.5H3zM6.5 14h3" />,
  list: <path d="M2.5 4h11M2.5 8h11M2.5 12h7" />,
  folder: <path d="M2 4.5A1 1 0 0 1 3 3.5h3l1.5 1.5H13a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1z" />,
  attach: <path d="M13.5 7.5 8 13a3.4 3.4 0 0 1-4.8-4.8l5.6-5.6a2.2 2.2 0 0 1 3.1 3.1L6.3 11.3a1.1 1.1 0 0 1-1.6-1.6l5.1-5.1" />,
  arrowUp: <path d="M8 13V3M3.5 7.5 8 3l4.5 4.5" />,
  mic: <path d="M8 1.5a2 2 0 0 0-2 2v4a2 2 0 0 0 4 0v-4a2 2 0 0 0-2-2zM3.5 7.5a4.5 4.5 0 0 0 9 0M8 12v2.5M5.5 14.5h5" />,
  external: <path d="M9.5 2.5h4v4M13.5 2.5 7.5 8.5M12 9.5v4H2.5V4h4" />,
  edit: <path d="M2.5 13.5h3l7.5-7.5-3-3-7.5 7.5zM9 4l3 3" />,
  image: (
    <>
      <rect x="2" y="3" width="12" height="10" rx="1" />
      <circle cx="6" cy="6.5" r="1" />
      <path d="m14 11-3.5-3.5L4 13" />
    </>
  ),
  video: (
    <>
      <rect x="1.5" y="3.5" width="9.5" height="9" rx="1" />
      <path d="m11 7 3.5-2v6L11 9" />
    </>
  ),
  pause: <path d="M5.5 3.5v9M10.5 3.5v9" strokeWidth="2" />,
  playSolid: <path d="M5 3.5v9l7-4.5z" fill="currentColor" />,
  sound: <path d="M2.5 6h2.5l3.5-3v10L5 10H2.5zM11 5.5a3.5 3.5 0 0 1 0 5M12.8 3.8a6 6 0 0 1 0 8.4" />,
  muted: <path d="M2.5 6h2.5l3.5-3v10L5 10H2.5zM11 6l3.5 4M14.5 6 11 10" />,
  loop: <path d="M3 7V6a2 2 0 0 1 2-2h7.5M10.5 2 12.5 4l-2 2M13 9v1a2 2 0 0 1-2 2H3.5M5.5 14l-2-2 2-2" />,
  pip: (
    <>
      <rect x="2" y="3" width="12" height="10" rx="1" />
      <rect x="8" y="8" width="4.5" height="3.5" rx=".5" fill="currentColor" />
    </>
  ),
  fullscreen: <path d="M2.5 6V2.5H6M10 2.5h3.5V6M13.5 10v3.5H10M6 13.5H2.5V10" />,
  minus: <path d="M3.5 8h9" />,
  keyboard: (
    <>
      <rect x="1.5" y="4" width="13" height="8" rx="1" />
      <path d="M4 6.5h.01M6.5 6.5h.01M9 6.5h.01M11.5 6.5h.01M5 9.5h6" />
    </>
  ),
} satisfies Record<string, ReactNode>

export type IconName = keyof typeof PATHS

/** Decoration unless `label` names it. */
export function Icon({ name, label, ...props }: { name: IconName; label?: string } & SVGProps<SVGSVGElement>) {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.4"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={label ? undefined : true}
      role={label ? 'img' : undefined}
      aria-label={label}
      {...props}
    >
      {PATHS[name]}
    </svg>
  )
}
