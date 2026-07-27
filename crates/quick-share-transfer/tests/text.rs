use quick_share_platform::clipboard::{Clipboard, ClipboardError};
use quick_share_transfer::text::{
    MAX_TEXT_BYTES, TextDeliveryTarget, TextError, TextPayload, TextSource, capture_clipboard,
    deliver_text,
};
use std::fs;
use tempfile::tempdir;

struct FakeClipboard {
    read: Result<String, ClipboardError>,
    write: Result<(), ClipboardError>,
    observed: Option<String>,
}

impl Clipboard for FakeClipboard {
    fn read_text(&mut self) -> Result<String, ClipboardError> {
        self.read.clone()
    }

    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        self.observed = Some(text.to_owned());
        self.write.clone()
    }
}

fn unavailable() -> ClipboardError {
    ClipboardError::Unavailable("headless; use stdout or --output".to_owned())
}

#[test]
fn clipboard_capture_is_bounded_and_debug_redacts_content() {
    let secret = "secret clipboard value";
    let mut clipboard = FakeClipboard {
        read: Ok(secret.to_owned()),
        write: Ok(()),
        observed: None,
    };

    let payload = capture_clipboard(&mut clipboard).expect("capture clipboard");
    let debug = format!("{payload:?}");

    assert_eq!(payload.as_str(), secret);
    assert_eq!(payload.source(), TextSource::Clipboard);
    assert_eq!(payload.summary().bytes, secret.len());
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains(secret));
    assert!(matches!(
        TextPayload::new(TextSource::Literal, "x".repeat(MAX_TEXT_BYTES + 1)),
        Err(TextError::TooLarge { .. })
    ));
}

#[test]
fn successful_native_clipboard_write_needs_no_fallback() {
    let payload = TextPayload::new(TextSource::Literal, "hello").expect("payload");
    let mut clipboard = FakeClipboard {
        read: Err(unavailable()),
        write: Ok(()),
        observed: None,
    };
    let mut stdout = Vec::new();

    let delivery = deliver_text(&payload, &mut clipboard, None, &mut stdout).expect("delivery");

    assert_eq!(delivery.target, TextDeliveryTarget::Clipboard);
    assert!(delivery.clipboard_warning.is_none());
    assert_eq!(clipboard.observed.as_deref(), Some("hello"));
    assert!(stdout.is_empty());
}

#[test]
fn unavailable_clipboard_falls_back_to_stdout_without_executing_text() {
    let root = tempdir().expect("temporary root");
    let marker = root.path().join("must-not-exist");
    let dangerous = format!("touch {}", marker.display());
    let payload = TextPayload::new(TextSource::Literal, dangerous.clone()).expect("payload");
    let mut clipboard = FakeClipboard {
        read: Err(unavailable()),
        write: Err(unavailable()),
        observed: None,
    };
    let mut stdout = Vec::new();

    let delivery = deliver_text(&payload, &mut clipboard, None, &mut stdout).expect("fallback");

    assert_eq!(delivery.target, TextDeliveryTarget::Stdout);
    assert_eq!(stdout, dangerous.as_bytes());
    assert!(!marker.exists());
    assert!(delivery.clipboard_warning.is_some());
}

#[test]
fn explicit_output_bypasses_clipboard_and_never_overwrites() {
    let root = tempdir().expect("temporary root");
    let output = root.path().join("received.txt");
    let payload = TextPayload::new(TextSource::Literal, "received text").expect("payload");
    let mut clipboard = FakeClipboard {
        read: Err(unavailable()),
        write: Ok(()),
        observed: None,
    };
    let mut stdout = Vec::new();

    let delivery =
        deliver_text(&payload, &mut clipboard, Some(&output), &mut stdout).expect("file output");

    assert_eq!(delivery.target, TextDeliveryTarget::File(output.clone()));
    assert_eq!(
        fs::read_to_string(&output).expect("output text"),
        "received text"
    );
    assert!(stdout.is_empty());
    assert!(clipboard.observed.is_none());
    assert!(deliver_text(&payload, &mut clipboard, Some(&output), &mut stdout).is_err());
}
