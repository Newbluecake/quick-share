#!/usr/bin/env bash
set -Eeuo pipefail

binary="${1:?usage: small-files-linux.sh BINARY [COUNT] [PORT]}"
count="${2:-10000}"
port="${3:-49332}"
if ((count < 1 || count > 10000)); then echo 'count must be 1..10000' >&2; exit 2; fi
root="$(mktemp -d "${TMPDIR:-/tmp}/quick-share-small.XXXXXX")"
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
mkdir -p "$root/receiver-home" "$root/sender-home" "$root/output" "$root/source/empty-dir"
for ((index = 0; index < count; index++)); do
    directory="$root/source/dir-$(printf '%03d' "$((index / 100))")"
    mkdir -p "$directory"
    printf '%05d\n' "$index" >"$directory/file-$(printf '%05d' "$index")-中文.txt"
done

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

start_ns="$(date +%s%N)"
HOME="$root/sender-home" XDG_CONFIG_HOME="$root/sender-home/config" \
XDG_STATE_HOME="$root/sender-home/state" \
    /usr/bin/time -v -o "$root/sender.time" \
    "$binary" send --peer "127.0.0.1:$port" --yes "$root/source" \
    >"$root/sender.log" 2>&1
wait "$receiver_pid"
receiver_pid=""
end_ns="$(date +%s%N)"
test "$(find "$root/output/source" -type f | wc -l)" -eq "$count"
test -d "$root/output/source/empty-dir"
diff -qr "$root/source" "$root/output/source" >/dev/null
elapsed_ms="$(((end_ns - start_ns) / 1000000))"
sender_rss="$(awk -F: '/Maximum resident set size/ {gsub(/^[ \t]+/, "", $2); print $2}' "$root/sender.time")"
receiver_rss="$(awk -F: '/Maximum resident set size/ {gsub(/^[ \t]+/, "", $2); print $2}' "$root/receiver.time")"
printf 'small-files PASS count=%s elapsed_ms=%s files_per_second=%.2f sender_rss_kib=%s receiver_rss_kib=%s\n' \
    "$count" "$elapsed_ms" "$(awk -v n="$count" -v ms="$elapsed_ms" 'BEGIN { print n / (ms / 1000) }')" \
    "$sender_rss" "$receiver_rss"
