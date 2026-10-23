// A Silt server as the Agent SDK's session store (ADR 0042). Copy this file
// into your app; it needs only `fetch`.
//
//   const store = new SiltSessionStore({ server: "https://silt.example", token: process.env.SILT_TOKEN! });
//   query({ prompt, options: { sessionStore: store } });
//
// The token is a device token: `silt auth login` on the app's host writes
// one to `servers.json`, or make one with `silt server pair`.
import type { SessionKey, SessionStore, SessionStoreEntry } from "@anthropic-ai/claude-agent-sdk";

/** The server's `PROTOCOL`: update it with the server. */
export const SILT_PROTOCOL = 6;

export interface SiltSessionStoreOptions {
  /** The server's URL, as devices reach it. */
  server: string;
  /** A device token of the person the sessions belong to. */
  token: string;
  /** The project the sessions belong to, as a normalized remote such as `github.com/acme/app`. */
  repo?: string;
}

export class SiltSessionStore implements SessionStore {
  constructor(private readonly options: SiltSessionStoreOptions) {}

  private async call(method: "GET" | "POST", path: string, body?: unknown): Promise<unknown> {
    const response = await fetch(`${this.options.server.replace(/\/+$/, "")}${path}`, {
      method,
      headers: {
        Authorization: `Bearer ${this.options.token}`,
        "Content-Type": "application/json",
        "silt-protocol": String(SILT_PROTOCOL),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!response.ok) {
      throw new Error(`Silt answered ${response.status} to ${method} ${path}: ${await response.text()}`);
    }
    return response.status === 204 ? undefined : response.json();
  }

  async append(key: SessionKey, entries: SessionStoreEntry[]): Promise<void> {
    await this.call("POST", "/v1/session-store/append", { key, entries, repo: this.options.repo });
  }

  async load(key: SessionKey): Promise<SessionStoreEntry[] | null> {
    return (await this.call("POST", "/v1/session-store/load", { key })) as SessionStoreEntry[] | null;
  }

  async listSessions(projectKey: string): Promise<Array<{ sessionId: string; mtime: number }>> {
    const query = new URLSearchParams({ project_key: projectKey });
    return (await this.call("GET", `/v1/session-store/sessions?${query}`)) as Array<{ sessionId: string; mtime: number }>;
  }

  async listSubkeys(key: { projectKey: string; sessionId: string }): Promise<string[]> {
    return (await this.call("POST", "/v1/session-store/subkeys", key)) as string[];
  }

  async delete(key: SessionKey): Promise<void> {
    await this.call("POST", "/v1/session-store/delete", { key });
  }
}
