use anyhow::Result;
use arboard::Clipboard;
use clap::{Parser, ValueEnum};
use serde::Serialize;
use std::env;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Mode {
    Read,
    Roundtrip,
}

#[derive(Debug, Parser)]
#[command(about = "Disposable Quick Share native clipboard capability probe")]
struct Args {
    #[arg(long, value_enum, default_value_t = Mode::Read)]
    mode: Mode,
    #[arg(long, default_value = "quick-share-clipboard-spike")]
    text: String,
}

#[derive(Debug, Serialize)]
struct ProbeResult {
    os: &'static str,
    arch: &'static str,
    display: Option<String>,
    wayland_display: Option<String>,
    mode: String,
    clipboard_available: bool,
    read_succeeded: bool,
    write_succeeded: bool,
    observed_length: Option<usize>,
    error: Option<String>,
    fallback: &'static str,
}

fn sanitized_error(error: impl std::fmt::Display) -> String {
    let value = error.to_string();
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(300)
        .collect()
}

fn probe(args: &Args) -> ProbeResult {
    let mut result = ProbeResult {
        os: env::consts::OS,
        arch: env::consts::ARCH,
        display: env::var("DISPLAY").ok(),
        wayland_display: env::var("WAYLAND_DISPLAY").ok(),
        mode: format!("{:?}", args.mode).to_lowercase(),
        clipboard_available: false,
        read_succeeded: false,
        write_succeeded: false,
        observed_length: None,
        error: None,
        fallback: "stdout or --output file",
    };

    let mut clipboard = match Clipboard::new() {
        Ok(clipboard) => {
            result.clipboard_available = true;
            clipboard
        }
        Err(error) => {
            result.error = Some(sanitized_error(error));
            return result;
        }
    };

    match args.mode {
        Mode::Read => match clipboard.get_text() {
            Ok(text) => {
                result.read_succeeded = true;
                result.observed_length = Some(text.len());
            }
            Err(error) => result.error = Some(sanitized_error(error)),
        },
        Mode::Roundtrip => match clipboard.set_text(args.text.clone()) {
            Ok(()) => {
                result.write_succeeded = true;
                match clipboard.get_text() {
                    Ok(text) => {
                        result.read_succeeded = text == args.text;
                        result.observed_length = Some(text.len());
                        if text != args.text {
                            result.error = Some("roundtrip content differed".to_owned());
                        }
                    }
                    Err(error) => result.error = Some(sanitized_error(error)),
                }
            }
            Err(error) => result.error = Some(sanitized_error(error)),
        },
    }
    result
}

fn main() -> Result<()> {
    let args = Args::parse();
    println!("{}", serde_json::to_string_pretty(&probe(&args))?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::sanitized_error;

    #[test]
    fn error_text_cannot_inject_terminal_control_characters() {
        // Arrange + Act
        let value = sanitized_error("failure\n\u{1b}[31mred");

        // Assert
        assert_eq!(value, "failure[31mred");
    }

    #[test]
    fn error_text_has_a_hard_length_limit() {
        // Arrange + Act
        let value = sanitized_error("x".repeat(500));

        // Assert
        assert_eq!(value.len(), 300);
    }
}
