#![forbid(unsafe_code)]

#[tokio::main]
async fn main() {
    let intent = match quick_share_cli::parse_intent_from(std::env::args_os()) {
        Ok(intent) => intent,
        Err(error) => error.exit(),
    };
    if let Err(error) = quick_share_cli::app::run(intent).await {
        if !matches!(error, quick_share_cli::AppError::Cancelled) {
            eprintln!(
                "error: {}",
                quick_share_cli::terminal::terminal_safe(&error.to_string())
            );
        }
        std::process::exit(i32::from(error.exit_code()));
    }
}
