# 0132: No telemetry, an offline switch, and artifacts' libraries through Silt by default

- Status: proposed
- Date: 2026-11-26
- Amends: [ADR 0071](0071-artifact-design-guidance-design-systems-and-libraries.md) (libraries load through Silt by default), [ADR 0100](0100-update-channels.md) (update checks can be turned off in Settings and are off offline), [ADR 0095](0095-an-agent-s-plan-usage-in-the-composer.md) (plan usage can be turned off), [ADR 0005](0005-acp-registry.md) (the registry fetch can be turned off)

## Context
People choose Silt to keep their code and conversations under their own control. Tools in the same space have been found sending usage data home by default, with no way to turn it off, while saying they didn't. Silt has never collected telemetry, but nothing said so where people look first, and nothing promised it would stay that way.

What Silt does connect to on its own carries none of a person's data, but it isn't nothing:
- update checks, to GitHub or the clone's remote, every six hours, and plugins' repositories once a day;
- the ACP registry, once a day;
- the agent's provider, with the agent's own sign-in, for plan usage;
- the CDNs an artifact's page names, from every viewer's browser, since ADR 0071 loaded them directly by default, including Google Fonts.

Only `SILT_UPDATE_CHECK=never` turned any of them off, and `network.md` said artifacts' libraries went through Silt when by default they didn't.

## Decision
**Silt collects no telemetry, analytics or crash reports, and won't.** The README, the landing page and `network.md` say so. Every connection Silt makes on its own is listed in `network.md`, with what it carries and how to turn it off; a new one isn't added without its entry there and its switch.

**Settings › Network** holds a switch for each:
- **Offline,** above the rest. On, Silt connects only to the servers it's signed in to, and the others read as off without losing their own values.
- **Check for updates,** Silt's and plugins'.
- **Plan usage:** off, the message box shows only what the agent recorded on this machine.
- **Agent registry:** off, the list built into Silt answers.
- **Load artifacts' libraries and fonts through Silt,** moved here from Appearance. Offline, only what Silt kept is served, and nothing is fetched.

What the person asks for still happens offline: a model or an agent downloaded, a plugin installed, Check now. Those are their choice, made each time.

**The settings live in `$SILT_HOME/network.json`,** read where each connection is about to start, so a change applies at once. The environment wins over the file: `SILT_OFFLINE=1` turns Offline on and `SILT_UPDATE_CHECK=never` turns update checks off, and Settings › Network shows the switch locked, naming the variable.

**Artifacts' libraries load through Silt by default,** on the daemon and on the server (`artifact_proxy` defaults to `true`). A viewer's browser then contacts no CDN; the machine or the server fetches each pinned file once.

## Consequences
- A page that names a library without an exact version, which `artifact-design` already forbids, loads only with the proxy off: the vendor route takes pinned files alone.
- A server or a machine without internet renders pages without the libraries it hasn't kept, as before, but now no viewer contacts a CDN either way.
- A machine that was relying on the old default and wants direct loading turns the switch off; one that set it explicitly keeps its choice.
- The agents' own traffic, including their telemetry, stays outside Silt's reach. `network.md` says so, and Silt turns off Claude Code's nonessential traffic on a local model, as ADR 0092 does.
