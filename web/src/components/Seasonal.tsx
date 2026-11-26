// December's decorations in the desktop app (ADR 0133). Each one is drawn over
// the page, takes no pointer events and is hidden from screen readers, and
// takes its colours from the theme's tokens, so the theme stays as it is.

import { useEffect, useRef, useState } from 'react'
import { useMotionReduced, useSeasonal } from '../seasonal'
import { cx } from '../ui'

/** The bulbs' and baubles' colours, in turn: the theme's own. */
const TONES = ['var(--danger)', 'var(--ok)', 'var(--warn)', 'var(--info)', 'var(--accent)']
const SNOW = 'light-dark(#ffffff, #f4efe6)'
/** Snow on a light ground needs an edge to be seen; on a dark one it has none. */
const SNOW_EDGE = 'light-dark(rgb(0 0 0 / 0.3), transparent)'

function useWidth<T extends Element>() {
  const ref = useRef<T>(null)
  const [width, setWidth] = useState(0)
  useEffect(() => {
    const element = ref.current
    if (!element || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(([entry]) => setWidth(Math.round(entry?.contentRect.width ?? 0)))
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  return [ref, width] as const
}

const SPAN = 66

/** A string of lights hanging from the bottom edge of a page's header. */
export function SeasonalLights() {
  return useSeasonal().on ? <Lights /> : null
}

function Lights() {
  const [ref, width] = useWidth<HTMLSpanElement>()
  const scallops = Math.ceil(width / SPAN)
  let wire = 'M0 1'
  for (let at = 0; at < scallops; at++) wire += ` Q ${at * SPAN + SPAN / 2} 11 ${(at + 1) * SPAN} 1`
  return (
    <span
      ref={ref}
      aria-hidden="true"
      data-testid="seasonal-lights"
      className="pointer-events-none absolute inset-x-0 top-full -mt-px block h-4 overflow-hidden"
    >
      {width > 0 && (
        <svg width={width} height={16} viewBox={`0 0 ${width} 16`} className="block">
          <path d={wire} fill="none" stroke="var(--border-strong)" strokeWidth={1.2} opacity={0.75} />
          {Array.from({ length: scallops }, (_, at) => {
            const x = at * SPAN + SPAN / 2
            const delay = `${-((at * 1.37) % 3.6)}s`
            return (
              <g key={at}>
                <rect x={x - 1.4} y={5} width={2.8} height={2.6} rx={0.6} fill="var(--fg-3)" />
                <ellipse
                  cx={x}
                  cy={10.2}
                  rx={2.5}
                  ry={3.3}
                  fill={TONES[at % TONES.length]}
                  className="silt-twinkle"
                  style={{ animationDelay: delay }}
                />
              </g>
            )
          })}
        </svg>
      )}
    </span>
  )
}

/** A hat on the workspace's tile, at its top right. */
export function SeasonalHat() {
  const { on } = useSeasonal()
  if (!on) return null
  return (
    <svg
      aria-hidden="true"
      data-testid="seasonal-hat"
      width={17}
      height={15}
      viewBox="0 0 17 15"
      className="pointer-events-none absolute -top-2 -right-2.5 rotate-[14deg] overflow-visible"
    >
      <path d="M2.5 11.5 C4 5 8 1.2 13 2.4 C15.6 3 16.4 6 15.2 8.6 C14 6.8 12.4 6.4 11.4 7.4 L12.6 11.5 Z" fill="var(--danger)" />
      <circle cx={15.3} cy={9.4} r={1.9} fill={SNOW} stroke={SNOW_EDGE} strokeWidth={0.8} />
      <rect x={1} y={10.6} width={13.4} height={3.6} rx={1.8} fill={SNOW} stroke={SNOW_EDGE} strokeWidth={0.8} />
    </svg>
  )
}

/** The same drift for the same `seed`, so it doesn't shift as the page draws again. */
function drift(seed: number): string {
  let state = seed
  const next = () => (state = (state * 9301 + 49297) % 233280) / 233280
  let path = 'M0 9'
  for (let x = 0; x < 400;) {
    const step = 18 + next() * 40
    path += ` Q ${(x + step / 2).toFixed(1)} ${(0.5 + next() * 3).toFixed(1)} ${Math.min(x + step, 400).toFixed(1)} ${(4 + next() * 3).toFixed(1)}`
    x += step
  }
  return path
}

/** Snow resting on the top edge of the box it's in, which must be positioned. */
export function SnowDrift({ seed = 11, className }: { seed?: number; className?: string }) {
  const { on } = useSeasonal()
  if (!on) return null
  const top = drift(seed)
  return (
    <svg
      aria-hidden="true"
      data-testid="seasonal-drift"
      viewBox="0 0 400 11"
      preserveAspectRatio="none"
      height={11}
      className={cx('pointer-events-none absolute -top-2 -right-px -left-px z-[1] w-[calc(100%+2px)]', className)}
    >
      <path d={`${top} L 400 9.6 L 0 9.6 Z`} fill={SNOW} />
      <path d={`${top} L 400 9`} fill="none" stroke={SNOW_EDGE} strokeWidth={0.8} vectorEffect="non-scaling-stroke" />
    </svg>
  )
}

const ROWS: [number, number][] = [
  [38, 14],
  [62, 26],
  [86, 38],
  [110, 50],
]

/**
 * A fir drawn as a branch graph: branches fork out of `main` and merge back,
 * with commits for baubles. `lit` baubles are lit; `dark` puts out the star too.
 */
export function BranchTree({ lit = 8, dark = false, className }: { lit?: number; dark?: boolean; className?: string }) {
  const mid = 66
  const branch = {
    fill: 'none',
    stroke: 'var(--ok)',
    strokeWidth: 2,
    strokeLinecap: 'round' as const,
  }
  let bauble = 0
  return (
    <svg aria-hidden="true" data-testid="seasonal-tree" viewBox="0 0 132 168" className={className}>
      <path d={`M${mid} 22 V140`} {...branch} />
      {ROWS.map(([y, w]) =>
        [-1, 1].map((side) => (
          <g key={`${y}${side}`}>
            <path d={`M${mid} ${y - 14} C ${mid} ${y - 4}, ${mid + side * w} ${y - 10}, ${mid + side * w} ${y}`} {...branch} />
            <path
              d={`M${mid + side * w} ${y} C ${mid + side * w} ${y + 8}, ${mid} ${y + 6}, ${mid} ${y + 14}`}
              {...branch}
              opacity={0.45}
            />
          </g>
        )),
      )}
      {ROWS.flatMap(([y, w], row) =>
        [-1, 1].map((side) => {
          const at = bauble++
          const on = !dark && at < lit
          return (
            <circle
              key={`${y}${side}`}
              cx={mid + side * w}
              cy={y}
              r={5}
              stroke="var(--bg)"
              strokeWidth={2.5}
              fill={on ? TONES[(row * 2 + (side > 0 ? 3 : 0)) % TONES.length] : 'var(--surface-3)'}
            />
          )
        }),
      )}
      <path
        d={`M${mid} 4 l3.2 6.6 7.2 1 -5.2 5 1.3 7.1 -6.5 -3.4 -6.5 3.4 1.3 -7.1 -5.2 -5 7.2 -1z`}
        fill={dark ? 'var(--surface-3)' : 'var(--accent)'}
      />
      <rect x={mid - 9} y={140} width={18} height={12} rx={2} fill="var(--surface-2)" stroke="var(--border-strong)" />
      <text x={mid} y={165} textAnchor="middle" fontFamily="var(--font-mono)" fontSize={10} fontWeight={500} fill="var(--fg-3)">
        main
      </text>
    </svg>
  )
}

/** The tree over an empty screen's question; it sits above it, so nothing under it moves. */
export function SeasonalTree() {
  const { on } = useSeasonal()
  if (!on) return null
  return (
    <BranchTree className="pointer-events-none absolute bottom-full left-1/2 mb-4 h-[107px] w-[84px] -translate-x-1/2 [@media(max-height:640px)]:hidden" />
  )
}

interface Flake {
  x: number
  y: number
  r: number
  alpha: number
  fall: number
  sway: number
}

function luminance(color: string): number {
  const [r = 0, g = 0, b = 0] = (color.match(/\d+(\.\d+)?/g) ?? []).map(Number)
  return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255
}

/**
 * Snow falling behind a screen with nothing to read yet. It lies still with
 * less motion, and stops while the window is hidden.
 */
export function Snowfall() {
  const { snow } = useSeasonal()
  const reduced = useMotionReduced()
  const canvas = useRef<HTMLCanvasElement>(null)
  useEffect(() => {
    const element = canvas.current
    const context = element?.getContext('2d')
    if (!element || !context) return
    let flakes: Flake[] = []
    let width = 0
    let height = 0
    let frame = 0
    let last = 0
    const size = () => {
      const ratio = window.devicePixelRatio || 1
      width = element.clientWidth
      height = element.clientHeight
      element.width = width * ratio
      element.height = height * ratio
      context.setTransform(ratio, 0, 0, ratio, 0, 0)
      let seed = 5
      const next = () => (seed = (seed * 9301 + 49297) % 233280) / 233280
      flakes = Array.from({ length: Math.round((width * height) / 14_000) }, () => ({
        x: next() * width,
        y: next() * height,
        r: 1 + next() * 2.2,
        alpha: 0.45 + next() * 0.45,
        fall: 8 + next() * 14,
        sway: next() * 6.28,
      }))
    }
    const paint = () => {
      const light = luminance(getComputedStyle(document.body).backgroundColor) > 0.5
      context.clearRect(0, 0, width, height)
      context.fillStyle = getComputedStyle(element).color
      context.shadowColor = light ? 'rgba(0, 0, 0, 0.35)' : 'transparent'
      context.shadowBlur = light ? 2 : 0
      for (const flake of flakes) {
        context.globalAlpha = flake.alpha
        context.beginPath()
        context.arc(flake.x, flake.y, flake.r, 0, Math.PI * 2)
        context.fill()
      }
      context.globalAlpha = 1
    }
    const tick = (now: number) => {
      const seconds = Math.min(0.05, (now - (last || now)) / 1000)
      last = now
      for (const flake of flakes) {
        flake.y += flake.fall * seconds
        flake.sway += seconds * 0.8
        flake.x += Math.sin(flake.sway) * 6 * seconds
        if (flake.y > height + 4) {
          flake.y = -4
          flake.x = Math.random() * width
        }
      }
      paint()
      frame = requestAnimationFrame(tick)
    }
    const run = () => {
      cancelAnimationFrame(frame)
      last = 0
      if (reduced || document.hidden) paint()
      else frame = requestAnimationFrame(tick)
    }
    size()
    run()
    const resized =
      typeof ResizeObserver === 'undefined'
        ? null
        : new ResizeObserver(() => {
            size()
            run()
          })
    resized?.observe(element)
    document.addEventListener('visibilitychange', run)
    return () => {
      cancelAnimationFrame(frame)
      resized?.disconnect()
      document.removeEventListener('visibilitychange', run)
    }
  }, [snow, reduced])
  if (!snow) return null
  return (
    <canvas
      ref={canvas}
      aria-hidden="true"
      data-testid="seasonal-snow"
      style={{ color: SNOW }}
      className="pointer-events-none absolute inset-0 h-full w-full"
    />
  )
}
