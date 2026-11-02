"""A Silt server as the Agent SDK's session store (ADR 0042).

Copy this file into your app; it needs only the standard library.

    store = SiltSessionStore(server="https://silt.example", token=os.environ["SILT_TOKEN"])
    query(prompt=prompt, options=ClaudeAgentOptions(session_store=store))

The token is a device token: `silt auth login` on the app's host writes one
to `servers.json`, or make one with `silt server pair`.
"""

import asyncio
import json
import urllib.error
import urllib.parse
import urllib.request
from typing import Any

# The server's `PROTOCOL`: update it with the server.
SILT_PROTOCOL = 8


class SiltSessionStore:
    def __init__(self, server: str, token: str, repo: str | None = None) -> None:
        self.server = server.rstrip("/")
        self.token = token
        self.repo = repo

    def _call(self, method: str, path: str, body: Any = None) -> Any:
        request = urllib.request.Request(
            f"{self.server}{path}",
            method=method,
            data=None if body is None else json.dumps(body).encode(),
            headers={
                "Authorization": f"Bearer {self.token}",
                "Content-Type": "application/json",
                "silt-protocol": str(SILT_PROTOCOL),
            },
        )
        try:
            response = urllib.request.urlopen(request, timeout=60)
        except urllib.error.HTTPError as error:
            raise RuntimeError(f"Silt answered {error.code} to {method} {path}: {error.read().decode(errors='replace')}") from None
        with response:
            text = response.read()
            return json.loads(text) if text else None

    async def _ask(self, method: str, path: str, body: Any = None) -> Any:
        return await asyncio.to_thread(self._call, method, path, body)

    async def append(self, key: dict, entries: list[dict]) -> None:
        await self._ask("POST", "/v1/session-store/append", {"key": _key(key), "entries": entries, "repo": self.repo})

    async def load(self, key: dict) -> list[dict] | None:
        return await self._ask("POST", "/v1/session-store/load", {"key": _key(key)})

    async def list_sessions(self, project_key: str) -> list[dict]:
        query = urllib.parse.urlencode({"project_key": project_key})
        listed = await self._ask("GET", f"/v1/session-store/sessions?{query}")
        return [{"session_id": item["sessionId"], "mtime": item["mtime"]} for item in listed]

    async def list_subkeys(self, key: dict) -> list[str]:
        return await self._ask(
            "POST", "/v1/session-store/subkeys", {"projectKey": key["project_key"], "sessionId": key["session_id"]}
        )

    async def delete(self, key: dict) -> None:
        await self._ask("POST", "/v1/session-store/delete", {"key": _key(key)})


def _key(key: dict) -> dict:
    """The SDK's Python key, in the server's (and TypeScript's) names."""
    out = {"projectKey": key["project_key"], "sessionId": key["session_id"]}
    if key.get("subpath") is not None:
        out["subpath"] = key["subpath"]
    return out
