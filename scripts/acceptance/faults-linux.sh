#!/usr/bin/env bash
set -Eeuo pipefail

binary="${1:?usage: faults-linux.sh BINARY [BASE_PORT]}"
base_port="${2:-49340}"
root="$(mktemp -d "${TMPDIR:-/tmp}/quick-share-faults.XXXXXX")"
pids=()
cleanup() {
    code=$?
    for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
    chmod -R u+rwX "$root" 2>/dev/null || true
    if ((code != 0)); then find "$root" -name '*.log' -exec sh -c 'echo ===$1; tail -60 "$1"' _ {} \; >&2; fi
    rm -rf "$root"
    exit "$code"
}
trap cleanup EXIT
mkdir -p "$root/source"; printf 'conflict-safe\n' >"$root/source/item.txt"

start_receiver() {
    local name="$1" port="$2" output="$3" extra="${4:-}"
    mkdir -p "$root/$name-home"
    if [[ "$extra" == limited ]]; then
        (trap '' XFSZ; ulimit -f 1024; HOME="$root/$name-home" XDG_CONFIG_HOME="$root/$name-home/config" XDG_STATE_HOME="$root/$name-home/state" exec "$binary" receive --bind 127.0.0.1 --port "$port" --yes --output "$output") >"$root/$name.log" 2>&1 &
    else
        HOME="$root/$name-home" XDG_CONFIG_HOME="$root/$name-home/config" XDG_STATE_HOME="$root/$name-home/state" "$binary" receive --bind 127.0.0.1 --port "$port" --yes --output "$output" >"$root/$name.log" 2>&1 &
    fi
    LAST_PID=$!; pids+=("$LAST_PID")
    for _ in $(seq 1 100); do grep -q 'receiver is ready' "$root/$name.log" && return; kill -0 "$LAST_PID" 2>/dev/null || return 1; sleep .05; done
    return 1
}
send() {
    local name="$1" port="$2" source="$3"
    mkdir -p "$root/$name-home"
    HOME="$root/$name-home" XDG_CONFIG_HOME="$root/$name-home/config" XDG_STATE_HOME="$root/$name-home/state" "$binary" send --peer "127.0.0.1:$port" --yes "$source" >"$root/$name.log" 2>&1
}

# Conflict policy defaults to bounded rename, never overwrite.
mkdir -p "$root/conflict-out"
start_receiver conflict "$base_port" "$root/conflict-out"; conflict_pid=$LAST_PID
send conflict-send-1 "$base_port" "$root/source/item.txt"
send conflict-send-2 "$base_port" "$root/source/item.txt"
cmp "$root/source/item.txt" "$root/conflict-out/item.txt"
cmp "$root/source/item.txt" "$root/conflict-out/item (1).txt"
kill "$conflict_pid" 2>/dev/null || true; wait "$conflict_pid" 2>/dev/null || true

# Unwritable destination reports an actionable remote resource error and no final file.
mkdir -p "$root/readonly"; chmod 500 "$root/readonly"
start_receiver readonly "$((base_port + 1))" "$root/readonly"; readonly_pid=$LAST_PID
if send readonly-send "$((base_port + 1))" "$root/source/item.txt"; then echo 'read-only send unexpectedly succeeded' >&2; exit 1; fi
grep -q 'check receiver disk space, output permissions' "$root/readonly-send.log"
test ! -e "$root/readonly/item.txt"
kill "$readonly_pid" 2>/dev/null || true; wait "$readonly_pid" 2>/dev/null || true
chmod 700 "$root/readonly"

# RLIMIT_FSIZE deterministically emulates storage/quota exhaustion.
mkdir -p "$root/full-out"; truncate -s 16M "$root/source/large.bin"
start_receiver full "$((base_port + 2))" "$root/full-out" limited; full_pid=$LAST_PID
if send full-send "$((base_port + 2))" "$root/source/large.bin"; then echo 'limited send unexpectedly succeeded' >&2; exit 1; fi
grep -q 'check receiver disk space, output permissions' "$root/full-send.log"
test ! -e "$root/full-out/large.bin"
kill "$full_pid" 2>/dev/null || true; wait "$full_pid" 2>/dev/null || true

# Port conflicts fail without touching firewall or another listener.
python3 - "$((base_port + 3))" <<'PY' >"$root/port-holder.log" 2>&1 &
import socket,sys,time
s=socket.socket();s.bind(('127.0.0.1',int(sys.argv[1])));s.listen();time.sleep(30)
PY
holder=$!; pids+=("$holder"); sleep .2
mkdir -p "$root/port-home"
if HOME="$root/port-home" XDG_CONFIG_HOME="$root/port-home/config" XDG_STATE_HOME="$root/port-home/state" "$binary" receive --bind 127.0.0.1 --port "$((base_port + 3))" --once --yes --output "$root/port-out" >"$root/port.log" 2>&1; then echo 'port conflict unexpectedly succeeded' >&2; exit 1; fi
grep -q 'Address already in use' "$root/port.log"
kill "$holder" 2>/dev/null || true; wait "$holder" 2>/dev/null || true

printf 'faults PASS conflict=rename permission=fail-closed storage-limit=fail-closed port-conflict=actionable\n'
