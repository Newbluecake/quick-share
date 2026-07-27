#!/usr/bin/env bash
set -Eeuo pipefail

binary="${1:?usage: large-transfer-unix.sh BINARY SIZE [PORT]}"
size="${2:?usage: large-transfer-unix.sh BINARY SIZE [PORT]}"
port="${3:-49331}"
chunk_size="${4:-}"
root="$(mktemp -d "${TMPDIR:-/tmp}/quick-share-large.XXXXXX")"
receiver_pid=""
cleanup() {
    code=$?
    if ((code != 0)); then
        cat "$root/sender.log" >&2 2>/dev/null || true
        tail -100 "$root/receiver.log" >&2 2>/dev/null || true
    fi
    if [[ -n "$receiver_pid" ]]; then kill "$receiver_pid" 2>/dev/null || true; fi
    rm -rf "$root"
    exit "$code"
}
trap cleanup EXIT
mkdir -p "$root/receiver-home" "$root/sender-home" "$root/output"
truncate -s "$size" "$root/payload.bin"
bytes="$(stat -c %s "$root/payload.bin")"

HOME="$root/receiver-home" XDG_CONFIG_HOME="$root/receiver-home/config" \
XDG_STATE_HOME="$root/receiver-home/state" \
    /usr/bin/time -v -o "$root/receiver.time" \
    "$binary" receive --bind 127.0.0.1 --port "$port" --once --yes \
    --output "$root/output" >"$root/receiver.log" 2>&1 &
receiver_pid=$!
for _ in $(seq 1 100); do
    grep -q 'Quick Share receiver is ready' "$root/receiver.log" && break
    kill -0 "$receiver_pid" 2>/dev/null || exit 1
    sleep 0.05
done

if [[ -n "$chunk_size" ]]; then
    HOME="$root/sender-home" XDG_CONFIG_HOME="$root/sender-home/config" \
    XDG_STATE_HOME="$root/sender-home/state" \
        "$binary" config set transfer.chunk-size "$chunk_size" >/dev/null
fi
start_ns="$(date +%s%N)"
HOME="$root/sender-home" XDG_CONFIG_HOME="$root/sender-home/config" \
XDG_STATE_HOME="$root/sender-home/state" \
    /usr/bin/time -v -o "$root/sender.time" \
    "$binary" send --peer "127.0.0.1:$port" --yes "$root/payload.bin" \
    >"$root/sender.log" 2>&1
wait "$receiver_pid"
receiver_pid=""
end_ns="$(date +%s%N)"
cmp "$root/payload.bin" "$root/output/payload.bin"
elapsed_ms="$(((end_ns - start_ns) / 1000000))"
throughput_mib="$(awk -v b="$bytes" -v ms="$elapsed_ms" 'BEGIN { printf "%.2f", b / 1048576 / (ms / 1000) }')"
sender_rss="$(awk -F: '/Maximum resident set size/ {gsub(/^[ \t]+/, "", $2); print $2}' "$root/sender.time")"
receiver_rss="$(awk -F: '/Maximum resident set size/ {gsub(/^[ \t]+/, "", $2); print $2}' "$root/receiver.time")"
sender_cpu="$(awk -F: '/Percent of CPU/ {gsub(/^[ \t%]+|[ \t%]+$/, "", $2); print $2}' "$root/sender.time")"
receiver_cpu="$(awk -F: '/Percent of CPU/ {gsub(/^[ \t%]+|[ \t%]+$/, "", $2); print $2}' "$root/receiver.time")"
printf 'large-transfer PASS bytes=%s elapsed_ms=%s throughput_mib_s=%s sender_rss_kib=%s receiver_rss_kib=%s sender_cpu_pct=%s receiver_cpu_pct=%s\n' \
    "$bytes" "$elapsed_ms" "$throughput_mib" "$sender_rss" "$receiver_rss" "$sender_cpu" "$receiver_cpu"
