#!/usr/bin/env bash
set -Eeuo pipefail

binary="${1:?usage: resume-restart-linux.sh BINARY [SIZE] [PORT]}"
size="${2:-1G}"
port="${3:-49333}"
root="$(mktemp -d "${TMPDIR:-/tmp}/quick-share-resume.XXXXXX")"
receiver_pid=""
sender_pid=""
cleanup() {
    code=$?
    if ((code != 0)); then
        cat "$root/sender-first.log" >&2 2>/dev/null || true
        cat "$root/sender-resume.log" >&2 2>/dev/null || true
        tail -100 "$root/receiver.log" >&2 2>/dev/null || true
    fi
    if [[ -n "$sender_pid" ]]; then kill -9 "$sender_pid" 2>/dev/null || true; fi
    if [[ -n "$receiver_pid" ]]; then kill "$receiver_pid" 2>/dev/null || true; fi
    rm -rf "$root"
    exit "$code"
}
trap cleanup EXIT
mkdir -p "$root/receiver-home" "$root/sender-home" "$root/output"
truncate -s "$size" "$root/payload.bin"

start_receiver() {
    HOME="$root/receiver-home" XDG_CONFIG_HOME="$root/receiver-home/config" \
    XDG_STATE_HOME="$root/receiver-home/state" \
        "$binary" receive --bind 127.0.0.1 --port "$port" --yes \
        --output "$root/output" >>"$root/receiver.log" 2>&1 &
    receiver_pid=$!
    for _ in $(seq 1 100); do
        [[ "$(grep -c 'Quick Share receiver is ready' "$root/receiver.log")" -ge "$1" ]] && return
        kill -0 "$receiver_pid" 2>/dev/null || exit 1
        sleep 0.05
    done
    echo 'receiver readiness timeout' >&2
    exit 1
}

start_receiver 1
HOME="$root/sender-home" XDG_CONFIG_HOME="$root/sender-home/config" \
XDG_STATE_HOME="$root/sender-home/state" \
    "$binary" send --peer "127.0.0.1:$port" --yes "$root/payload.bin" \
    >"$root/sender-first.log" 2>&1 &
sender_pid=$!
for _ in $(seq 1 1000); do
    grep -Eq 'Transfer: [1-9][0-9]+/' "$root/sender-first.log" && break
    kill -0 "$sender_pid" 2>/dev/null || exit 1
    sleep 0.01
done
transfer_id="$(sed -n 's/^Transfer ID: //p' "$root/sender-first.log" | head -1)"
test -n "$transfer_id"
kill -9 "$receiver_pid"
wait "$receiver_pid" 2>/dev/null || true
receiver_pid=""
wait "$sender_pid" 2>/dev/null && { echo 'sender unexpectedly succeeded' >&2; exit 1; } || true
sender_pid=""
test ! -e "$root/output/payload.bin"

start_receiver 2
start_ns="$(date +%s%N)"
HOME="$root/sender-home" XDG_CONFIG_HOME="$root/sender-home/config" \
XDG_STATE_HOME="$root/sender-home/state" \
    "$binary" send --resume "$transfer_id" --peer "127.0.0.1:$port" --yes \
    "$root/payload.bin" >"$root/sender-resume.log" 2>&1
end_ns="$(date +%s%N)"
cmp "$root/payload.bin" "$root/output/payload.bin"
grep -q "Resuming transfer $transfer_id" "$root/sender-resume.log"
kill "$receiver_pid" 2>/dev/null || true
wait "$receiver_pid" 2>/dev/null || true
receiver_pid=""
printf 'resume-restart PASS transfer_id=%s resume_ms=%s\n' \
    "$transfer_id" "$(((end_ns - start_ns) / 1000000))"
