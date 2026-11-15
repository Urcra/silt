import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { api, type FoundModel, type LocalModel, type ModelFit } from '../api'
import { keys, message } from '../query'
import { SETTINGS_HEADING } from '../settingsContext'
import { Button, EmptyState, Input, cx, toast } from '../ui'

const FIT: Record<ModelFit, { label: string; tone: string }> = {
  fits: { label: 'Fits', tone: 'text-ok' },
  tight: { label: 'Tight', tone: 'text-warn' },
  too_big: { label: 'Too big here', tone: 'text-danger' },
}

/** What a download of llama.cpp itself is called, as `silt_daemon::models::RUNTIME` names it. */
const RUNTIME = 'llama.cpp'

function gigabytes(bytes: number): string {
  return bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`
}

function Fit({ fit }: { fit: ModelFit }) {
  return <span className={cx('text-2xs font-medium', FIT[fit].tone)}>{FIT[fit].label}</span>
}

function Installed({ model, onRemove }: { model: LocalModel; onRemove: () => void }) {
  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-line-subtle px-4 py-3 text-sm last:border-b-0">
      <span className="min-w-0 flex-1">
        <span className="font-medium">{model.name}</span>
        <span className="ml-2 font-mono text-2xs text-fg-2">{model.id}</span>
        <span className="block text-xs text-fg-2">
          From {model.source} · {gigabytes(model.size)} · {model.context.toLocaleString()} tokens of context
        </span>
      </span>
      <Fit fit={model.fit} />
      <Button variant="ghost" size="sm" onClick={onRemove}>
        Remove
      </Button>
    </li>
  )
}

/** A repository a search found, opening to its quantizations. */
function Found({ found, busy, onPull }: { found: FoundModel; busy: boolean; onPull: (source: string) => void }) {
  const [open, setOpen] = useState(false)
  const files = useQuery({
    queryKey: keys.modelFiles(found.repo),
    queryFn: () => api.modelFiles(found.repo),
    enabled: open,
  })
  return (
    <li className="border-b border-line-subtle text-sm last:border-b-0">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex w-full items-center gap-3 px-4 py-2.5 text-left hover:bg-surface-2 max-md:min-h-11"
      >
        <span className="min-w-0 flex-1 truncate font-mono text-xs">{found.repo}</span>
        <span className="text-2xs text-fg-2">{found.downloads.toLocaleString()} downloads</span>
      </button>
      {open && (
        <div className="px-4 pb-3">
          {files.isPending && <p className="text-xs text-fg-2">Asking Hugging Face…</p>}
          {files.error && <p className="text-xs text-danger">{message(files.error)}</p>}
          {files.data && (
            <>
              <p className="pb-1 text-xs text-fg-2">
                {[files.data.architecture, files.data.context_length && `trained for ${files.data.context_length.toLocaleString()} tokens of context`]
                  .filter(Boolean)
                  .join(' · ')}
              </p>
              {files.data.quantizations.length === 0 && (
                <p className="text-xs text-fg-2">No GGUF file here that Silt can check: each needs the SHA-256 Hugging Face keeps for it.</p>
              )}
              <ul aria-label={`quantizations of ${found.repo}`} className="flex flex-col">
                {files.data.quantizations.map((quantization) => (
                  <li key={quantization.source} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-1.5">
                    <span className="min-w-28 font-mono text-xs">{quantization.label}</span>
                    <span className="flex-1 text-xs text-fg-2">
                      {gigabytes(quantization.size)}
                      {quantization.parts > 1 && ` in ${quantization.parts} parts`} · {quantization.context.toLocaleString()} tokens of context here
                    </span>
                    <Fit fit={quantization.fit} />
                    <Button size="sm" disabled={busy} onClick={() => onPull(quantization.source)}>
                      Download
                    </Button>
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}
    </li>
  )
}

/** The models this machine runs itself, and finding others on Hugging Face (ADR 0066). */
export function LocalModelsSettings() {
  const queries = useQueryClient()
  const view = useQuery({
    queryKey: keys.models,
    queryFn: api.models,
    refetchInterval: (query) => (query.state.data?.pull ? 1000 : false),
  })
  const [words, setWords] = useState('')
  const [asked, setAsked] = useState('')
  const found = useQuery({
    queryKey: keys.modelSearch(asked),
    queryFn: () => api.searchModels(asked),
  })
  const [source, setSource] = useState('')
  const [error, setError] = useState<string>()
  const refresh = () => void queries.invalidateQueries({ queryKey: keys.models })
  const pull = (wanted: string) => {
    setError(undefined)
    api.pullModel(wanted).then(refresh, (failure: unknown) => setError(message(failure)))
  }
  const fetchRuntime = () => {
    setError(undefined)
    api.pullRuntime().then(refresh, (failure: unknown) => setError(message(failure)))
  }
  const remove = (id: string) => {
    setError(undefined)
    api.removeModel(id).then(
      (said) => {
        toast(said, 'ok')
        refresh()
      },
      (failure: unknown) => setError(message(failure)),
    )
  }
  const current = view.data
  const pulling = current?.pull
  return (
    <section aria-label="Local models" className="flex flex-col gap-3">
      <h2 className={SETTINGS_HEADING}>Local models</h2>
      <p className="text-sm text-fg-2">
        Models Silt runs on this machine with llama.cpp, for Silt's own agent and for Claude Code. What you send them stays here. A model is downloaded only
        when you ask for it.
      </p>
      {current && (
        <p className="text-xs text-fg-2">
          {current.server ? (
            <>
              llama-server: <code className="font-mono">{current.server}</code>
            </>
          ) : (
            <span className="inline-flex flex-wrap items-center gap-2">
              No llama-server found.
              <Button size="sm" disabled={Boolean(pulling)} onClick={fetchRuntime}>
                Download llama.cpp
              </Button>
              or put one on the PATH, such as a CUDA build for an NVIDIA GPU.
            </span>
          )}
          {' · '}
          {current.devices.length > 0 ? current.devices.map((device) => `${device.description} (${gigabytes(device.total)})`).join(', ') : 'no GPU found'}
          {' · '}room for models: {gigabytes(current.room)}
        </p>
      )}
      {current && current.models.length === 0 && <EmptyState title="No models here yet">Find one below, or add a GGUF file you have.</EmptyState>}
      {current && current.models.length > 0 && (
        <ul aria-label="models here" className="overflow-hidden rounded-md border border-line bg-elevated">
          {current.models.map((model) => (
            <Installed key={model.id} model={model} onRemove={() => remove(model.id)} />
          ))}
        </ul>
      )}
      {pulling && (
        <p role="status" className="text-xs text-fg-2">
          Downloading {pulling.id === RUNTIME ? 'llama.cpp' : pulling.id}…{' '}
          {pulling.total > 0 ? `${gigabytes(pulling.done)} of ${gigabytes(pulling.total)}` : ''}
        </p>
      )}
      {current?.failed && (
        <p className="text-xs text-danger">
          {current.failed.id} didn't download: {current.failed.reason}
        </p>
      )}
      <h3 className="text-sm font-medium">Find a model</h3>
      <form
        className="flex gap-1.5"
        onSubmit={(event) => {
          event.preventDefault()
          setAsked(words.trim())
        }}
      >
        <Input
          aria-label="search Hugging Face"
          placeholder="qwen coder, gpt-oss, abliterated…"
          value={words}
          onChange={(event) => setWords(event.target.value)}
          className="min-w-0 flex-1 text-xs"
        />
        <Button type="submit">Search</Button>
      </form>
      <p className="text-xs text-fg-2">
        {asked ? `GGUF models on Hugging Face matching “${asked}”` : 'The most downloaded GGUF models on Hugging Face'}, the most downloaded first. Open one to
        see its quantizations and which fit here.
      </p>
      {found.isPending && <p className="text-xs text-fg-2">Asking Hugging Face…</p>}
      {found.error && <p className="text-xs text-danger">{message(found.error)}</p>}
      {found.data && found.data.length === 0 && <p className="text-xs text-fg-2">Nothing found.</p>}
      {found.data && found.data.length > 0 && (
        <ul aria-label="found on Hugging Face" className="overflow-hidden rounded-md border border-line bg-elevated">
          {found.data.map((model) => (
            <Found key={model.repo} found={model} busy={Boolean(pulling)} onPull={pull} />
          ))}
        </ul>
      )}
      <form
        className="flex flex-col gap-1.5 text-sm"
        onSubmit={(event) => {
          event.preventDefault()
          if (source.trim()) pull(source.trim())
        }}
      >
        <span className="font-medium">Another model</span>
        <span className="text-xs text-fg-2">
          A GGUF on Hugging Face as <code className="font-mono">owner/repo:file.gguf</code>, or the full path of a GGUF file on this machine, which stays where
          it is.
        </span>
        <span className="flex gap-1.5">
          <Input
            aria-label="model to add"
            placeholder="unsloth/Qwen3.6-27B-GGUF:Qwen3.6-27B-Q4_K_M.gguf"
            value={source}
            onChange={(event) => setSource(event.target.value)}
            className="min-w-0 flex-1 font-mono text-xs"
          />
          <Button type="submit" disabled={Boolean(pulling)}>
            Add
          </Button>
        </span>
      </form>
      {error && <p className="text-xs text-danger">{error}</p>}
    </section>
  )
}
