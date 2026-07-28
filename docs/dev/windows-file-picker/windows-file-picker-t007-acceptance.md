# T-007 Windows Desktop Acceptance

- Date: 2026-07-27
- Host: Windows 11 Enterprise 10.0.22621, x86-64, interactive console Session 1
- Result: **passed**

## Native MSVC gate

The Windows host was provisioned with:

- Rust/rustc 1.92.0 (`x86_64-pc-windows-msvc`);
- Cargo 1.92.0 and Clippy 0.1.92;
- Visual Studio 2022 Build Tools, MSVC 14.44.35207;
- Windows SDK resource compiler 10.0.26100.0.

The first full-workspace attempt exhausted virtual memory because the host has no pagefile and Cargo started several LLVM jobs concurrently. No code was changed for this environment failure. The same immutable source/vendor inputs were rerun with `CARGO_BUILD_JOBS=1` and test/dev debuginfo disabled.

Final native results:

- `cargo test --workspace --all-targets --locked --offline` — passed;
- `cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings` — passed;
- `cargo build -p quick-share-cli --locked --offline --release` — passed;
- `cargo build -p quick-share-platform --example windows_desktop_probe --locked --offline --release` — passed.

Production MSVC `quick-share.exe`:

- SHA-256: `3fbe5b1409e4bf898de58c96265cd03d2083c4b4c97d9e7427e8d5b4e28170ad`;
- size: 12,048,384 bytes.

## GNU and manifest evidence

The production CLI also built for `x86_64-pc-windows-gnu` with SHA-256 `d8d55e9c6931091ee6b06076b0a77e1b39ba51c19e2d1c0ce3b20da7f27e098e`.

PE resource inspection verified:

- numeric resource type ID `0x18` (`RT_MANIFEST`), not a string resource;
- resource ID 1 and language `0x409`;
- Microsoft Common Controls v6 dependency;
- `asInvoker` and `uiAccess=false`.

The standalone acceptance probe initially reproduced the expected `0xC0000139` failure because Cargo examples do not pass through the CLI package's resource build script. The production manifest was embedded into that standalone harness with the Windows SDK `mt.exe`; this did not alter production UI logic. The installed acceptance artifact SHA-256 was `1e8652b37d0db4e68b707826043cd322ec74fe52077ca2e42a18832c89c83163`.

## Interactive checklist

The user completed the following on the unlocked interactive Windows desktop and reported every item passed:

1. source dialog displayed `选择文件 / 选择文件夹 / 取消`;
2. native file picker accepted multiple files;
3. tray `Open Quick Share` reopened the workflow;
4. native folder picker opened and accepted/cancelled normally;
5. closing a dialog mapped to cancellation while the tray agent remained available;
6. tray `Exit` removed the tray icon.

System-side post-check of the first run found that the acceptance harness, after correctly closing its tray/event loop, waited indefinitely for its helper thread. This was not accepted as graceful process exit. The harness was repaired to avoid an unbounded post-event-loop join, rebuilt with MSVC, had its manifest re-embedded, and the user repeated the Exit check.

Final independent exit evidence:

- user reported the repeated Exit check passed;
- process count: 0;
- scheduled task state: Ready;
- scheduled task result: `0x00000000`;
- one-time task removed successfully.

## Installed acceptance harness

The non-transferring acceptance harness and shortcut were left available for repeat UI checks:

- `%LOCALAPPDATA%\Programs\Quick Share T007 Acceptance\quick-share-ui-acceptance.exe`;
- desktop shortcut `Quick Share T007 UI 验收`.

It does not read selected payloads or initiate a transfer. T-008/T-009 and the T-010 end-to-end transfer acceptance remain separate gates.
