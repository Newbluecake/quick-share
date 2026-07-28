#![forbid(unsafe_code)]

fn report_error(error: quick_share_cli::AppError) -> ! {
    if !matches!(error, quick_share_cli::AppError::Cancelled) {
        eprintln!(
            "error: {}",
            quick_share_cli::terminal::terminal_safe(&error.to_string())
        );
    }
    std::process::exit(i32::from(error.exit_code()));
}

fn main() {
    let intent = match quick_share_cli::parse_intent_from(std::env::args_os()) {
        Ok(intent) => intent,
        Err(error) => error.exit(),
    };

    #[cfg(windows)]
    if matches!(&intent.command, quick_share_cli::IntentCommand::Agent(_)) {
        if let Err(error) = quick_share_cli::app::run_windows_agent(intent) {
            report_error(error);
        }
        return;
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => report_error(quick_share_cli::AppError::Config(error.to_string())),
    };
    if let Err(error) = runtime.block_on(quick_share_cli::app::run(intent)) {
        report_error(error);
    }
}
