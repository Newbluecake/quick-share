#!/usr/bin/env bash
set -Eeuo pipefail

binary="${1:?binary path is required}"
"$binary" --version
"$binary" --help >/dev/null

root="$(mktemp -d "${TMPDIR:-/tmp}/quick-share-smoke.XXXXXX")"
receiver_pid=""
cleanup() {
    if [[ -n "$receiver_pid" ]]; then kill "$receiver_pid" 2>/dev/null || true; fi
    rm -rf "$root"
}
trap cleanup EXIT
mkdir -p "$root/receiver-home" "$root/sender-home" "$root/output"
printf 'release-loopback-%s\n' "$(uname -s)" > "$root/payload.txt"

HOME="$root/receiver-home" XDG_CONFIG_HOME="$root/receiver-home/config" \
XDG_STATE_HOME="$root/receiver-home/state" XDG_CACHE_HOME="$root/receiver-home/cache" \
    "$binary" receive --bind 127.0.0.1 --port 49327 --once --yes \
    --output "$root/output" >"$root/receiver.log" 2>&1 &
receiver_pid=$!
for _ in $(seq 1 50); do
    if grep -q 'Quick Share receiver is ready' "$root/receiver.log"; then break; fi
    if ! kill -0 "$receiver_pid" 2>/dev/null; then cat "$root/receiver.log" >&2; exit 1; fi
    sleep 0.1
done

HOME="$root/sender-home" XDG_CONFIG_HOME="$root/sender-home/config" \
XDG_STATE_HOME="$root/sender-home/state" XDG_CACHE_HOME="$root/sender-home/cache" \
    "$binary" send --peer 127.0.0.1:49327 --yes "$root/payload.txt" \
    >"$root/sender.log" 2>&1 || {
        cat "$root/sender.log" >&2
        cat "$root/receiver.log" >&2
        exit 1
    }

for _ in $(seq 1 100); do
    if ! kill -0 "$receiver_pid" 2>/dev/null; then break; fi
    sleep 0.1
done
if kill -0 "$receiver_pid" 2>/dev/null; then
    cat "$root/receiver.log" >&2
    exit 1
fi
wait "$receiver_pid"
receiver_pid=""
cmp "$root/payload.txt" "$root/output/payload.txt"
