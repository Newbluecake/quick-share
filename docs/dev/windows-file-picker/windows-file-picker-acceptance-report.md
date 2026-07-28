# Windows File Picker — Acceptance Report

- Date: 2026-07-27
- Feature: cross-device remote source selection and authenticated callback transfer (first phase: Windows native GUI)
- Scope: T-001 through T-010
- Verdict: **CONDITIONAL PASS**

## Verdict summary

All automated gates pass, the full Linux ↔ Windows 11 authenticated remote-selection and callback path was demonstrated on real hardware for single-file and folder transfers, and every requirement has either an automated test or true-host evidence. The verdict is conditional because a subset of P0/P1 behaviors are currently covered by automated tests but were not additionally exercised on the true host in this cycle, and because the final release remains gated by the repository's T-024 release acceptance, `snow 0.10.0` security review, and macOS true-host gates. No version bump, tag, or release is performed by this report.

## Automated verification

- `cargo fmt --all -- --check` — clean.
- `cargo test --workspace --all-targets` — passed (the true-host mDNS test stays intentionally ignored).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — passed.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings` — passed.
- `cargo check -p quick-share-cli --all-targets` (Linux) — passed.
- pinned-nightly `cargo fuzz build noise_records` and a 10-second run — passed, 455,501 runs, no crash.
- `cargo audit --no-fetch` — passed.
- `git diff --check` — clean.
- Windows 11 native MSVC `cargo test --workspace`, Clippy, and release build — passed (T-007 acceptance note).
- `cargo deny check` — not run; `cargo-deny` is not installed in this environment. RustSec is covered by `cargo audit`; license/advisory policy remains a pre-release gate.

## True-host validation (Linux ↔ Windows 11, Session 1)

Executed with an isolated Linux identity and receive directory against the production `quick-share.exe` agent:

- Linux `receive --request --peer` bound a callback listener, authenticated over Noise XX, and sent a QSP/1.1 source-selection request.
- Windows showed the authorization window (device name, DeviceId, verification code, localized buttons); `Allow once` authorized without establishing trust.
- Windows showed the source picker; a single file and, in a separate run, a folder were selected.
- Windows dialed back the authenticated source IP with the pinned key; the Linux one-shot expected-offer matched and received.
- Single file: received SHA-256 matched the source byte for byte.
- Folder: the root directory, a nested subdirectory, an empty directory, and both text files were preserved with exact contents.

Two Critical UI defects were found and fixed during validation: native dialogs parented to the hidden owner rendered without text, and requesting attention on / closing the hidden owner showed a blank window and stopped the agent. Both were repaired (independent foreground TaskDialogs; owner never foregrounded and its close is ignored) and reconfirmed on the true host.

## Requirement evidence (F-001 – F-020)

| ID | Requirement | Evidence | Status |
|---|---|---|---|
| F-001 | Windows multi-file selection | Source mapping unit tests (multi-file); true-host single-file received and hash-verified | Partial (multi-file automated; true-host single only) |
| F-002 | Windows folder selection | True-host folder with nested + empty dirs preserved; core manifest/plan tests | Pass |
| F-003 | Send-selection cancel | Broker/dialog close→Cancelled tests; selection Cancelled→exit 0 test | Pass (test) |
| F-004 | Receive directory confirm | Directory mapping + validation tests; true-host directory confirm | Pass |
| F-005 | Change directory | Directory mapping change/confirm/cancel tests | Pass (test) |
| F-006 | Per-device memory | `ReceiveDestinationStore` restart + full-key tests | Pass (test) |
| F-007 | Stale directory invalidation | Directory validation (invalid/unwritable) tests; store re-selection | Pass (test) |
| F-008 | Paired fast path | Trusted classification / auto path tests | Pass (test) |
| F-009 | Unpaired authorization | Selection Unknown authorize tests; true-host `Allow once` | Pass |
| F-010 | Identity-change isolation | Changed peer rejected in selection/offer + no directory inheritance tests | Pass (test) |
| F-011 | Request de-duplication | Idempotent duplicate request + expected-offer one-shot tests | Pass (test) |
| F-012 | Anti-harassment | Per-device rate, global pending, terminal cache tests | Pass (test) |
| F-013 | Per-entry conflict | `build_plan_sync` overwrite/skip/rename tests; store planned-commit tests | Pass (test) |
| F-014 | Apply-all | `build_plan_sync` apply-all scope + destination apply-all tests | Pass (test) |
| F-015 | Background/tray invocation | Tray action stream + true-host tray Open/Exit (T-007) | Pass |
| F-016 | UI Busy | Broker Busy + selection overlapping-UI Busy tests | Pass (test) |
| F-017 | Non-interactive desktop | Broker event-loop-exit fail-closed; spike no-UI refusal (T-001) | Pass (test) |
| F-018 | Error feedback | Selection terminal status → stable CLI exit-class tests | Pass (test) |
| F-019 | Existing regression | Full send/receive/resume/web/QSP-1.0 suites remain green | Pass |
| F-020 | First-phase boundary | Non-Windows agent usage boundary + `--peer` fail-closed tests | Pass |

## Items deferred to explicit final acceptance

The following are automated-test covered but were not additionally exercised on the true host this cycle, and are carried into the release acceptance:

- multi-file selection on the true host (F-001);
- explicit cancellation on the true host (F-003);
- conflict overwrite/skip/rename and apply-all on the true host (F-013/F-014);
- change-directory and per-device restart recovery on the true host (F-005/F-006);
- identity-change rejection and UI-unavailable fail-closed on the true host (F-010/F-017);
- `cargo deny check` in an environment with `cargo-deny` installed.

## Out of first-phase scope

- Linux and macOS native pickers, auto-start, and a Windows Service remain second-phase; Linux continues to use the existing terminal interaction.

## Release gate

Per repository policy, a final `v2.0.0` tag requires T-024 release acceptance approval and must not bypass the `snow 0.10.0` security-review and macOS true-host gates. This report authorizes neither a version bump nor a tag.
