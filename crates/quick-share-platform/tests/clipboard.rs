#[cfg(windows)]
use quick_share_platform::clipboard::{Clipboard, NativeClipboard};
#[cfg(all(unix, not(target_os = "macos")))]
use quick_share_platform::clipboard::{ClipboardError, NativeClipboard};

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn headless_linux_returns_an_actionable_structured_error() {
    if std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return;
    }

    let result = NativeClipboard::connect();

    assert!(
        matches!(result, Err(ClipboardError::Unavailable(message)) if message.contains("--output"))
    );
}

#[cfg(windows)]
#[test]
#[ignore = "modifies the true-host Windows clipboard"]
fn windows_native_unicode_clipboard_round_trip() {
    let expected = "Quick Share Windows 剪贴板 🚀";
    let mut clipboard = NativeClipboard::connect().expect("native clipboard");

    clipboard.write_text(expected).expect("write clipboard");
    let observed = clipboard.read_text().expect("read clipboard");

    assert_eq!(observed, expected);
}
