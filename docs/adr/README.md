# Architecture decision records

Each record says why Silt is built the way it is: the context, the decision and its consequences. A later record that changes an earlier one says so in its header (`Amends`), and the earlier one points back (`Amended by`). [How Silt is built](../architecture.md) names the record behind each part.

To add one, copy [0000-template.md](0000-template.md) to the next free number, and link it both ways from any record it changes.

| Record | Decision | Status |
|---|---|---|
| [0001](0001-crdt-design.md) | Eg-walker-style CRDT based on git versions | accepted |
| [0002](0002-git-strategy.md) | Git: cheap checkouts via alternates, gix filters, git CLI for writes | accepted |
| [0003](0003-wire-format.md) | WebSocket + postcard sync protocol, Sans-I/O state machines | accepted |
| [0004](0004-crate-boundaries.md) | Crate boundaries and dependency direction | accepted |
| [0005](0005-acp-registry.md) | Agents are discovered through the public ACP registry | accepted |
| [0006](0006-product-direction.md) | Async handoff, forking and search over live multiplayer | accepted |
| [0007](0007-agent-session-capture.md) | Capturing and restoring native agent sessions | accepted; amended by 0038 |
| [0008](0008-local-capture-architecture.md) | Local capture architecture | accepted; amended by 0018, 0035, 0047, 0089 |
| [0009](0009-sync-and-handoff.md) | HTTP sync, git bundles, thread leases and resume | accepted; amended by 0015, 0032, 0121 |
| [0010](0010-forks-pulls-and-merge-requests.md) | Forks, pulls, merge requests and visibility | accepted; amended by 0029 |
| [0011](0011-history-tools-search-and-web-ui.md) | History tools for agents, search, comments and the web UI | accepted; amended by 0020, 0026, 0055, 0065, 0068, 0112 |
| [0012](0012-agent-adapters-and-repo-conventions.md) | Agent adapters, cross-agent handoff and repo conventions | accepted |
| [0013](0013-accounts-orgs-and-self-hosting.md) | Accounts, orgs, inbox, thread lifecycle and self-hosting | accepted; amended by 0027, 0028, 0032 |
| [0014](0014-live-collaboration-and-silt-driven-agents.md) | Live collaboration, agents from the web UI, the native harness | accepted; amended by 0017, 0081, 0020, 0021, 0022, 0026, 0050, 0056 |
| [0015](0015-secret-overlay.md) | Secret redaction with a local overlay | accepted; amended by 0016, 0017, 0019, 0123 |
| [0016](0016-untrusted-peer-content.md) | Content from other people is untrusted input | accepted; amended by 0017, 0019, 0045 |
| [0017](0017-acp-agents-only.md) | ACP agents only, no native harness | accepted; amended by 0092, 0093 |
| [0018](0018-daemon-optional-capture.md) | Capture works without the daemon | accepted; amended by 0034, 0036, 0089, 0128 |
| [0019](0019-redact-to-gits-threat-model.md) | Redact to git's threat model | accepted; amended by 0060 |
| [0020](0020-next-generation-web-ui.md) | The next-generation web UI | accepted; amended by 0039, 0041 |
| [0021](0021-browser-event-bus.md) | The browser event bus | accepted; amended by 0101, 0127 |
| [0022](0022-agent-runtime.md) | The agent runtime | accepted; amended by 0044, 0093 |
| [0023](0023-projects.md) | Projects as first-class records | accepted; amended by 0044, 0053, 0059, 0065 |
| [0024](0024-per-person-state.md) | Per-person state on the server | accepted; amended by 0067 |
| [0025](0025-comment-replies-and-requested-changes.md) | Comment replies and requested changes | accepted; amended by 0055 |
| [0026](0026-hosted-mode-and-remote-control.md) | Hosted mode and remote control | accepted; amended by 0028, 0056 |
| [0027](0027-read-only-links.md) | Read-only links | accepted; amended by 0072, 0068 |
| [0028](0028-notifications-and-phones.md) | Notifications and phones | accepted; amended by 0112 |
| [0029](0029-fork-from-and-revert-to-a-turn.md) | Forking from and reverting to a turn | accepted |
| [0030](0030-commits-and-turns.md) | Commits and the turns that made them | accepted |
| [0031](0031-permalinks.md) | Permalinks | accepted; amended by 0075 |
| [0032](0032-several-servers-and-local-projects.md) | Several servers per daemon, and projects kept on this machine | accepted; amended by 0043, 0065, 0115 |
| [0033](0033-landing-page-and-site.md) | The landing page and the site | accepted |
| [0034](0034-performance-budgets.md) | Performance budgets, their fixtures, and the gate | accepted; amended by 0052, 0036, 0037, 0091, 0126 |
| [0035](0035-deltas-stored-once-compressed.md) | A family's deltas stored once, compressed in chunks | accepted |
| [0036](0036-a-hook-starts-the-daemon.md) | A hook that finds no daemon starts one | accepted |
| [0037](0037-cloud-machine-budgets.md) | Budgets for a cloud session's machine | accepted; amended by 0126 |
| [0038](0038-cloud-sessions.md) | Recording Claude Code cloud sessions | proposed |
| [0039](0039-the-web-ui-as-designed.md) | The web UI as designed | accepted; amended by 0054, 0044, 0098 |
| [0040](0040-cloud-sessions-from-the-web-ui.md) | Starting and steering Claude Code cloud sessions from the web UI | proposed |
| [0041](0041-the-desktop-app.md) | The desktop app | accepted; amended by 0045, 0108 |
| [0042](0042-git-and-forge-interop.md) | Git notes, the forge bridge and the Agent SDK session store | accepted |
| [0043](0043-history-from-day-one.md) | Your history from day one: release binaries, importing old sessions, sending them | accepted |
| [0044](0044-the-standard-agent-workspace.md) | The standard agent workspace: open on the composer, one shell for every screen | accepted; amended by 0054, 0048, 0049, 0053, 0059, 0062, 0067, 0114, 0115, 0135 |
| [0045](0045-windows-natively-and-in-wsl.md) | Windows, natively and in WSL | accepted; amended by 0119, 0046, 0064, 0108, 0115 |
| [0046](0046-no-server-needed-on-one-computer.md) | No server needed on one computer | accepted |
| [0047](0047-your-clone-between-agents.md) | Your clone between agents | accepted; amended by 0089 |
| [0048](0048-an-agents-modes-are-its-permissions.md) | An agent's modes are its permissions | superseded by [0114](0114-one-set-of-permissions-and-agents-by-their-marks.md); amended by 0065, 0114, 0124 |
| [0049](0049-the-agents-work-grouped-by-turn-or-by-run.md) | The agent's work, grouped by turn or by run | accepted; amended by 0051, 0107 |
| [0050](0050-acp-v2-and-turns-the-agent-opens.md) | ACP v2 where the agent speaks it, and turns the agent opens itself | accepted |
| [0051](0051-subagents-and-background-tasks.md) | Subagents and background tasks | accepted; amended by 0057 |
| [0052](0052-commits-tolerate-a-slower-machine.md) | Commits tolerate a slower machine, and say so | accepted; amended by 0091, 0131 |
| [0053](0053-projects-without-a-remote.md) | Projects without a remote | accepted; amended by 0059 |
| [0054](0054-the-desktop-ui-review.md) | The desktop UI review | accepted; amended by 0058 |
| [0055](0055-reviewing-a-threads-changes.md) | Reviewing a thread's changes | accepted; amended by 0066, 0077, 0068 |
| [0056](0056-signing-an-agent-in-from-the-web-ui.md) | Signing an agent in from the web UI | accepted; amended by 0125 |
| [0057](0057-silt-ends-what-agents-leave-running.md) | Silt ends what agents leave running | accepted |
| [0058](0058-new-threads-settings-outside-the-composer.md) | New thread's project and place outside the composer | accepted; amended by 0059 |
| [0059](0059-chats-without-a-project.md) | Chats without a project | accepted; amended by 0062, 0065 |
| [0060](0060-ignored-files-leave-the-thread-and-large-diffs-are-read-by-file.md) | Files git comes to ignore leave the thread, and a large diff is read a file at a time | accepted |
| [0061](0061-dictation-on-this-machine.md) | Dictation, on this machine | accepted; amended by 0062, 0079 |
| [0062](0062-the-composer-plus-and-enter.md) | The composer: + for what goes with a prompt, ⏎ to send, settings under the box | accepted; amended by 0063 |
| [0063](0063-the-composer-grows-and-keeps-its-draft.md) | The composer: stacked to start, in line in a thread, grows with its text, keeps its draft | accepted |
| [0064](0064-the-desktop-app-starts-every-backend.md) | The desktop app starts every backend when it launches | accepted |
| [0065](0065-workspaces.md) | Workspaces: a server, defaults, rules and history apart for each kind of work | accepted; amended by 0092, 0093, 0114, 0115 |
| [0066](0066-pending-comments-follow-their-text.md) | Pending review comments follow their text | accepted |
| [0067](0067-the-expanded-review.md) | The expanded review: a thread's changes across the page, seen files and reviewed turns | accepted |
| [0068](0068-artifacts.md) | Artifacts: what an agent shows the person, beside the thread | accepted; amended by 0070, 0071, 0072, 0073, 0074, 0075, 0076, 0082, 0121 |
| [0069](0069-git-rewrites-can-be-left-out-of-a-diff.md) | What git rewrote can be left out of a diff | accepted; amended by 0089 |
| [0070](0070-an-artifacts-controls-in-its-header.md) | An artifact's version and view in its header | accepted |
| [0071](0071-artifact-design-guidance-design-systems-and-libraries.md) | Artifact design: built-in guidance, design systems, and libraries from CDNs | accepted; amended by 0072, 0073, 0074, 0076, 0086, 0087, 0088 |
| [0072](0072-what-an-artifacts-page-can-do.md) | What an artifact's page can do: capabilities through the viewer | accepted; amended by 0074, 0077, 0078, 0079, 0080, 0081, 0082 |
| [0073](0073-revising-artifacts-mermaid-in-pages-and-checked-palettes.md) | Revising an artifact in place, Mermaid in pages, notes on versions, and palettes Silt checks | accepted |
| [0074](0074-an-artifacts-files-and-assets.md) | An artifact's files and assets: pages served from a frame address | accepted |
| [0075](0075-every-artifact-in-one-place.md) | Every artifact in one place | accepted |
| [0076](0076-artifact-types-slides-canvases-and-data-pages.md) | Artifact types: slides, design canvases and data pages, drawn by Silt | accepted; amended by 0077, 0080, 0081, 0084 |
| [0077](0077-docs-and-sheets-edited-together.md) | Docs and sheets that people and the agent edit together | accepted; amended by 0082, 0083 |
| [0078](0078-a-page-reads-the-threads-files.md) | A page reads the thread's files | accepted |
| [0079](0079-voice-in-a-page.md) | Voice in a page: dictation and speech | accepted |
| [0080](0080-a-page-calls-mcp-servers.md) | A page calls MCP servers the viewer lets it use | accepted; amended by 0084, 0093 |
| [0081](0081-live-in-a-page-rooms-and-whiteboards.md) | Live in a page: rooms, and whiteboards | accepted |
| [0082](0082-comments-inside-docs-and-pages.md) | Comments inside docs and pages | accepted; amended by 0085 |
| [0083](0083-a-sheets-names-widths-and-formats.md) | A sheet's names, column widths and number formats | accepted |
| [0084](0084-data-pages-live-from-a-connector.md) | A data page's numbers live from a connector | accepted |
| [0085](0085-comments-on-a-pages-elements.md) | Comments on a page's own elements | accepted |
| [0086](0086-design-systems-components.md) | A design system's components | accepted; amended by 0087, 0088 |
| [0087](0087-no-built-in-design-systems.md) | Silt ships no design system of its own | accepted |
| [0088](0088-design-systems-in-settings.md) | Design systems in Settings, made by an agent | accepted; amended by 0096 |
| [0089](0089-a-checkout-per-thread-in-a-shared-clone.md) | A checkout per thread in a shared clone | accepted |
| [0090](0090-agents-aware-of-each-other.md) | Agents in one project know about each other | accepted |
| [0091](0091-a-faster-gate-and-a-pass-that-stands.md) | A faster gate, and a pass that stands | accepted; amended by 0131 |
| [0092](0092-local-models.md) | Local models, run by Silt, and workspaces that stay on the machine | accepted |
| [0093](0093-silt-sandboxes-the-agents-it-starts.md) | Silt sandboxes the agents it starts, and production is reached through plugins | accepted; amended by 0094, 0102, 0103, 0104, 0106, 0135 |
| [0094](0094-silt-s-sandbox-is-its-own-rule.md) | Silt's sandbox is a workspace rule of its own | accepted |
| [0095](0095-an-agent-s-plan-usage-in-the-composer.md) | An agent's plan usage in the composer | accepted |
| [0096](0096-design-systems-made-in-a-thread-or-a-chat.md) | Design systems made in a thread's own words, or in a chat | accepted |
| [0097](0097-wasm-bindings-from-xtask.md) | The browser replica's bindings come from xtask | accepted |
| [0098](0098-themes-for-silts-ui.md) | Themes for Silt's UI | accepted; amended by 0099, 0107 |
| [0099](0099-themes-sync-ship-fonts-and-reach-the-window.md) | Themes sync, ship fonts, and reach the desktop window | accepted |
| [0100](0100-update-channels.md) | Updating from the app, on a stable or a nightly channel | accepted; amended by 0109, 0115, 0129 |
| [0101](0101-the-ui-under-heavy-load.md) | The web UI under heavy load | accepted |
| [0102](0102-agents-ask-for-folders.md) | A sandboxed agent asks for more folders | accepted; amended by 0104 |
| [0103](0103-the-github-plugin.md) | The GitHub plugin | accepted; amended by 0104 |
| [0104](0104-one-plugin-model.md) | One plugin model, in WebAssembly, for Silt's plugins and everyone's | accepted; amended by 0122 |
| [0105](0105-the-plugin-marketplace.md) | The plugin marketplace, self-hostable as Silt is | proposed |
| [0106](0106-silt-s-sandbox-guards-against-an-eager-agent.md) | Silt's sandbox guards against an eager agent, and the person reviews its work | accepted |
| [0107](0107-text-and-layout.md) | Text and layout | accepted |
| [0108](0108-machines-over-ssh.md) | Machines over SSH | accepted; amended by 0109, 0115 |
| [0109](0109-installing-and-updating-silt-over-ssh.md) | Installing and updating Silt over SSH | accepted |
| [0110](0110-suggested-prompts-in-the-composer.md) | Suggested prompts in the composer | accepted |
| [0111](0111-steering-without-stopping.md) | Steering without stopping | accepted |
| [0112](0112-phones-on-the-local-network.md) | Phones on the local network | accepted; amended by 0113 |
| [0113](0113-pairing-a-phone-without-its-certificate.md) | Pairing a phone without its certificate | accepted |
| [0114](0114-one-set-of-permissions-and-agents-by-their-marks.md) | One set of permissions, and agents shown by their marks | accepted; amended by 0118, 0132, 0135 |
| [0115](0115-the-sidebar-names-the-workspace-and-the-machine.md) | The sidebar names the workspace at its top and the machine at its bottom | accepted |
| [0116](0116-memory-that-follows-the-work.md) | Memory that follows the work | accepted |
| [0117](0117-conversations-open-at-their-end.md) | Conversations open at their end | accepted; amended by 0127 |
| [0118](0118-an-agents-own-modes-in-the-permissions-menu.md) | An agent's own modes in the permissions menu | accepted |
| [0119](0119-no-windows-ci-or-releases.md) | No Windows CI or Windows releases | accepted; amended by 0129 |
| [0120](0120-agents-ask-other-agents.md) | Agents ask other agents | accepted |
| [0121](0121-videos-and-images-without-a-size-limit.md) | Videos, and images without a size limit | accepted |
| [0122](0122-the-plugin-sdk-is-mit.md) | The plugin SDK is MIT | accepted |
| [0123](0123-redaction-ids-keyed-per-machine.md) | Redaction ids keyed per machine | accepted |
| [0124](0124-an-agents-own-allow-all-follows-the-preset.md) | An agent's own allow-all follows the preset | accepted; amended by 0118 |
| [0125](0125-antigravity-resumes-from-its-own-store.md) | Antigravity CLI resumes from its own store, and starts primed elsewhere | accepted |
| [0126](0126-budgets-for-a-github-actions-runner.md) | Budgets for a GitHub Actions runner | accepted |
| [0127](0127-switching-threads-waits-for-nothing-else.md) | Switching threads waits for nothing else | accepted; amended by 0128 |
| [0128](0128-a-pages-views-go-ahead-of-an-agents-capture.md) | A page's views go ahead of an agent's capture | accepted |
| [0129](0129-nightly-releases-and-three-channels.md) | Nightly releases, and three channels: stable, nightly and dev | accepted |
| [0130](0130-the-whole-gate-on-every-pull-request.md) | The whole gate on every pull request | accepted |
| [0131](0131-the-gate-runs-on-the-pull-request.md) | The gate runs on the pull request | accepted |
| [0132](0132-auto-for-agents-without-an-auto-mode.md) | Auto for agents without an auto mode | accepted |
| [0133](0133-december-decorations-in-the-desktop-app.md) | December decorations in the desktop app | accepted |
| [0134](0134-no-telemetry-and-an-offline-switch.md) | No telemetry, an offline switch, and artifacts' libraries through Silt by default | accepted |
| [0135](0135-a-threads-header-holds-its-title-and-actions.md) | A thread's header holds its title and actions, and its menu is a right-click | accepted |
| [0136](0136-every-connected-machines-threads-in-one-list.md) | Every connected machine's threads in one list | accepted |
