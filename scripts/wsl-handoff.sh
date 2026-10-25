#!/usr/bin/env bash
# Hands a thread from a Windows machine to a WSL distribution's and back,
# through a server running on Windows (ADR 0045). Run from Git Bash:
#
#   SILT=silt.exe FAKE_CLAUDE=silt-fake-claude.exe scripts/wsl-handoff.sh <distribution>
#
# with `silt` and `silt-fake-claude` on the distribution's PATH. The
# distribution shares Windows' loopback, as WSL 1 and WSL 2's mirrored mode
# do. SILT_HANDOFF_LOCAL=1 runs the distribution's side here instead, to try
# the script on Linux.
set -euo pipefail

distribution=${1:?usage: wsl-handoff.sh <distribution>}
silt=${SILT:?SILT names the Windows silt.exe}
fake=${FAKE_CLAUDE:?FAKE_CLAUDE names the Windows silt-fake-claude.exe}
local_run=${SILT_HANDOFF_LOCAL:-}
# Git Bash would rewrite arguments that look like Unix paths, which the
# distribution's commands are.
export MSYS2_ARG_CONV_EXCL='*'

# A path as Windows programs take it, and as the distribution sees it.
windows_path() { if [ -n "$local_run" ]; then printf '%s' "$1"; else cygpath -m "$1"; fi; }
in_wsl() {
    if [ -n "$local_run" ]; then bash -c "$1"; else wsl.exe -d "$distribution" --exec bash -lc "$1"; fi
}
wsl_path() { if [ -n "$local_run" ]; then printf '%s' "$1"; else in_wsl "wslpath -u '$(cygpath -w "$1")'"; fi; }

root=$(mktemp -d)
port=${SILT_HANDOFF_PORT:-47213}
url=http://127.0.0.1:$port
server_pid=

finish() {
    native daemon stop >/dev/null 2>&1 || true
    on_wsl 'silt daemon stop' >/dev/null 2>&1 || true
    [ -n "$server_pid" ] && kill "$server_pid" 2>/dev/null || true
}
trap finish EXIT

eventually() {
    for _ in $(seq 120); do
        if eval "$1"; then return 0; fi
        sleep 1
    done
    echo "wsl-handoff: timed out waiting for: $1" >&2
    return 1
}

git_t() { git -c user.name=t -c user.email=t@example.com "$@"; }

# What `silt claude` recorded, from its `silt: recording thread <id>`.
recorded() { sed -n 's/^silt: recording thread \([0-9a-f]*\).*/\1/p' | head -n 1; }
# The checkout `silt resume` reported, as the tests read it.
checkout_in() { grep -E 'is now here, in |, in ' | head -n 1 | sed 's/.*, in //; s/^is now here, in //'; }

expect_main() {
    local got want
    got=$(cat "$1")
    want=$(printf 'fn main() {}\nfn farewell() {}\n%s' "$2")
    if [ "$got" != "$want" ]; then
        printf 'wsl-handoff: %s holds\n%s\nnot\n%s\n' "$1" "$got" "$want" >&2
        return 1
    fi
}

git_t init -q -b main "$root/seed"
mkdir "$root/seed/src"
printf 'fn main() {}\n' >"$root/seed/src/main.rs"
git_t -C "$root/seed" add .
git_t -C "$root/seed" commit -q -m init
git clone -q --bare "$root/seed" "$root/origin.git"

"$silt" server run --listen "127.0.0.1:$port" --data "$(windows_path "$root/server")" 2>"$root/server.log" &
server_pid=$!
eventually 'grep -q "listening on" "$root/server.log"'
pair() { "$silt" server pair --user owner --data "$(windows_path "$root/server")"; }

# The Windows machine, in a home of its own whose git keeps LF.
win=$root/windows
mkdir -p "$win/claude"
printf '[core]\n\tautocrlf = false\n' >"$win/.gitconfig"
native() {
    (
        cd "$win/repo" && HOME=$(windows_path "$win") SILT_HOME=$(windows_path "$win/silt") \
            CLAUDE_CONFIG_DIR=$(windows_path "$win/claude") SILT_CLAUDE_BIN=$(windows_path "$fake") "$silt" "$@"
    )
}
HOME=$win git clone -q "$root/origin.git" "$win/repo"
native auth login --server "$url" --code "$(pair)" --name windows

# The distribution's machine, in a home of its own there.
on_wsl() {
    in_wsl "set -eu
        machine=\$HOME/silt-handoff-$(basename "$root")
        export HOME=\$machine SILT_HOME=\$machine/silt CLAUDE_CONFIG_DIR=\$machine/claude
        export SILT_CLAUDE_BIN=\$(command -v silt-fake-claude)
        mkdir -p \$machine/claude && cd \$machine
        if [ -d repo ]; then cd repo; fi
        $1"
}
on_wsl "printf '[core]\n\tautocrlf = false\n' > .gitconfig
    git clone -q '$(wsl_path "$root/origin.git")' repo
    cd repo && silt auth login --server $url --code $(pair) --name wsl"

echo "wsl-handoff: Windows records a thread, and $distribution continues it"
native claude 2>"$root/windows-claude.log" || { cat "$root/windows-claude.log" >&2; exit 1; }
from_windows=$(recorded <"$root/windows-claude.log")
[ -n "$from_windows" ] || { cat "$root/windows-claude.log" >&2; exit 1; }
on_wsl "for _ in \$(seq 120); do silt log | grep -q '^$from_windows' && break; sleep 1; done
    resumed=\$(FAKE_LINE='fn wsl() {}' silt resume $from_windows 2>&1) || { echo \"\$resumed\" >&2; exit 1; }
    checkout=\$(echo \"\$resumed\" | grep -E 'is now here, in |, in ' | head -n 1 | sed 's/.*, in //; s/^is now here, in //')
    got=\$(cat \"\$checkout/src/main.rs\")
    want=\$(printf 'fn main() {}\nfn farewell() {}\nfn wsl() {}')
    [ \"\$got\" = \"\$want\" ] || { printf '%s holds\n%s\n' \"\$checkout/src/main.rs\" \"\$got\" >&2; exit 1; }"

echo "wsl-handoff: $distribution records a thread, and Windows continues it"
from_wsl=$(on_wsl 'silt claude 2>&1 >/dev/null
    silt sync --once >/dev/null 2>&1 || true' | recorded)
[ -n "$from_wsl" ] || { echo "wsl-handoff: $distribution recorded no thread" >&2; exit 1; }
eventually "native log | grep -q '^$from_wsl'"
FAKE_LINE='fn windows() {}' native resume "$from_wsl" 2>"$root/windows-resume.log" || { cat "$root/windows-resume.log" >&2; exit 1; }
checkout=$(checkout_in <"$root/windows-resume.log")
if [ -z "$local_run" ]; then checkout=$(cygpath -u "$checkout"); fi
expect_main "$checkout/src/main.rs" 'fn windows() {}'

echo "wsl-handoff: a thread went from Windows to $distribution and another back"
