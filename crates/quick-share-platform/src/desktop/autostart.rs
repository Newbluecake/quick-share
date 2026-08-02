//! Windows per-user logon autostart registration for the resident agent.

use super::DesktopError;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "QuickShare";

/// Returns whether the agent is registered to launch at user logon.
pub(crate) fn is_enabled() -> bool {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey(RUN_KEY);
    match key {
        Ok(key) => key.get_value::<String, _>(VALUE_NAME).is_ok(),
        Err(_) => false,
    }
}

/// Registers or removes the agent logon autostart entry.
///
/// The entry points at the current executable with the `agent` subcommand so
/// in-place signed updates keep working without rewriting the registration.
pub(crate) fn set_enabled(enabled: bool) -> Result<(), DesktopError> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(RUN_KEY)
        .map_err(|_| DesktopError::Backend)?;
    if enabled {
        let executable = std::env::current_exe().map_err(|_| DesktopError::Backend)?;
        let command = format!("\"{}\" agent", executable.display());
        key.set_value(VALUE_NAME, &command)
            .map_err(|_| DesktopError::Backend)?;
    } else {
        // A missing value is already the disabled state.
        let _ = key.delete_value(VALUE_NAME);
    }
    Ok(())
}
