# 0064: The desktop app starts every backend when it launches

- Status: accepted
- Date: 2026-11-13
- Amends: [ADR 0045](0045-windows-natively-and-in-wsl.md) (a distribution's `silt` looked for when first picked)

## Context
ADR 0045 had the desktop app look for a distribution's `silt` only once it was picked, so that launching the app didn't start every distribution. The app then started only the backend it showed. Someone who works on both sides had to pick each backend in turn, or run `silt open` in PowerShell and in WSL, before both daemons ran. Until a daemon runs on a side, that side's watchers don't notice edits and its sync doesn't push or pull. Capture didn't depend on it, since a hook starts the daemon (ADR 0036).

## Decision
**At launch, the app starts the daemon of every backend it knows**: this computer's and each WSL distribution's, Docker Desktop's left out as before. The window still shows one backend at a time and switches between them under "Runs on".
- The backend shown is started by the window, through `silt open --no-browser`, as before. Each of the others runs `silt daemon start`, outside any repository, in the person's own login shell for a distribution, as the window's start does.
- They start at once, side by side, since a distribution's first start can take seconds.
- A distribution's `silt` is found through a login shell and remembered in `wsl-silt.json`, as before. The window and the launch share what's found, so a distribution is looked for once.
- A distribution without `silt` is left alone. It is still booted to look, at each launch, as nothing records that a distribution lacks it.
- A daemon that stops later isn't started again by the app, except the one the window shows, which it follows as before.

## Consequences
- Launching the app boots each WSL distribution. That costs a few seconds in the background and the distributions' memory while they run.
- `silt daemon stop` on a side holds until the app is launched again, or a hook starts that side's daemon.
- Whether WSL 2 lets a daemon outlive the `wsl.exe` that started it is still the open probe of ADR 0045. If it doesn't, the daemons started at launch stop with their distribution, and keeping one `wsl.exe` open per distribution, which ADR 0045 names, applies to all of them rather than to the one shown.
