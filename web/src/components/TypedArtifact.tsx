// The viewers for artifact types (ADR 0076): a deck, a canvas of boards, and a data page, drawn by Silt from the
// version's JSON. Slides and boards hold an agent's HTML, so each renders in its own sealed frame; a data page holds no
// HTML at all, and every number it shows names its source.

import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { type FrameTheme } from "../artifacts";
import {
  type Block,
  type Canvas,
  type DataPage,
  type Deck,
  type Source,
  chartTable,
  csvOf,
  layout,
  parsed,
  partDocument,
  shown,
  slideFromHash,
} from "../typed";
import { Button, Icon, IconButton, cx } from "../ui";
import { Markdown } from "./Markdown";

/** The scale that fits a `width`×`height` stage into the box `ref` measures. */
function useFit(width: number, height: number) {
  const box = useRef<HTMLDivElement>(null);
  const [scale, setScale] = useState(1);
  useEffect(() => {
    const element = box.current;
    if (!element) return;
    const measure = () =>
      setScale(
        Math.max(
          0.05,
          Math.min(element.clientWidth / width, element.clientHeight / height),
        ),
      );
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [width, height]);
  return { box, scale };
}

export function SlidesView({
  text,
  style,
  theme,
  title,
}: {
  text: string;
  style: string | null;
  theme: FrameTheme;
  title: string;
}) {
  const deck = useMemo(() => parsed<Deck>(text), [text]);
  const count = deck?.slides.length ?? 0;
  const [at, setAt] = useState(() =>
    slideFromHash(window.location.hash, count),
  );
  const [presenting, setPresenting] = useState(false);
  const [started, setStarted] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  const [width, height] = deck?.aspect === "4:3" ? [1024, 768] : [1280, 720];
  const { box, scale } = useFit(width, height);
  const room = useRef<HTMLDivElement>(null);
  const go = (to: number) => setAt(Math.min(Math.max(to, 0), count - 1));
  useEffect(() => {
    const keys = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
      if (["ArrowRight", "PageDown", " "].includes(event.key))
        setAt((was) => Math.min(was + 1, count - 1));
      else if (["ArrowLeft", "PageUp"].includes(event.key))
        setAt((was) => Math.max(was - 1, 0));
      else if (event.key === "Home") setAt(0);
      else if (event.key === "End") setAt(count - 1);
      else return;
      event.preventDefault();
    };
    window.addEventListener("keydown", keys);
    return () => window.removeEventListener("keydown", keys);
  }, [count]);
  useEffect(() => {
    if (!presenting) return;
    const tick = window.setInterval(() => setNow(Date.now()), 1000);
    const left = () => {
      if (!document.fullscreenElement) setPresenting(false);
    };
    document.addEventListener("fullscreenchange", left);
    return () => {
      window.clearInterval(tick);
      document.removeEventListener("fullscreenchange", left);
    };
  }, [presenting]);
  if (!deck || count === 0)
    return (
      <p className="px-5 py-6 text-sm text-danger">
        This deck's content isn't one Silt can draw.
      </p>
    );
  const slide = deck.slides[at];
  const present = async () => {
    setStarted(Date.now());
    setNow(Date.now());
    setPresenting(true);
    try {
      await room.current?.requestFullscreen();
    } catch {
      // Without full screen, the presenter view still shows in the panel.
    }
  };
  const elapsed = Math.floor((now - started) / 1000);
  return (
    <div
      ref={room}
      className="flex h-full min-h-0 flex-col bg-elevated"
      data-testid="slides"
    >
      <div className="flex min-h-0 flex-1">
        <div
          ref={box}
          className="relative min-w-0 flex-1 overflow-hidden"
          aria-label={`slide ${at + 1} of ${count}`}
        >
          <iframe
            key={at}
            title={`${title}, slide ${at + 1}: ${slide.title}`}
            sandbox="allow-scripts"
            srcDoc={partDocument(
              slide.html,
              deck.style,
              width,
              height,
              style,
              theme,
            )}
            className="absolute top-1/2 left-1/2 border-0 bg-white shadow-overlay"
            style={{
              width,
              height,
              transform: `translate(-50%, -50%) scale(${scale})`,
            }}
            data-testid="slide-frame"
          />
        </div>
        {presenting && (
          <aside
            className="flex w-72 shrink-0 flex-col gap-3 border-l border-line bg-bg p-4 text-sm"
            aria-label="presenter view"
            data-testid="presenter"
          >
            <p
              className="font-mono text-2xl tabular-nums"
              aria-label="time presenting"
            >
              {Math.floor(elapsed / 60)}:{String(elapsed % 60).padStart(2, "0")}
            </p>
            <div>
              <h3 className="font-mono text-2xs tracking-[0.08em] text-fg-2 uppercase">
                Notes
              </h3>
              <p className="mt-1 whitespace-pre-wrap text-fg-1">
                {slide.notes || "No notes for this slide."}
              </p>
            </div>
            <div>
              <h3 className="font-mono text-2xs tracking-[0.08em] text-fg-2 uppercase">
                Next
              </h3>
              <p className="mt-1 text-fg-1">
                {deck.slides[at + 1]?.title ?? "The end"}
              </p>
            </div>
          </aside>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-2 border-t border-line-subtle px-2 py-1.5">
        <IconButton
          icon="chevronLeft"
          label="Previous slide"
          disabled={at === 0}
          onClick={() => go(at - 1)}
        />
        <span className="font-mono text-xs text-fg-2 tabular-nums">
          {at + 1} / {count}
        </span>
        <IconButton
          icon="chevronRight"
          label="Next slide"
          disabled={at === count - 1}
          onClick={() => go(at + 1)}
        />
        <ol
          className="flex min-w-0 flex-1 gap-1 overflow-x-auto"
          aria-label="slides"
        >
          {deck.slides.map((each, index) => (
            <li key={index}>
              <button
                type="button"
                aria-current={index === at}
                onClick={() => go(index)}
                className={cx(
                  "max-w-40 truncate rounded-sm border px-2 py-1 text-xs",
                  index === at
                    ? "border-accent text-fg-1"
                    : "border-line text-fg-2 hover:border-line-strong",
                )}
              >
                {index + 1}. {each.title}
              </button>
            </li>
          ))}
        </ol>
        <Button
          size="sm"
          onClick={() =>
            void (presenting
              ? document.exitFullscreen?.().catch(() => setPresenting(false))
              : present())
          }
        >
          {presenting ? "Stop presenting" : "Present"}
        </Button>
      </div>
    </div>
  );
}

export function CanvasView({
  text,
  style,
  theme,
}: {
  text: string;
  style: string | null;
  theme: FrameTheme;
}) {
  const canvas = useMemo(() => parsed<Canvas>(text), [text]);
  const placed = useMemo(() => layout(canvas?.boards ?? []), [canvas]);
  const box = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ x: 40, y: 60, scale: 0.4 });
  const drag = useRef<{ x: number; y: number; from: typeof view } | null>(null);
  const extent = useMemo(
    () => ({
      width: Math.max(1, ...placed.map((board) => board.x + board.width)),
      height: Math.max(
        1,
        ...placed.map((board) => board.y + board.height + 40),
      ),
    }),
    [placed],
  );
  const fit = () => {
    const element = box.current;
    if (!element) return;
    const scale = Math.min(
      (element.clientWidth - 80) / extent.width,
      (element.clientHeight - 100) / extent.height,
      1,
    );
    setView({ x: 40, y: 60, scale: Math.max(scale, 0.05) });
  };
  // A new canvas opens with every board in view.
  useEffect(fit, [extent]);
  useEffect(() => {
    const element = box.current;
    if (!element) return;
    const wheel = (event: WheelEvent) => {
      event.preventDefault();
      const rect = element.getBoundingClientRect();
      const [px, py] = [event.clientX - rect.left, event.clientY - rect.top];
      setView((was) => {
        const scale = Math.min(
          Math.max(was.scale * Math.exp(-event.deltaY / 400), 0.05),
          4,
        );
        return {
          scale,
          x: px - ((px - was.x) * scale) / was.scale,
          y: py - ((py - was.y) * scale) / was.scale,
        };
      });
    };
    element.addEventListener("wheel", wheel, { passive: false });
    return () => element.removeEventListener("wheel", wheel);
  }, []);
  if (!canvas || placed.length === 0)
    return (
      <p className="px-5 py-6 text-sm text-danger">
        This canvas's content isn't one Silt can draw.
      </p>
    );
  const down = (event: ReactPointerEvent) => {
    if ((event.target as HTMLElement).closest("[data-board-label]")) return;
    drag.current = { x: event.clientX, y: event.clientY, from: view };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  };
  const move = (event: ReactPointerEvent) => {
    const from = drag.current;
    if (from)
      setView({
        ...from.from,
        x: from.from.x + event.clientX - from.x,
        y: from.from.y + event.clientY - from.y,
      });
  };
  const zoomTo = (board: (typeof placed)[number]) => {
    const element = box.current;
    if (!element) return;
    const scale = Math.min(
      (element.clientWidth - 40) / board.width,
      (element.clientHeight - 80) / board.height,
      1,
    );
    setView({ scale, x: 20 - board.x * scale, y: 50 - board.y * scale });
  };
  return (
    <div
      className="relative h-full min-h-0 overflow-hidden bg-surface-1"
      data-testid="canvas"
    >
      <div
        ref={box}
        className="absolute inset-0 cursor-grab touch-none active:cursor-grabbing"
        onPointerDown={down}
        onPointerMove={move}
        onPointerUp={() => (drag.current = null)}
        onPointerCancel={() => (drag.current = null)}
      >
        <div
          className="absolute top-0 left-0 origin-top-left"
          style={{
            transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})`,
          }}
        >
          {placed.map((board) => (
            <figure
              key={board.id}
              className="absolute m-0"
              style={{ left: board.x, top: board.y, width: board.width }}
              data-testid="board"
            >
              <figcaption
                data-board-label
                className="absolute bottom-full left-0 mb-2 flex items-baseline gap-2 overflow-hidden whitespace-nowrap"
                style={{
                  fontSize: Math.min(14 / view.scale, board.width / 10),
                  maxWidth: board.width,
                }}
              >
                <button
                  type="button"
                  onClick={() => zoomTo(board)}
                  className="font-medium text-fg-1 hover:underline"
                >
                  {board.name}
                </button>
                <span className="truncate text-fg-2">
                  {board.width}×{board.height}
                  {board.caption ? ` · ${board.caption}` : ""}
                </span>
              </figcaption>
              <iframe
                title={board.name}
                sandbox="allow-scripts"
                srcDoc={partDocument(
                  board.html,
                  canvas.style,
                  board.width,
                  board.height,
                  style,
                  theme,
                )}
                className="pointer-events-auto block border-0 bg-white shadow-overlay"
                style={{ width: board.width, height: board.height }}
              />
            </figure>
          ))}
        </div>
      </div>
      <div className="absolute right-2 bottom-2 flex items-center gap-1 rounded-md border border-line bg-elevated p-1 shadow-overlay">
        <IconButton
          icon="plus"
          label="Zoom in"
          onClick={() =>
            setView((was) => ({ ...was, scale: Math.min(was.scale * 1.25, 4) }))
          }
        />
        <span className="w-12 text-center font-mono text-xs text-fg-2 tabular-nums">
          {Math.round(view.scale * 100)}%
        </span>
        <IconButton
          icon="x"
          label="Zoom out"
          onClick={() =>
            setView((was) => ({
              ...was,
              scale: Math.max(was.scale / 1.25, 0.05),
            }))
          }
        />
        <Button size="sm" variant="ghost" onClick={fit}>
          Fit all
        </Button>
      </div>
    </div>
  );
}

const CHART = {
  light: ["#009183", "#a8432b", "#855193", "#7a8a2b", "#15669d", "#c2702b"],
  dark: ["#06a596", "#d36f4f", "#b074c4", "#8b9837", "#5194d2", "#bf7f3b"],
};

function saveCsv(name: string, columns: string[], rows: unknown[][]) {
  const url = URL.createObjectURL(
    new Blob([csvOf(columns, rows)], { type: "text/csv" }),
  );
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = `${name.replace(/[^A-Za-z0-9_-]+/g, "-").replace(/^-|-$/g, "") || "data"}.csv`;
  anchor.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function SourceLink({ id, sources }: { id: string; sources: Source[] }) {
  const source = sources.find((candidate) => candidate.id === id);
  return (
    <a
      href={`#source-${id}`}
      className="font-mono text-2xs text-fg-3 hover:text-fg-1"
      title={source ? `${source.detail} · ${source.at}` : id}
    >
      source: {source?.name ?? id}
    </a>
  );
}

function Chart({
  block,
  theme,
}: {
  block: Extract<Block, { type: "chart" }>;
  theme: FrameTheme;
}) {
  const colors = CHART[theme === "dark" ? "dark" : "light"];
  const [hover, setHover] = useState<number | null>(null);
  const { columns, rows } = chartTable(block.series);
  // Drawn at the width it's shown, so its labels stay the size of the page's text.
  const holder = useRef<HTMLDivElement>(null);
  const [shownWidth, setShownWidth] = useState(640);
  useEffect(() => {
    const element = holder.current;
    if (!element) return;
    const measure = () =>
      setShownWidth(Math.max(280, Math.round(element.clientWidth)));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const [W, H, L, B, T, R] = [shownWidth, 260, 48, 28, 12, 12];
  const values = block.series
    .flatMap((one) => one.points.map(([, y]) => y))
    .filter((y): y is number => typeof y === "number");
  const stackedTotals = rows.map((row) =>
    row
      .slice(1)
      .reduce<number>((sum, y) => sum + (typeof y === "number" ? y : 0), 0),
  );
  const top = Math.max(0, ...(block.stacked ? stackedTotals : values));
  const bottom = Math.min(0, ...values);
  const span = top - bottom || 1;
  const sx = (index: number) =>
    L + (W - L - R) * (rows.length === 1 ? 0.5 : index / (rows.length - 1));
  const sy = (value: number) => T + (H - T - B) * (1 - (value - bottom) / span);
  const ticks = [bottom, bottom + span / 2, top];
  const band = (W - L - R) / Math.max(rows.length, 1);
  const marks: React.ReactNode[] = [];
  if (block.chart === "bar" || block.chart === "segmented") {
    rows.forEach((row, index) => {
      let base = 0;
      const ys = row.slice(1);
      const each =
        block.stacked || block.chart === "segmented"
          ? band * 0.7
          : (band * 0.7) / ys.length;
      ys.forEach((y, s) => {
        if (typeof y !== "number") return;
        const x =
          L +
          band * index +
          band * 0.15 +
          (block.stacked || block.chart === "segmented" ? 0 : each * s);
        const from = block.stacked || block.chart === "segmented" ? base : 0;
        base += y;
        const [y0, y1] = [sy(from), sy(from + y)];
        marks.push(
          <rect
            key={`${index}-${s}`}
            x={x + 1}
            y={Math.min(y0, y1)}
            width={Math.max(each - 2, 1)}
            height={Math.max(Math.abs(y0 - y1) - 2, 1)}
            rx={2}
            fill={colors[s % colors.length]}
          />,
        );
      });
    });
  } else if (block.chart === "heatmap") {
    const cell = (H - T - B) / block.series.length;
    block.series.forEach((_one, s) =>
      rows.forEach((row, index) => {
        const y = row[s + 1];
        if (typeof y !== "number") return;
        marks.push(
          <rect
            key={`${s}-${index}`}
            x={L + band * index + 1}
            y={T + cell * s + 1}
            width={band - 2}
            height={cell - 2}
            fill={colors[0]}
            opacity={0.15 + (0.85 * (y - bottom)) / span}
          />,
        );
      }),
    );
  } else {
    block.series.forEach((_one, s) => {
      const color = colors[s % colors.length];
      const points = rows
        .map((row, index) => [index, row[s + 1]] as const)
        .filter(
          (point): point is readonly [number, number] =>
            typeof point[1] === "number",
        );
      const path = points
        .map(([index, y], at) => `${at === 0 ? "M" : "L"}${sx(index)},${sy(y)}`)
        .join("");
      if (block.chart === "line" || block.chart === "area")
        marks.push(
          <path
            key={`l${s}`}
            d={path}
            fill="none"
            stroke={color}
            strokeWidth={2}
          />,
        );
      if (block.chart === "area" && points.length > 1)
        marks.push(
          <path
            key={`a${s}`}
            d={`${path}L${sx(points[points.length - 1][0])},${sy(Math.max(bottom, 0))}L${sx(points[0][0])},${sy(Math.max(bottom, 0))}Z`}
            fill={color}
            opacity={0.15}
          />,
        );
      if (block.chart === "dot" || block.chart === "scatter")
        for (const [index, y] of points)
          marks.push(
            <circle
              key={`d${s}-${index}`}
              cx={sx(index)}
              cy={sy(y)}
              r={4}
              fill={color}
              stroke="var(--color-elevated, #fff)"
              strokeWidth={2}
            />,
          );
    });
  }
  const everyNth = Math.max(
    1,
    Math.ceil(rows.length / Math.max(2, Math.floor((W - L - R) / 56))),
  );
  return (
    <div ref={holder}>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        width={W}
        height={H}
        role="img"
        aria-label={`${block.title ?? block.chart}: ${block.series.map((one) => one.name).join(", ")}`}
        className="w-full text-fg-2"
        onMouseLeave={() => setHover(null)}
      >
        {ticks.map((tick) => (
          <g key={tick}>
            <line
              x1={L}
              x2={W - R}
              y1={sy(tick)}
              y2={sy(tick)}
              stroke="currentColor"
              strokeOpacity={0.15}
            />
            <text
              x={L - 6}
              y={sy(tick) + 4}
              textAnchor="end"
              fontSize={11}
              fill="currentColor"
            >
              {shown(Number(tick.toPrecision(3)))}
            </text>
          </g>
        ))}
        {marks}
        {rows.map((_row, index) => (
          <rect
            key={`h${index}`}
            x={L + band * index}
            y={T}
            width={band}
            height={H - T - B}
            fill="transparent"
            onMouseEnter={() => setHover(index)}
          />
        ))}
        {rows.map((row, index) =>
          index % everyNth !== 0 ? null : (
            <text
              key={`x${index}`}
              x={
                block.chart === "line" ||
                block.chart === "area" ||
                block.chart === "dot" ||
                block.chart === "scatter"
                  ? sx(index)
                  : L + band * (index + 0.5)
              }
              y={H - 8}
              textAnchor="middle"
              fontSize={11}
              fill="currentColor"
            >
              {String(row[0])}
            </text>
          ),
        )}
        {hover !== null && (
          <g pointerEvents="none">
            <rect
              x={Math.min(L + band * hover, W - 170)}
              y={T}
              width={160}
              height={18 + 16 * block.series.length}
              rx={4}
              fill="var(--color-elevated, #fff)"
              stroke="currentColor"
              strokeOpacity={0.3}
            />
            <text
              x={Math.min(L + band * hover, W - 170) + 8}
              y={T + 14}
              fontSize={11}
              fill="currentColor"
            >
              {String(rows[hover][0])}
            </text>
            {block.series.map((one, s) => (
              <text
                key={s}
                x={Math.min(L + band * hover, W - 170) + 8}
                y={T + 30 + 16 * s}
                fontSize={11}
                fill={colors[s % colors.length]}
              >
                {one.name}: {shown(rows[hover][s + 1])}
              </text>
            ))}
          </g>
        )}
        <desc>{columns.join(", ")}</desc>
      </svg>
    </div>
  );
}

export function DataView({ text, theme }: { text: string; theme: FrameTheme }) {
  const page = useMemo(() => parsed<DataPage>(text), [text]);
  const [tables, setTables] = useState<Set<number>>(new Set());
  if (!page)
    return (
      <p className="px-5 py-6 text-sm text-danger">
        This data page's content isn't one Silt can draw.
      </p>
    );
  const colors = CHART[theme === "dark" ? "dark" : "light"];
  const toggle = (index: number) =>
    setTables((was) => {
      const next = new Set(was);
      if (!next.delete(index)) next.add(index);
      return next;
    });
  const stats = page.blocks.filter((block) => block.type === "stat");
  return (
    <div className="h-full overflow-auto" data-testid="data-page">
      <article className="mx-auto flex max-w-4xl flex-col gap-5 px-6 pt-6 pb-10">
        <header>
          <h1 className="text-xl font-semibold text-balance text-fg-1">
            {page.title}
          </h1>
          {page.summary && (
            <p className="mt-1 text-sm text-fg-2">{page.summary}</p>
          )}
        </header>
        {stats.length > 0 && (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(10rem,1fr))] gap-3">
            {stats.map((block, index) => (
              <div
                key={index}
                className="flex flex-col gap-1 rounded-md border border-line bg-elevated p-3"
                data-testid="stat"
              >
                <span className="text-xs text-fg-2">{block.label}</span>
                <span className="text-2xl font-semibold text-fg-1 tabular-nums">
                  {shown(block.value)}
                  {block.unit && (
                    <span className="ml-1 text-sm font-normal text-fg-2">
                      {block.unit}
                    </span>
                  )}
                </span>
                {typeof block.change === "number" && (
                  <span className="text-xs text-fg-2">
                    {block.change > 0 ? "▲" : block.change < 0 ? "▼" : "="}{" "}
                    {shown(Math.abs(block.change * 100))}%
                  </span>
                )}
                <SourceLink id={block.source} sources={page.sources} />
              </div>
            ))}
          </div>
        )}
        {page.blocks.map((block, index) => {
          if (block.type === "stat") return null;
          if (block.type === "text")
            return (
              <div key={index} className="text-sm text-fg-1">
                <Markdown text={block.text} />
              </div>
            );
          const table =
            block.type === "table"
              ? { columns: block.columns, rows: block.rows }
              : chartTable(block.series);
          const asTable = block.type === "table" || tables.has(index);
          return (
            <section
              key={index}
              className="flex flex-col gap-2 rounded-md border border-line bg-elevated p-3"
              aria-label={block.title ?? `block ${index + 1}`}
            >
              <div className="flex items-center gap-2">
                <h2 className="min-w-0 flex-1 truncate text-sm font-medium text-fg-1">
                  {block.title ??
                    (block.type === "chart" ? block.chart : "Table")}
                </h2>
                {block.type === "chart" && (
                  <Button
                    size="sm"
                    variant="ghost"
                    aria-pressed={asTable}
                    onClick={() => toggle(index)}
                  >
                    {asTable ? "Chart" : "Table"}
                  </Button>
                )}
                <IconButton
                  icon="download"
                  label="Download as CSV"
                  onClick={() =>
                    saveCsv(
                      block.title ?? page.title,
                      table.columns,
                      table.rows,
                    )
                  }
                />
              </div>
              {block.type === "chart" && !asTable && (
                <>
                  <Chart block={block} theme={theme} />
                  {block.series.length > 1 && (
                    <ul
                      className="flex flex-wrap gap-3 text-xs text-fg-2"
                      aria-label="legend"
                    >
                      {block.series.map((one, s) => (
                        <li
                          key={one.name}
                          className="inline-flex items-center gap-1.5"
                        >
                          <span
                            aria-hidden="true"
                            className="h-2 w-2 rounded-full"
                            style={{ background: colors[s % colors.length] }}
                          />
                          {one.name}
                        </li>
                      ))}
                    </ul>
                  )}
                </>
              )}
              {asTable && (
                <div className="overflow-x-auto">
                  <table className="w-full text-left text-xs">
                    <thead>
                      <tr>
                        {table.columns.map((column) => (
                          <th
                            key={column}
                            className="border-b border-line px-2 py-1 font-medium text-fg-2"
                          >
                            {column}
                          </th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {table.rows.map((row, at) => (
                        <tr key={at}>
                          {row.map((cell, c) => (
                            <td
                              key={c}
                              className={cx(
                                "border-b border-line-subtle px-2 py-1 text-fg-1",
                                typeof cell === "number" &&
                                  "text-right tabular-nums",
                              )}
                            >
                              {shown(cell)}
                            </td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
              <SourceLink id={block.source} sources={page.sources} />
            </section>
          );
        })}
        {page.sources.length > 0 && (
          <footer className="border-t border-line pt-3">
            <h2 className="font-mono text-2xs tracking-[0.08em] text-fg-2 uppercase">
              Sources
            </h2>
            <ul className="mt-2 flex flex-col gap-2 text-xs">
              {page.sources.map((source) => (
                <li
                  key={source.id}
                  id={`source-${source.id}`}
                  className="flex flex-col"
                >
                  <span className="text-fg-1">
                    <Icon name="info" className="mr-1 inline h-3 w-3" />
                    {source.name}{" "}
                    <span className="text-fg-3">· {source.kind}</span>
                  </span>
                  <code className="break-all text-fg-2">{source.detail}</code>
                  <span className="text-fg-3">{source.at}</span>
                </li>
              ))}
            </ul>
          </footer>
        )}
      </article>
    </div>
  );
}
