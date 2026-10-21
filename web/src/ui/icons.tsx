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
