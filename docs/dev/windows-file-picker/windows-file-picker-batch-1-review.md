# Windows File Picker — Batch 1 Review

- Reviewed at: 2026-07-27T11:08:11Z
- Scope: T-002, T-003, T-004
- Complexity: complex
- Review strategy: two-stage (specification compliance, then code quality)
- Result: **passed**

## Stage 1 — Specification compliance

### Initial findings and repair loop

The first pass compared the implementation and tests directly with the Batch 1 task definitions.

| Finding | Severity | Repair |
|---|---:|---|
| QSP/1.1 selection frames could use the generic control codec without presenting negotiated QSP/1.1 capability state. | Important | Added negotiated encode/decode entry points; generic codecs now reject selection message types, and QSP/1.0 or missing-capability sessions fail closed. |
| `apply-all` was global rather than scoped to subsequent conflicts of the same kind. | Important | Added `ConflictKind` and independent file/directory fallbacks with a mixed-kind regression test. |
| Destination preference had no direct Downloads fallback API and accepted a `TrustedDevice` value without an explicit trust-state gate. | Important | Added `preferred_directory(..., AppDirs)` and changed persistence to require `TrustStatus::Trusted`; Unknown/KeyMismatch cannot persist a directory. |
| Concurrent updates through shared store handles could lose a read-modify-write update. | Important | Added a shared store mutex covering load/modify/save and a concurrent multi-device regression test. |
| Rename planning could select a path reserved by another manifest entry. | Important | Added portable manifest/plan reservation checks and a `file (1)`/`file (2)` regression test. |
| Desktop cancellation was represented in DTOs but not explicitly mapped to `AppError::Cancelled`. | Important | Added DTO result helpers, `require_desktop_choice`, stable desktop error mapping, and CLI contract tests. |

All findings were repaired and re-reviewed before Stage 2.

### T-002 result

**Passed.** Verified in actual protocol, discovery, Noise codec, and fixture tests:

- QSP/1.1 and `RemoteSelection` are explicit additions;
- message codes 11/12 are stable and strictly decoded;
- callback port and response status/transfer-ID combinations fail closed;
- `TransferOffer.initiated_by=None` is omitted, preserving QSP/1.0 offer fixtures;
- QSP/1.0 INFO and mDNS surfaces do not expose the unknown capability;
- negotiation removes `RemoteSelection` below QSP/1.1;
- selection frames cannot bypass authenticated INFO negotiation through the generic codec.

### T-003 result

**Passed.** Verified in actual core stores and destination planning tests:

- directory preference is tied to complete pinned identity and survives rename/restart;
- changed or malformed identity does not inherit a directory;
- Unknown/KeyMismatch trust states cannot persist preferences;
- missing preference falls back to `AppDirs::download_dir()`;
- corrupt/version-invalid state fails closed and writes are atomic;
- shared-handle concurrent writes preserve all device records;
- active bindings are immutable/idempotent and bind transfer, sender, manifest digest, root, and plan;
- overwrite/skip/rename, directory subtree rewrite/skip, and same-kind apply-all are explicit;
- portable case collisions, reserved names through `RelativePath`, path escape, and symlink ancestors remain rejected;
- a property test checks generated commit paths remain normalized beneath the output root;
- persisted binding and plan diagnostics redact local paths.

The planned three core source files were consolidated into `destination.rs` and `receive.rs`; this is a naming-only deviation with the same responsibility split and no requirement loss.

### T-004 result

**Passed.** Verified in actual CLI and platform code:

- `quick-share agent [--port ...] [--bind ...]` parses independently of Windows crates;
- `receive --request [--peer ...]` is explicit and remote request implies one-shot;
- `--peer` requires `--request`;
- non-interactive request without a peer fails closed;
- non-Windows agent execution returns the documented first-phase boundary;
- platform-neutral authorization/source/directory/conflict/notification DTOs and `DesktopInteraction` exist;
- unsupported desktop operations never fabricate a choice;
- cancellation maps to `AppError::Cancelled` (success exit semantics), not filesystem/network failure;
- local path diagnostics are redacted and untrusted remote display fields are documented.

`agent` and remote-request runtime orchestration intentionally remain unavailable until T-008/T-009; T-004 fixes only the public contract and abstraction as specified.

### Stage 1 verdict

**PASS** — no unresolved specification finding remains in Batch 1 scope.

## Stage 2 — Code quality and security

### Initial findings and repair loop

| Finding | Severity | Repair |
|---|---:|---|
| Derived `Debug` output exposed destination roots and planned relative paths. | Important | Added redacted custom `Debug` implementations for `DestinationPlan`, `ReceiveBinding`, and desktop path-bearing DTOs. |
| Destination persistence could accept a malformed ID/key pair and only detect it on reload. | Important | Added complete key-derived `DeviceId` validation before lookup/write. |
| Cross-target test imports produced a Windows-only unused-import warning. | Minor | Removed the target-specific import and verified Windows GNU Clippy with warnings denied. |

All findings were repaired and re-reviewed.

### Quality assessment

| Dimension | Result | Evidence |
|---|---|---|
| Readability | Pass | Protocol, persistence, planning, and desktop boundaries remain separate; public safety behavior is documented. |
| Testability | Pass | Protocol fixtures, strict codec tests, persistence restart/corruption/concurrency tests, conflict regressions, property testing, CLI contracts, and desktop stub tests are present. |
| Maintainability | Pass | No Windows dependency leaks into CLI parsing; stores reuse `atomic_write`; the negotiated codec provides one reusable enforcement point for T-006. |
| Security | Pass | Noise XX boundary is unchanged; new control frames require QSP/1.1 capability negotiation; identity/path checks fail closed; no new secret is persisted; path-bearing diagnostics are redacted. |
| Compatibility | Pass | Existing send/receive/Web/resume tests pass; optional offer correlation is omitted for QSP/1.0; discovery filters the new capability. |

### Stage 2 verdict

**PASS** — no unresolved Critical, Important, or Minor finding remains in Batch 1 scope.

## Verification

- `cargo test --workspace --all-targets` — passed (mDNS true-host test remains intentionally ignored by its existing annotation).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — passed.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings` — passed.
- `cargo audit --no-fetch` — passed.
- `git diff --check` — passed.
- `cargo deny check` — not run because `cargo-deny` is not installed in this environment; no dependency was added beyond using existing workspace dependencies.

## Remaining gates outside Batch 1

- Batch 2 (T-005/T-006) has not started.
- Windows production UI integration remains T-007/T-008.
- Native MSVC test, Clippy, and release build remain a hard gate before T-007 production UI merge, per the approved Conditional Go.
- This batch does not claim a real Linux ↔ Windows transfer; that remains T-010 acceptance scope.
