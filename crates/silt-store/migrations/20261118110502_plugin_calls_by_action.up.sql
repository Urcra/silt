-- WebAssembly plugins (ADR 0100): a call is `wasm` when its request is the
-- call as its plugin prepared it, with its card and effects, rather than a
-- canonical request of `plugins.json`'s kinds.
ALTER TABLE plugin_calls ADD COLUMN kind TEXT NOT NULL DEFAULT 'legacy';
-- A WebAssembly plugin's grant covers one of its actions; none covers every
-- read or every change, as `plugins.json`'s did.
ALTER TABLE plugin_grants ADD COLUMN action TEXT;
