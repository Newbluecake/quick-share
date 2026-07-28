# Windows File Picker — Batch 2 Review

- Reviewed at: 2026-07-27T12:00:39Z
- Scope: T-005, T-006
- Complexity: complex
- Review strategy: two-stage (specification compliance, then code quality/security)
- Result: **passed**

## Stage 1 — Specification compliance

### Initial findings and repair loop

The first pass independently compared the implementation and tests with the T-005/T-006 task definitions.

| Finding | Severity | Repair |
|---|---:|---|
| A directory overwrite/merge plan and a no-clobber directory plan both encoded `replace_existing=false`, so a commit-time race could not be distinguished from an approved merge. | Important | `DestinationPlan` now records explicit overwrite permission for directories; planned directory commit interprets it as merge for existing directories and replacement for conflicting files. |
| Planned payload commit initially included every staged payload, including text, although desktop destination plans apply to file entries. | Important | `commit_files_planned` now receives an explicit bounded file-entry set; text delivery continues through the existing verified in-memory path. |
| A plan update could change an entry that had already committed or skipped. | Important | File and metadata commit paths now compare resolved journal state with the current plan and fail closed on mutation. |
| `ReceiverRouter` existed independently but the direct server still required concrete `ReceiverService`, preventing an agent from using routed roots. | Important | Added the `ReceiverEndpoint` boundary and made `ServerContext` generic over fixed-root or routed receivers without changing terminal receive behavior. |
| Expected callback auto-authorization had no hook to persist routing/conflict state before granting upload. | Important | `OfferPrompt::prepare_expected` is required and runs before one-shot callback authorization; ordinary `decide` receives peer plus full offer so it can authorize, plan, bind, then return Accept. |
| Selection request registration used two lock phases, allowing simultaneous identical IDs to race into duplicate work. | Important | Registration, replay check, rate check, and pending insertion now occur in one critical section; duplicates subscribe to one terminal result. |
| Dropping a selection future could leave the UI gate or pending capacity stuck. | Important | The UI gate uses an atomic RAII lease; pending records have bounded deadlines and are converted to cached `Expired` results during pruning. |
| A missing output root made the complete binding file look corrupt and could block unrelated routes. | Important | Persisted bindings validate absolute roots without requiring current availability; new bindings require an existing directory, and the router fails only the affected resume with `BindingMismatch`. |

All findings were repaired and re-reviewed before Stage 2.

### T-005 result

**Passed.** Actual code and tests verify:

- one `ReceiverRouter` routes simultaneous transfers to distinct immutable output roots;
- global receive-task and file-stream limits cannot be bypassed by adding roots;
- binding and plan APIs are available before authorization, with direct-server preparation hooks enforcing the ordering for future agent orchestration;
- transfer ID, sender ID, manifest digest, root availability, and complete plan entry set are checked on routing/reopen;
- planned file overwrite, skip, rename, race conflict, plan update, and retry behavior are explicit;
- directory merge, skip subtree, rename prefix rewrite, and descendant file placement are exercised through routed receiver completion;
- no-clobber races return local `ConflictPending` rather than silently overwriting;
- already committed/skipped journal entries cannot be redirected by a later plan;
- planned commit crash injection covers intent, rename, pre-journal, and post-journal points and reopens idempotently;
- paused transfers reopen at the original root, while a detached root fails explicitly;
- cleanup removes staging before deleting its active binding;
- existing fixed-root receiver, resume, store, sender, text, and direct tests continue to pass;
- `ServerContext<P, R>` accepts both fixed-root and routed `ReceiverEndpoint` implementations; the desktop agent still has no implicit default target.

### T-006 result

**Passed.** Actual code and tests verify:

- authenticated peer classification remains based on the complete Noise static key;
- Trusted requests skip authorization UI, Unknown supports AcceptOnce or explicit SAS-confirmed AcceptAndTrust, and Changed rejects by default;
- per-device rate, global pending, terminal cache, and replay structures have nonzero hard upper bounds and bounded durations;
- one request ID invokes the selector once, with concurrent and later duplicates sharing the same terminal result;
- a second modal workflow returns `Busy` rather than overlapping UI;
- timeout cancels/drops preparation and never returns a callback target afterward;
- callback endpoint IP comes only from the authenticated control connection's observed source IP; the wire request supplies only a nonzero port;
- expected callback matching binds request ID, transfer ID, DeviceId, complete static key, source IP, expiry, and correlated `initiatedBy`;
- expected grants are one-shot and replay-cached; mismatched or expired callbacks fail closed;
- expected callback preparation runs before automatic AcceptOnce and ordinary offers still use their existing prompt path;
- direct client/server dispatch uses dynamic INFO, QSP/1.1 negotiated codecs, a separate selection timeout, and stable terminal outcomes;
- QSP/1.0 INFO removes `RemoteSelection`, and a downgraded client cannot encode message 11;
- selection request/response types are exercised by the Noise fuzz target.

### Stage 1 verdict

**PASS** — no unresolved specification finding remains in Batch 2 scope.

## Stage 2 — Code quality and security

### Initial findings and repair loop

| Finding | Severity | Repair |
|---|---:|---|
| Per-root receiver services could each consume their own file-stream limit. | Important | Router-level `(DeviceId, RequestId, TransferId)` reservations now enforce one global file-stream bound and release on final fragment, error, disconnect, or cleanup. |
| Expected callback expired entries and selection rate buckets could accumulate beyond their intended long-lived bounds. | Important | Added expiry pruning, combined replay-cache trimming, empty rate-bucket removal, count ceilings, and duration ceilings. |
| `ReceiverService::Debug` still printed the local output root. | Minor | Output root and all new path/key-bearing state are redacted in `Debug`; network errors remain sanitized. |
| The harness exposes stable Cargo directly rather than the rustup proxy, causing `cargo fuzz` to invoke stable despite `RUSTUP_TOOLCHAIN`. | Environment | Verified with the pinned nightly toolchain directory prepended to `PATH` and `RUSTC` explicitly set; no source workaround was added. |

All code findings were repaired and re-reviewed.

### Quality assessment

| Dimension | Result | Evidence |
|---|---|---|
| Readability | Pass | Receiver routing, expected offers, and selection state live in separate modules; fixed-root receiver behavior remains isolated. |
| Testability | Pass | Store fault injection, routed roots, pause/restart, plan races, identity mismatch, rate/pending/UI/timeout, exact callbacks, and direct QSP downgrade have focused tests. |
| Maintainability | Pass | `ReceiverEndpoint`, `SelectionDispatcher`, and offer preparation contracts are reusable by T-008/T-009 without target-specific dependencies. |
| Security | Pass | Noise XX identity boundary is unchanged; callback IP is observed, keys are fully matched, grants are one-shot, caches are bounded, and path writes stay capability/root checked. |
| Compatibility | Pass | Ordinary offer, fixed receiver, sender, resume, text, Web, QSP/1.0 fixture, and CLI tests remain green. |

### Stage 2 verdict

**PASS** — no unresolved Critical, Important, or Minor finding remains in Batch 2 scope.

## Verification

- `cargo test --workspace --all-targets` — passed; the pre-existing true-host mDNS test remains intentionally ignored.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — passed.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings` — passed.
- `RUSTUP_TOOLCHAIN=nightly-2026-07-01 cargo check --manifest-path fuzz/Cargo.toml --locked` — passed.
- pinned-nightly `cargo fuzz build noise_records` — passed after explicitly placing the nightly toolchain binaries first in `PATH`.
- pinned-nightly `cargo fuzz run noise_records -- -max_total_time=10 -timeout=5` — passed, 187,924 executions with no crash in the recorded run.
- `cargo audit --no-fetch` — passed.
- `git diff --check` — passed.
- `cargo deny check` — unavailable because `cargo-deny` is not installed in this environment.

## Remaining gates outside Batch 2

- No production Windows dialog/tray adapter has been merged; that is T-007.
- No Windows agent or Linux `receive --request` product orchestration has been enabled; those are T-008/T-009.
- This batch validates direct control and callback authorization primitives, not an end-to-end Linux ↔ Windows file transfer; T-010 remains the acceptance gate.
- Native MSVC test, Clippy, and release build remain a hard gate before T-007 production UI merge under the approved Conditional Go.
