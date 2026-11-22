#!/bin/sh
# The screen references' world (ADR 0039): live-fixture.sh's server, Alice,
# Bob and Alice's laptop, plus a fork of Alice's thread with a merge request
# from it. Prints live-fixture.sh's JSON line with the fork added.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
silt="$root/target/debug/silt"
# What an agent offers comes from its runs here, not from asking it in the background, whose answer could land
# before or after a screen is taken.
export SILT_ASK_AGENTS=0
# Phones on this network name the machine by its host name otherwise, which differs on each machine taking the screens.
export SILT_LAN_NAME=alice
line=$("$here/live-fixture.sh" | tail -n 1)
work=$(printf '%s' "$line" | sed 's/.*"work":"\([^"]*\)".*/\1/')
thread=$(printf '%s' "$line" | sed 's/.*"thread":"\([^"]*\)".*/\1/')

export SILT_HOME="$work/alice/silt"
export SILT_VOICE_BIN="${SILT_VOICE_BIN:-/nonexistent}"
export CLAUDE_CONFIG_DIR="$work/alice/claude"
# Agent sessions on the machine running the tests would show in the import
# offer, so each machine here has a home of its own.
export HOME="$work/alice/home"
unset CODEX_HOME GEMINI_CLI_HOME COPILOT_HOME XDG_DATA_HOME XDG_CONFIG_HOME
mkdir -p "$HOME"
export SILT_CLAUDE_BIN="$here/fake-claude.sh"
export SILT_HOOK="$silt hook"
cd "$work/alice/repo"
FAKE_SAY="Retry blob uploads on a 5xx" FAKE_LINE="fn retry() {}" "$silt" fork "$thread" > /dev/null 2>&1
fork=$("$silt" log | awk '/fork of/ { print $1; exit }')
"$silt" mr create --thread "$fork" --title "Retry blob uploads on 5xx" > /dev/null

# A design system of Alice's own (ADR 0088), in her data folder rather than her repository, where new files would
# show in her threads' changes. Its face is a file of its own, the web UI's IBM Plex Sans, so its frames draw the same
# on every machine: `system-ui` is whatever fontconfig picks there.
harbor="$SILT_HOME/design-systems/harbor"
mkdir -p "$harbor/components" "$harbor/fonts"
for weight in 400 500 600 700; do
    cp "$root/web/node_modules/@fontsource/ibm-plex-sans/files/ibm-plex-sans-latin-$weight-normal.woff2" "$harbor/fonts/harbor-sans-$weight.woff2"
done
cat > "$harbor/tokens.json" <<'TOKENS'
{
  "name": "Harbor",
  "version": 1,
  "meta": { "source": "description" },
  "color": {
    "themes": [{ "id": "light", "name": "Light" }, { "id": "dark", "name": "Dark" }],
    "tokens": [
      { "name": "bg", "value": { "light": "#f4f6f7", "dark": "#0f1a20" }, "usage": "Page background." },
      { "name": "surface-1", "value": { "light": "#e8eef1", "dark": "#16252d" }, "usage": "Cards and panels, on bg." },
      { "name": "fg-1", "value": { "light": "#13242c", "dark": "#e6eef2" }, "usage": "Body text on bg and surface-1." },
      { "name": "fg-2", "value": { "light": "#46606d", "dark": "#a5bac4" }, "usage": "Secondary text on bg." },
      { "name": "border", "value": { "light": "#c9d6dc", "dark": "#2b3f49" }, "usage": "Hairlines around cards and fields." },
      { "name": "accent", "value": { "light": "#0f6e75", "dark": "#4fb3b9" }, "usage": "The primary action and the focus ring." },
      { "name": "fg-on-accent", "value": { "light": "#ffffff", "dark": "#0f1a20" }, "usage": "Text on accent." },
      { "name": "ok", "value": { "light": "#2e7a4b", "dark": "#6cc28b" }, "usage": "Done, always with a word." }
    ]
  },
  "type": {
    "fonts": [
      { "family": "Harbor Sans", "file": "fonts/harbor-sans-400.woff2", "weight": "400" },
      { "family": "Harbor Sans", "file": "fonts/harbor-sans-500.woff2", "weight": "500" },
      { "family": "Harbor Sans", "file": "fonts/harbor-sans-600.woff2", "weight": "600" },
      { "family": "Harbor Sans", "file": "fonts/harbor-sans-700.woff2", "weight": "700" }
    ],
    "families": { "sans": "\"Harbor Sans\", sans-serif" },
    "groups": [{ "name": "Text", "family": "sans", "styles": [
      { "name": "body", "fontSize": "14px", "lineHeight": 1.55, "fontWeight": 400, "usage": "Running text." },
      { "name": "title", "fontSize": "20px", "lineHeight": 1.25, "fontWeight": 600, "usage": "Page titles." }
    ] }]
  },
  "spacing": { "tokens": [{ "name": "space-2", "value": "8px", "usage": "Between controls." }, { "name": "space-4", "value": "16px", "usage": "Card padding." }] },
  "radius": { "tokens": [{ "name": "radius-md", "value": "6px", "usage": "Cards, buttons and fields." }] }
}
TOKENS
printf 'Calm and nautical. Set text in `body`, `fg-1` on `bg`; `accent` for the one primary action a screen has.\n' > "$harbor/README.md"
cat > "$harbor/components.css" <<'CSS'
.h-button { display: inline-flex; align-items: center; padding: 6px 14px; border: 1px solid var(--border); border-radius: var(--radius-md); background: var(--surface-1); color: var(--fg-1); font: 500 13px/1.4 var(--font-sans); }
.h-button.h-primary { border-color: transparent; background: var(--accent); color: var(--fg-on-accent); }
.h-card { max-width: 320px; padding: var(--space-4); border: 1px solid var(--border); border-radius: var(--radius-md); background: var(--surface-1); color: var(--fg-1); font: 400 14px/1.5 var(--font-sans); }
.h-card b { display: block; font-size: 16px; }
.h-tag { padding: 1px 8px; border-radius: 99px; background: var(--surface-1); color: var(--ok); font: 500 12px/1.5 var(--font-sans); }
CSS
printf '<!-- Buttons: secondary by default, h-primary once per screen. -->\n<button class="h-button h-primary">Set sail</button>\n<button class="h-button">Moor</button>\n' > "$harbor/components/button.html"
printf 'Starts an action. Use one primary button a screen.\n' > "$harbor/components/button.md"
printf '<!-- A card: a title and a line of detail. -->\n<div class="h-card"><b>Morning tide</b>High water at 06:40.</div>\n' > "$harbor/components/card.html"
printf '<!-- A status tag, always with its word. -->\n<span class="h-tag">Docked</span>\n' > "$harbor/components/tag.html"

printf '%s,"fork":"%s"}\n' "${line%\}}" "$fork"
