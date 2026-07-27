#!/usr/bin/env bash
set -Eeuo pipefail

binary="${1:?usage: routing-linux.sh BINARY [PORT]}"
port="${2:-49350}"
root="$(mktemp -d "${TMPDIR:-/tmp}/quick-share-routing.XXXXXX")"
pids=()
cleanup() { code=$?; for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done; if ((code)); then find "$root" -name '*.log' -exec sh -c 'echo ===$1; tail -80 "$1"' _ {} \; >&2; fi; rm -rf "$root"; exit "$code"; }
trap cleanup EXIT
mkdir -p "$root/source"; printf 'routing\n' >"$root/source/route.txt"

start_receiver() {
    local name="$1" approve="$2"
    mkdir -p "$root/$name-home" "$root/$name-out"
    local args=(receive --bind lan --port "$port" --once --output "$root/$name-out")
    [[ "$approve" == yes ]] && args+=(--yes)
    HOME="$root/$name-home" XDG_CONFIG_HOME="$root/$name-home/config" XDG_STATE_HOME="$root/$name-home/state" "$binary" "${args[@]}" >"$root/$name.log" 2>&1 &
    LAST_PID=$!; pids+=("$LAST_PID")
    for _ in $(seq 1 100); do grep -q 'receiver is ready' "$root/$name.log" && return; kill -0 "$LAST_PID" 2>/dev/null || return 1; sleep .05; done
    return 1
}

# Auto discovery selects the sole compatible receiver and completes in the default ~2s window.
start_receiver direct yes; direct_pid=$LAST_PID
mkdir -p "$root/sender-home"
start_ns=$(date +%s%N)
HOME="$root/sender-home" XDG_CONFIG_HOME="$root/sender-home/config" XDG_STATE_HOME="$root/sender-home/state" "$binary" send --yes "$root/source/route.txt" >"$root/direct-send.log" 2>&1
end_ns=$(date +%s%N)
wait "$direct_pid"; cmp "$root/source/route.txt" "$root/direct-out/route.txt"
grep -q 'encrypted device-to-device' "$root/direct-send.log"
discovery_transfer_ms=$(((end_ns-start_ns)/1000000))

# A discovered receiver rejection is terminal and never opens Web sharing.
sleep .5
start_receiver reject no; reject_pid=$LAST_PID
if HOME="$root/reject-sender-home" XDG_CONFIG_HOME="$root/reject-sender-home/config" XDG_STATE_HOME="$root/reject-sender-home/state" "$binary" send --yes "$root/source/route.txt" >"$root/reject-send.log" 2>&1; then echo 'rejected transfer unexpectedly succeeded' >&2; exit 1; fi
wait "$reject_pid" || true
grep -Eq 'rejected|expired' "$root/reject-send.log"
! grep -q 'Traditional Web sharing' "$root/reject-send.log"

# Definitive zero peers is the only automatic Web transition.
sleep 1
mkdir -p "$root/zero-home"
set +e
HOME="$root/zero-home" XDG_CONFIG_HOME="$root/zero-home/config" XDG_STATE_HOME="$root/zero-home/state" timeout --signal=INT 7 "$binary" send --yes "$root/source/route.txt" >"$root/zero.log" 2>&1
zero_status=$?
set -e
[[ "$zero_status" == 124 || "$zero_status" == 130 || "$zero_status" == 0 ]]
grep -q 'Mode: traditional Web sharing over HTTPS' "$root/zero.log"
grep -q 'Traditional Web sharing is ready' "$root/zero.log"

# Explicit peer failure and explicit Web keep their closed routing semantics.
if HOME="$root/peer-home" XDG_CONFIG_HOME="$root/peer-home/config" XDG_STATE_HOME="$root/peer-home/state" "$binary" send --peer 127.0.0.1:1 --yes "$root/source/route.txt" >"$root/peer.log" 2>&1; then echo 'bad peer unexpectedly succeeded' >&2; exit 1; fi
! grep -q 'Traditional Web sharing' "$root/peer.log"
set +e
HOME="$root/web-home" XDG_CONFIG_HOME="$root/web-home/config" XDG_STATE_HOME="$root/web-home/state" timeout --signal=INT 4 "$binary" send --web --yes "$root/source/route.txt" >"$root/web.log" 2>&1
web_status=$?
set -e
[[ "$web_status" == 124 || "$web_status" == 130 || "$web_status" == 0 ]]
grep -q 'Traditional Web sharing is ready' "$root/web.log"

printf 'routing PASS discovery_and_transfer_ms=%s rejection_no_fallback=yes zero_peer_web=yes explicit_modes=yes\n' "$discovery_transfer_ms"
