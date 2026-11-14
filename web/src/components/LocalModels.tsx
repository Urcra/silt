import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type ReactNode, useState } from "react";
import { api, type CatalogModel, type LocalModel, type ModelFit } from "../api";
import { keys, message } from "../query";
import { SETTINGS_HEADING } from "../settingsContext";
import { Button, EmptyState, Input, cx, toast } from "../ui";

const FIT: Record<ModelFit, { label: string; tone: string }> = {
  fits: { label: "Fits", tone: "text-ok" },
  tight: { label: "Tight", tone: "text-warn" },
  too_big: { label: "Too big here", tone: "text-danger" },
};

function gigabytes(bytes: number): string {
  return bytes >= 1e9
    ? `${(bytes / 1e9).toFixed(1)} GB`
    : `${Math.round(bytes / 1e6)} MB`;
}

function Fit({ fit }: { fit: ModelFit }) {
  return (
    <span className={cx("text-2xs font-medium", FIT[fit].tone)}>
      {FIT[fit].label}
    </span>
  );
}

function Row({
  model,
  action,
}: {
  model: LocalModel | CatalogModel;
  action: ReactNode;
}) {
  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-line-subtle px-4 py-3 text-sm last:border-b-0">
      <span className="min-w-0 flex-1">
        <span className="font-medium">{model.name}</span>
        <span className="ml-2 font-mono text-2xs text-fg-2">{model.id}</span>
        <span className="block text-xs text-fg-2">
          {"source" in model ? `From ${model.source}` : model.about}
          {" · "}
          {gigabytes(model.size)} · {model.context.toLocaleString()} tokens of
          context
          {"tested" in model && !model.tested && " · not tested yet"}
        </span>
      </span>
      <Fit fit={model.fit} />
      {action}
    </li>
  );
}

/** The models this machine runs itself, and the catalog's others (ADR 0066). */
export function LocalModelsSettings() {
  const queries = useQueryClient();
  const view = useQuery({
    queryKey: keys.models,
    queryFn: api.models,
    refetchInterval: (query) => (query.state.data?.pull ? 1000 : false),
  });
  const [source, setSource] = useState("");
  const [error, setError] = useState<string>();
  const refresh = () =>
    void queries.invalidateQueries({ queryKey: keys.models });
  const pull = (wanted: string) => {
    setError(undefined);
    api
      .pullModel(wanted)
      .then(refresh, (failure: unknown) => setError(message(failure)));
  };
  const remove = (id: string) => {
    setError(undefined);
    api.removeModel(id).then(
      (said) => {
        toast(said, "ok");
        refresh();
      },
      (failure: unknown) => setError(message(failure)),
    );
  };
  const current = view.data;
  const pulling = current?.pull;
  return (
    <section aria-label="Local models" className="flex flex-col gap-3">
      <h2 className={SETTINGS_HEADING}>Local models</h2>
      <p className="text-sm text-fg-2">
        Models Silt runs on this machine with llama.cpp, for Silt's own agent
        and for Claude Code. What you send them stays here. A model is
        downloaded only when you ask for it.
      </p>
      {current && (
        <p className="text-xs text-fg-2">
          {current.server ? (
            <>
              llama-server: <code className="font-mono">{current.server}</code>
            </>
          ) : (
            <>
              No llama-server found. Install llama.cpp, or point{" "}
              <code className="font-mono">SILT_LLAMA_SERVER</code> at one.
            </>
          )}
          {" · "}
          {current.devices.length > 0
            ? current.devices
                .map(
                  (device) =>
                    `${device.description} (${gigabytes(device.total)})`,
                )
                .join(", ")
            : "no GPU found"}
          {" · "}room for models: {gigabytes(current.room)}
        </p>
      )}
      {current && current.models.length === 0 && (
        <EmptyState title="No models here yet">
          Download one below, or add a GGUF file you have.
        </EmptyState>
      )}
      {current && current.models.length > 0 && (
        <ul
          aria-label="models here"
          className="overflow-hidden rounded-md border border-line bg-elevated"
        >
          {current.models.map((model) => (
            <Row
              key={model.id}
              model={model}
              action={
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => remove(model.id)}
                >
                  Remove
                </Button>
              }
            />
          ))}
        </ul>
      )}
      {pulling && (
        <p role="status" className="text-xs text-fg-2">
          Downloading {pulling.id}…{" "}
          {pulling.total > 0
            ? `${gigabytes(pulling.done)} of ${gigabytes(pulling.total)}`
            : ""}
        </p>
      )}
      {current?.failed && (
        <p className="text-xs text-danger">
          {current.failed.id} didn't download: {current.failed.reason}
        </p>
      )}
      {current && current.catalog.length > 0 && (
        <>
          <h3 className="text-sm font-medium">Catalog</h3>
          <ul
            aria-label="catalog"
            className="overflow-hidden rounded-md border border-line bg-elevated"
          >
            {current.catalog.map((model) => (
              <Row
                key={model.id}
                model={model}
                action={
                  <Button
                    size="sm"
                    disabled={Boolean(pulling)}
                    onClick={() => pull(model.id)}
                  >
                    Download
                  </Button>
                }
              />
            ))}
          </ul>
        </>
      )}
      <form
        className="flex flex-col gap-1.5 text-sm"
        onSubmit={(event) => {
          event.preventDefault();
          if (source.trim()) pull(source.trim());
        }}
      >
        <span className="font-medium">Another model</span>
        <span className="text-xs text-fg-2">
          A GGUF on Hugging Face as{" "}
          <code className="font-mono">owner/repo:file.gguf</code>, or the full
          path of a GGUF file on this machine, which stays where it is.
        </span>
        <span className="flex gap-1.5">
          <Input
            aria-label="model to add"
            placeholder="unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Qwen3-Coder-30B-A3B-Instruct-Q4_K_M.gguf"
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
  );
}
