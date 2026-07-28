# Windows File Picker — Batch 3 Review

- Reviewed at: 2026-07-27
- Scope: T-007, T-008, T-009
- Complexity: complex
- Review strategy: two-stage (specification compliance, then code quality/security), with real Windows 11 ↔ Linux hardware validation
- Result: **passed**

## Stage 1 — Specification compliance

### T-007 — Windows native dialogs, owner window, tray, UI broker

**Passed.** Actual code and tests verify:

- `rfd` file/folder/message dialogs, a hidden `winit` owner window, and a `tray-icon-win` tray menu are wired through a platform-neutral `DesktopInteraction`;
- a bounded, single-flight `DesktopBroker` marshals every request to the owner event-loop thread and enforces Busy, deadline, and event-loop-exit closed states without leaving stale modal work;
- fake-backend mapping tests cover every button and window-close for authorization, source, receive-directory, and the two-step conflict flow, and close always maps to cancellation;
- receive-directory validation rejects non-directories and unwritable targets;
- Common Controls v6 and `asInvoker` are embedded in `quick-share.exe`; the PE carries numeric `RT_MANIFEST` type `0x18`;
- MSVC native workspace test, Clippy, and release build passed on Windows 11.

Two defects were found during real-hardware validation and repaired:

| Finding | Severity | Repair |
|---|---:|---|
| Native dialogs were parented to the deliberately hidden owner window, so the TaskDialog rendered without visible text on the true host. | Critical | Native dialogs now show as independent top-level foreground windows; Common Controls v6 activation is process-wide via the manifest, so no owner parent is required. |
| Requesting user attention on the hidden owner forced a blank, textless owner window to the foreground, and closing it (`CloseRequested`) terminated the resident agent. | Critical | The owner window never requests attention and no longer treats `CloseRequested` as an application-close surface; only tray Exit or the explicit control channel stops the agent. |

Both repairs were confirmed on the true host: the authorization dialog then displayed device name, DeviceId, verification code, and the three localized buttons, and closing stray windows no longer stopped the agent.

### T-008 — Windows agent bidirectional orchestration

**Passed.** Actual code and tests verify:

- a resident `quick-share agent` runs the Windows event loop on the main thread and a Tokio runtime on a background thread, advertising QSP/1.1 with `RemoteSelection`;
- incoming offers never reuse trusted auto-accept: the agent policy is `Confirm`, and the desktop prompt performs identity UI (only for untrusted peers), receive-directory UI, destination planning, and binding strictly before returning Accept;
- directories are remembered only for a fully trusted identity, and a binding or trust-store failure removes the binding instead of authorizing;
- commit-time conflicts prompt overwrite/skip/rename plus this-entry/all-remaining, update the bound plan, and retry, with cancellation mapped to an authorized transfer cancel;
- trusted source requests reach source selection directly while unknown requests authorize first; folder and multi-file selections feed the existing manifest builder in a bounded blocking task;
- the callback dials back only the authenticated control connection's observed source IP with the pinned remote key and the correlated request ID, and a bounded callback semaphore reservation is held from selection through send;
- tray Exit, `Ctrl+C`, and event-loop exit cancel the listener, callbacks, and UI and pause the router.

### T-009 — Linux `receive --request` with expected-callback receipt

**Passed.** Actual code and tests verify:

- the receiver binds its callback listener before sending the request, and auto-discovery lists only peers that negotiated QSP/1.1 `RemoteSelection`;
- the request carries the authenticated requester identity and a nonzero callback port; the agent supplies the source IP from its authenticated socket, never the wire;
- a single `ExpectedOfferRegistry` entry binds request ID, transfer ID, sender DeviceId, complete static key, source IP, and expiry, and only the exact correlated callback offer is auto-accepted once;
- every terminal selection status (`Cancelled`, `Rejected`, `Busy`, `UiUnavailable`, `Expired`, `Failed`, and a `Ready` without a transfer ID) maps to a stable, documented CLI exit class, verified by unit tests;
- non-interactive remote receive still fails closed without `--peer`.

### Stage 1 verdict

**PASS** — no unresolved specification finding remains in Batch 3 scope; the two Critical UI defects were found and repaired during validation.

## Stage 2 — Code quality and security

| Dimension | Result | Evidence |
|---|---|---|
| Readability | Pass | Desktop broker, dialog mapping, native Windows adapter, agent orchestration, desktop prompt, and remote receive live in separate modules with redacted `Debug`. |
| Testability | Pass | Broker marshalling/Busy/deadline/exit, dialog button and directory mapping, selection handler authorize/select/cancel/busy, and remote-receive status classification have focused tests; desktop-only modules are compiled and unit-tested through `#[cfg(test)]`. |
| Maintainability | Pass | `DesktopInteraction`, `DesktopBroker`, `ReceiverEndpoint`, `SelectionHandler`, and `OfferPrompt` are reused without target-specific coupling; the fixed-root receiver path is unchanged. |
| Security | Pass | Noise XX identity boundary is unchanged; callback IP is observed not asserted, keys are fully matched, expected grants are one-shot, directory memory requires full trust, and dialogs never render attacker-controlled text as markup. |
| Compatibility | Pass | Terminal send/receive, Web, QSP/1.0 fixtures, resume, and CLI contracts remain green; QSP/1.1 stays hidden from QSP/1.0 peers. |

### Stage 2 verdict

**PASS** — no unresolved Critical, Important, or Minor finding remains in Batch 3 scope.

## Verification

- `cargo fmt --all -- --check` — clean.
- `cargo test --workspace --all-targets` — passed; the pre-existing true-host mDNS test remains intentionally ignored.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — passed.
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings` — passed.
- pinned-nightly `cargo fuzz build noise_records` and a 10-second run — passed, 455,501 runs with no crash.
- `cargo audit --no-fetch` — passed.
- `git diff --check` — clean.
- Windows 11 native MSVC workspace test, Clippy, and release build — passed (recorded in the T-007 acceptance note).

## Real-hardware validation (Linux ↔ Windows 11)

Executed against an isolated Linux identity and receive directory with the production `quick-share.exe` agent resident on Windows 11 Session 1:

- **F-001** file selection (partial): a real single file was authorized, selected, called back, and received; the received SHA-256 matched the source byte for byte. Multi-file selection was not exercised on the true host and remains open for T-010.
- **F-002** folder: selecting a source folder preserved the root directory, a nested subdirectory, an empty directory, and both text files with exact contents.
- Authorization: `Allow once` authorized without establishing trust; the next request re-prompted. (Requirement F-003 covers explicit cancellation and was not exercised on the true host in this batch.)
- End-to-end path: Linux `receive --request` bind → Noise XX → QSP/1.1 message 11 → Windows authorize → source selection → authenticated callback → one-shot expected-offer match → verified receive and clean receiver exit.

## Remaining gates outside Batch 3

- Conflict overwrite/skip/rename and apply-all (F-013/F-014), change-directory (F-005), identity-change rejection, and UI-unavailable fail-closed are covered by unit/integration tests but were not exercised on the true host in this batch; they remain part of the T-010 end-to-end acceptance.
- The final `v2.0.0` tag still requires the T-024 release acceptance approval and the `snow 0.10.0` security-review and macOS true-host gates.
