# Silt as the Agent SDK's session store

Adapters that make a Silt server the [session store](https://code.claude.com/docs/en/agent-sdk/session-storage) of an app built on the Claude Agent SDK (ADR 0042). Copy the one for your language into your app:

- `silt-session-store.ts`: TypeScript, with `fetch` only;
- `silt_session_store.py`: Python, with the standard library only.

Each takes the server's URL and a device token, and optionally the project (`github.com/acme/app`) the sessions belong to. `SILT_PROTOCOL` in each is the server's protocol version: update it when you update the server.

The Python adapter passes the SDK's conformance suite against a running server:

```python
import pytest
from claude_agent_sdk.testing import run_session_store_conformance
from silt_session_store import SiltSessionStore

@pytest.mark.anyio
async def test_conformance():
    # Each store needs empty storage: a token of a person with no sessions yet.
    tokens = iter(TOKENS)
    await run_session_store_conformance(lambda: SiltSessionStore("http://127.0.0.1:7450", next(tokens)))
```

See docs/guide/git-and-forges.md for what the sessions become in Silt.
