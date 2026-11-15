#!/usr/bin/env node
// Checks a chart palette against a surface: separation under normal vision
// and the three common colour-vision deficiencies, contrast, and a lightness
// band. Usage:
//   node validate-palette.mjs "#c25a3a,#5b7a8a,#7a8a3f" --surface "#f6f1e8" [--pairs all] [--json]
// Exits 1 when any check fails. Also importable: `validate(colors, surface, {pairs})`.

// Separation is OKLab distance x100; contrast is WCAG 2's ratio.
const SEPARATION_PASS = 8
const SEPARATION_FLOOR = 6
const NORMAL_FLOOR = 15
const CONTRAST_FLOOR = 3
const LIGHTNESS_SPREAD = 0.3

// Machado, Oliveira and Fernandes (2009), severity 1, applied in linear sRGB.
const DEFICIENCIES = {
  protan: [
    [0.152286, 1.052583, -0.204868],
    [0.114503, 0.786281, 0.099216],
    [-0.003882, -0.048116, 1.051998],
  ],
  deutan: [
    [0.367322, 0.860646, -0.227968],
    [0.280085, 0.672501, 0.047413],
    [-0.01182, 0.04294, 0.968881],
  ],
  tritan: [
    [1.255528, -0.076749, -0.178779],
    [-0.078411, 0.930809, 0.147602],
    [0.004733, 0.691367, 0.3039],
  ],
}

export function parseHex(hex) {
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim())
  if (!m) throw new Error(`${hex} isn't a #rgb or #rrggbb colour`)
  const digits = m[1].length === 3 ? [...m[1]].map((d) => d + d).join('') : m[1]
  return [0, 2, 4].map((i) => parseInt(digits.slice(i, i + 2), 16) / 255)
}

const toLinear = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)

function oklab([r, g, b]) {
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b)
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b)
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b)
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ]
}

function simulate(linear, matrix) {
  return matrix.map((row) => Math.min(1, Math.max(0, row[0] * linear[0] + row[1] * linear[1] + row[2] * linear[2])))
}

const distance = (a, b) => 100 * Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2])

function luminance(linear) {
  return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]
}

function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x)
  return (hi + 0.05) / (lo + 0.05)
}

/** The checks, as rows of `{check, subject, value, verdict}` with verdict PASS, WARN or FAIL. */
export function validate(colors, surface, { pairs = 'adjacent' } = {}) {
  const linear = colors.map((hex) => parseHex(hex).map(toLinear))
  const ground = parseHex(surface).map(toLinear)
  const rows = []

  const indexPairs = []
  for (let i = 0; i < colors.length; i++) {
    for (let j = i + 1; j < colors.length; j++) {
      if (pairs === 'all' || j === i + 1) indexPairs.push([i, j])
    }
  }
  for (const [i, j] of indexPairs) {
    const subject = `${colors[i]} / ${colors[j]}`
    const normal = distance(oklab(linear[i]), oklab(linear[j]))
    rows.push({ check: 'normal vision', subject, value: normal.toFixed(1), verdict: normal >= NORMAL_FLOOR ? 'PASS' : 'FAIL' })
    for (const [name, matrix] of Object.entries(DEFICIENCIES)) {
      const d = distance(oklab(simulate(linear[i], matrix)), oklab(simulate(linear[j], matrix)))
      const verdict = d >= SEPARATION_PASS ? 'PASS' : d >= SEPARATION_FLOOR ? 'WARN' : 'FAIL'
      rows.push({ check: name, subject, value: d.toFixed(1), verdict })
    }
  }

  colors.forEach((hex, i) => {
    const ratio = contrast(linear[i], ground)
    rows.push({ check: 'contrast', subject: `${hex} on ${surface}`, value: `${ratio.toFixed(2)}:1`, verdict: ratio >= CONTRAST_FLOOR ? 'PASS' : 'WARN' })
  })

  const lightness = linear.map((c) => oklab(c)[0])
  const spread = Math.max(...lightness) - Math.min(...lightness)
  rows.push({ check: 'lightness band', subject: 'all', value: spread.toFixed(2), verdict: spread <= LIGHTNESS_SPREAD ? 'PASS' : 'WARN' })
  return rows
}

function main(argv) {
  const args = argv.slice(2)
  const option = (name) => {
    const at = args.indexOf(name)
    return at >= 0 ? args.splice(at, 2)[1] : undefined
  }
  const surface = option('--surface')
  const pairs = option('--pairs') ?? 'adjacent'
  const json = args.includes('--json')
  const list = args.filter((arg) => arg !== '--json')[0]
  if (!list || !surface) {
    console.error('usage: node validate-palette.mjs "#hex,#hex,..." --surface "#hex" [--pairs all] [--json]')
    return 2
  }
  const rows = validate(list.split(',').map((c) => c.trim()).filter(Boolean), surface, { pairs })
  if (json) console.log(JSON.stringify(rows, null, 2))
  else {
    const widths = ['check', 'subject', 'value'].map((key) => Math.max(key.length, ...rows.map((row) => row[key].length)))
    for (const row of rows) {
      console.log(`${row.verdict.padEnd(5)} ${row.check.padEnd(widths[0])}  ${row.subject.padEnd(widths[1])}  ${row.value.padStart(widths[2])}`)
    }
  }
  return rows.some((row) => row.verdict === 'FAIL') ? 1 : 0
}

if (typeof process !== 'undefined' && import.meta.url === `file://${process.argv[1]}`) {
  process.exit(main(process.argv))
}
