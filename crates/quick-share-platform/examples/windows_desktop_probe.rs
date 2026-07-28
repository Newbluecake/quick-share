#![forbid(unsafe_code)]

#[cfg(not(windows))]
fn main() {
    eprintln!("windows_desktop_probe is only available on Windows");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use quick_share_platform::desktop::{
        DesktopInteraction, SourceDialog, windows::windows_desktop,
    };
    use std::{thread, time::Duration};

    let (desktop, _control, tray_events, runtime) = windows_desktop(Duration::from_secs(120))?;
    let _worker = thread::spawn(move || {
        thread::sleep(Duration::from_millis(750));
        loop {
            let result = desktop.choose_send_source(&SourceDialog {
                requester_name: "Quick Share true-host test".to_owned(),
            });
            println!("production desktop result: {result:?}");
            match tray_events.recv() {
                Ok(quick_share_platform::desktop::windows::TrayAction::Open) => {}
                Ok(quick_share_platform::desktop::windows::TrayAction::Exit) | Err(_) => break,
            }
        }
    });
    runtime.run()?;
    Ok(())
}
