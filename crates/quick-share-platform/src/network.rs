//! Read-only receive reachability diagnostics. This module never modifies firewall state.

use std::io;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkProfile {
    Public,
    Private,
    DomainAuthenticated,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveNetworkDiagnostic {
    pub profiles: Vec<NetworkProfile>,
    pub actionable_warning: Option<String>,
}

/// Detects Windows network category and emits guidance without changing firewall settings.
/// Other platforms return an empty diagnostic.
pub fn receive_network_diagnostic() -> Result<ReceiveNetworkDiagnostic, NetworkDiagnosticError> {
    platform_diagnostic()
}

#[cfg(windows)]
fn platform_diagnostic() -> Result<ReceiveNetworkDiagnostic, NetworkDiagnosticError> {
    use std::process::Command;

    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$utf8 = [System.Text.UTF8Encoding]::new($false); [Console]::OutputEncoding = $utf8; $OutputEncoding = $utf8; Get-NetConnectionProfile | ForEach-Object { $_.NetworkCategory }",
        ])
        .output()?;
    if !output.status.success() {
        return Err(NetworkDiagnosticError::CommandFailed);
    }
    let source = crate::encoding::decode_windows_command_output(&output.stdout)
        .map_err(|_| NetworkDiagnosticError::InvalidCommandOutput)?;
    Ok(parse_profiles(&source))
}

#[cfg(not(windows))]
fn platform_diagnostic() -> Result<ReceiveNetworkDiagnostic, NetworkDiagnosticError> {
    Ok(ReceiveNetworkDiagnostic {
        profiles: Vec::new(),
        actionable_warning: None,
    })
}

#[cfg(any(windows, test))]
fn parse_profiles(source: &str) -> ReceiveNetworkDiagnostic {
    let mut profiles = source
        .lines()
        .filter_map(|line| {
            let normalized = line.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                None
            } else {
                Some(match normalized.as_str() {
                    "public" => NetworkProfile::Public,
                    "private" => NetworkProfile::Private,
                    "domainauthenticated" | "domain_authenticated" => {
                        NetworkProfile::DomainAuthenticated
                    }
                    _ => NetworkProfile::Unknown,
                })
            }
        })
        .collect::<Vec<_>>();
    profiles.sort_by_key(|profile| match profile {
        NetworkProfile::Public => 0,
        NetworkProfile::Private => 1,
        NetworkProfile::DomainAuthenticated => 2,
        NetworkProfile::Unknown => 3,
    });
    profiles.dedup();
    let actionable_warning = profiles.contains(&NetworkProfile::Public).then(|| {
        "Windows reports a Public network profile. Inbound Quick Share may be blocked. Verify the network, then explicitly allow only the quick-share executable on the intended profile if you choose; Quick Share did not change the firewall."
            .to_owned()
    });
    ReceiveNetworkDiagnostic {
        profiles,
        actionable_warning,
    }
}

#[derive(Debug, Error)]
pub enum NetworkDiagnosticError {
    #[error("network profile diagnostic command failed")]
    CommandFailed,
    #[error("network profile diagnostic returned invalid text")]
    InvalidCommandOutput,
    #[error("network profile diagnostic could not start")]
    Io(#[from] io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_profile_warning_is_actionable_and_never_claims_to_modify_firewall() {
        let diagnostic = parse_profiles("Private\r\nPublic\r\nPublic\r\n");
        assert_eq!(
            diagnostic.profiles,
            vec![NetworkProfile::Public, NetworkProfile::Private]
        );
        let warning = diagnostic.actionable_warning.expect("warning");
        assert!(warning.contains("Public"));
        assert!(warning.contains("did not change the firewall"));
    }
}
