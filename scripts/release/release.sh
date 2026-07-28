#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/release/release.sh <version> [options]

Options:
  --dry-run       Validate and print the release plan without modifying files
  --changelog     Update Cargo.toml, Cargo.lock, and CHANGELOG.md only
  --skip-tests    Skip local quality gates (GitHub release CI still enforces them)
  --skip-push     Create the release commit and annotated tag locally only
  --no-sync-dev   Accepted for compatibility; this repository has no required dev sync
  -h, --help      Show this help
EOF
}

version=""
dry_run=false
changelog_only=false
skip_tests=false
skip_push=false

while (($#)); do
    case "$1" in
        --dry-run) dry_run=true ;;
        --changelog) changelog_only=true ;;
        --skip-tests) skip_tests=true ;;
        --skip-push) skip_push=true ;;
        --no-sync-dev) ;;
        -h|--help) usage; exit 0 ;;
        v[0-9]*|[0-9]*)
            if [[ -n "$version" ]]; then
                echo "error: version was supplied more than once" >&2
                exit 2
            fi
            version="${1#v}"
            ;;
        *) echo "error: unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

if [[ -z "$version" || ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
    echo "error: a semantic version such as 2.1.0 is required" >&2
    usage >&2
    exit 2
fi

tag="v$version"
root="$(git rev-parse --show-toplevel)"
cd "$root"

for command in git cargo awk sed; do
    command -v "$command" >/dev/null || {
        echo "error: required command is unavailable: $command" >&2
        exit 2
    }
done
if command -v rustup >/dev/null; then
    toolchain_bin="$(dirname "$(rustup which cargo)")"
    export PATH="$toolchain_bin:$PATH"
fi

branch="$(git branch --show-current)"
if [[ "$branch" != "master" && "$branch" != "main" ]]; then
    echo "error: releases must run from master or main (current: $branch)" >&2
    exit 2
fi
if [[ -n "$(git status --porcelain=v1)" ]]; then
    echo "error: working tree must be clean before release" >&2
    git status --short >&2
    exit 2
fi
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    echo "error: immutable tag already exists: $tag" >&2
    exit 2
fi

git fetch origin --tags --prune
if [[ "$(git rev-list --count "origin/$branch..HEAD")" != "0" || \
      "$(git rev-list --count "HEAD..origin/$branch")" != "0" ]]; then
    echo "error: $branch must match origin/$branch before preparing a release" >&2
    exit 2
fi

current_version="$({
    in_workspace=false
    while IFS= read -r line; do
        if [[ "$line" == "[workspace.package]" ]]; then
            in_workspace=true
        elif [[ "$line" == \[*\] ]]; then
            in_workspace=false
        elif $in_workspace && [[ "$line" =~ ^version[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
            printf '%s\n' "${BASH_REMATCH[1]}"
            break
        fi
    done < Cargo.toml
} )"
latest_tag="$(git describe --tags --abbrev=0 2>/dev/null || true)"
range="${latest_tag:+$latest_tag..}HEAD"
commit_count="$(git rev-list --count "$range")"
if [[ "$commit_count" == "0" ]]; then
    echo "error: there are no commits after ${latest_tag:-repository start}" >&2
    exit 2
fi

cat <<EOF
Release plan
  Branch:          $branch
  Current version: $current_version
  Target version:  $version
  Previous tag:    ${latest_tag:-none}
  Commits:         $commit_count
  Mode:            $($changelog_only && echo changelog-only || echo full release)
  Push:            $($skip_push && echo disabled || echo enabled)
EOF

if $dry_run; then
    echo
    echo "Commits included:"
    git log --no-merges --format='  %h %s' "$range"
    exit 0
fi

workspace_tmp="$(mktemp)"
changelog_tmp="$(mktemp)"
section_tmp="$(mktemp)"
trap 'rm -f "$workspace_tmp" "$changelog_tmp" "$section_tmp"' EXIT

awk -v target="$version" '
    /^\[workspace\.package\]$/ { in_workspace=1; print; next }
    /^\[/ { in_workspace=0 }
    in_workspace && /^version[[:space:]]*=/ { print "version = \"" target "\""; in_workspace=0; next }
    { print }
' Cargo.toml > "$workspace_tmp"
mv "$workspace_tmp" Cargo.toml

if ! grep -Fq "## [$version]" CHANGELOG.md; then
    declare -a groups=("Added|^feat(\\([^)]*\\))?:" "Fixed|^fix(\\([^)]*\\))?:" "Changed|^(perf|refactor)(\\([^)]*\\))?:" "Documentation|^docs(\\([^)]*\\))?:" "Tests|^test(\\([^)]*\\))?:" "Maintenance|^(build|ci|chore|style)(\\([^)]*\\))?:")
    {
        echo
        echo "## [$version] - $(date -I)"
        for group in "${groups[@]}"; do
            heading="${group%%|*}"
            pattern="${group#*|}"
            entries="$(git log --no-merges --format='%s' "$range" | grep -E "$pattern" || true)"
            if [[ -n "$entries" ]]; then
                echo
                echo "### $heading"
                while IFS= read -r subject; do
                    [[ -n "$subject" ]] && printf '%s\n' "- $subject"
                done <<< "$entries"
            fi
        done
    } > "$section_tmp"

    unreleased_line="$(grep -n -m1 '^## \[Unreleased\]' CHANGELOG.md | cut -d: -f1)"
    if [[ -z "$unreleased_line" ]]; then
        echo "error: CHANGELOG.md has no [Unreleased] heading" >&2
        exit 2
    fi
    head -n "$unreleased_line" CHANGELOG.md > "$changelog_tmp"
    cat "$section_tmp" >> "$changelog_tmp"
    tail -n "+$((unreleased_line + 1))" CHANGELOG.md >> "$changelog_tmp"
    mv "$changelog_tmp" CHANGELOG.md
fi

has_version_link=false
if grep -Fq "[$version]:" CHANGELOG.md; then
    has_version_link=true
fi
awk -v version="$version" -v add_link="$has_version_link" '
    /^\[Unreleased\]:/ {
        print "[Unreleased]: https://github.com/Newbluecake/quick-share/compare/v" version "...HEAD"
        if (add_link != "true") {
            print "[" version "]: https://github.com/Newbluecake/quick-share/releases/tag/v" version
        }
        next
    }
    { print }
' CHANGELOG.md > "$changelog_tmp"
mv "$changelog_tmp" CHANGELOG.md

# Cargo.toml is the sole package version source. Regenerate only Cargo.lock metadata.
cargo check --workspace >/dev/null

grep -Fq "version = \"$version\"" Cargo.toml
grep -Fq "## [$version]" CHANGELOG.md
git diff --check

if $changelog_only; then
    echo "Prepared version $version and CHANGELOG; no commit, tag, or push was created."
    exit 0
fi

if ! $skip_tests; then
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    cargo test --workspace --all-targets --all-features --locked
    cargo audit
    scripts/release/check-signing-key.sh
    if command -v cargo-deny >/dev/null || cargo deny --version >/dev/null 2>&1; then
        cargo deny check licenses sources
    else
        echo "warning: cargo-deny is unavailable locally; release CI enforces licenses and sources" >&2
    fi
    cargo build --workspace --release --locked
fi

git add Cargo.toml Cargo.lock CHANGELOG.md
if git diff --cached --quiet; then
    echo "Version and changelog were already prepared; tagging the current HEAD."
else
    git commit -m "chore(release): bump version to $tag" -m "Prepare the Cargo workspace, lockfile, and changelog for $tag."
fi

changelog_body="$(awk -v heading="## [$version]" '
    $0 == heading || index($0, heading " - ") == 1 { active=1; next }
    active && /^## \[/ { exit }
    active { print }
' CHANGELOG.md | sed '/^[[:space:]]*$/d' | head -50)"
git tag -a "$tag" -m "Release $tag

$changelog_body"

if $skip_push; then
    echo "Created local release commit and tag $tag; push was skipped."
    exit 0
fi

git push --atomic origin "$branch" "$tag"
echo "Pushed $tag. GitHub Actions release.yml now builds, signs, attests, and publishes assets."
if command -v gh >/dev/null; then
    echo "Monitor with: gh run list --workflow release.yml --branch $tag"
fi
