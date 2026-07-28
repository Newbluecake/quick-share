//! Human terminal rendering with control-character sanitization and explicit confirmations.

use crate::{AppError, InteractionPolicy};
use quick_share_discovery::UnverifiedPeer;
use quick_share_transfer::{offer::OfferView, sender::ProgressEvent};
use std::io::{self, IsTerminal, Write};

use crate::orchestration::{OfferChoice, ReceiveStartup, ReceiveTerminal, SendMode, SendTerminal};

#[derive(Debug, Clone, Copy)]
pub struct ConsoleTerminal {
    interactive: bool,
}

impl Default for ConsoleTerminal {
    fn default() -> Self {
        Self {
            interactive: io::stdin().is_terminal() && io::stderr().is_terminal(),
        }
    }
}

impl ConsoleTerminal {
    #[must_use]
    pub const fn new(interactive: bool) -> Self {
        Self { interactive }
    }

    #[must_use]
    pub const fn is_interactive(self) -> bool {
        self.interactive
    }

    pub fn confirm_update(
        self,
        current: &semver::Version,
        next: &semver::Version,
        assume_yes: bool,
    ) -> Result<(), AppError> {
        eprintln!("Update available: {current} -> {next}");
        if assume_yes {
            return Ok(());
        }
        if !self.interactive {
            return Err(AppError::ConfirmationRequired(
                "installing an update non-interactively requires --yes".to_owned(),
            ));
        }
        eprint!("Download, verify, and install this signed release? [y/N] ");
        io::stderr().flush().map_err(map_io)?;
        match read_line()?.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => Ok(()),
            _ => Err(AppError::Cancelled),
        }
    }

    pub fn confirm_tofu(
        self,
        peer_name: &str,
        device_id: &quick_share_protocol::DeviceId,
        sas: quick_share_transfer::noise::SasCode,
        assume_yes: bool,
    ) -> Result<(), AppError> {
        eprintln!(
            "Authenticated receiver: {} ({device_id}), SAS {sas}",
            terminal_safe(peer_name)
        );
        if !self.interactive {
            return if assume_yes {
                eprintln!("Proceeding with explicit --yes under TOFU; the SAS was not compared.");
                Ok(())
            } else {
                Err(AppError::ConfirmationRequired(
                    "unknown receiver requires interactive SAS review or --yes for explicit TOFU"
                        .to_owned(),
                ))
            };
        }
        eprint!("Compare the SAS if possible. Continue with this authenticated receiver? [y/N] ");
        io::stderr().flush().map_err(map_io)?;
        match read_line()?.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => Ok(()),
            _ => Err(AppError::Cancelled),
        }
    }
}

impl SendTerminal for ConsoleTerminal {
    fn warning(&self, message: &str) {
        eprintln!("warning: {}", terminal_safe(message));
    }

    fn mode(&self, mode: SendMode) {
        match mode {
            SendMode::DirectEncrypted => {
                eprintln!("Mode: encrypted device-to-device transfer (Noise XX)");
            }
            SendMode::WebHttps => {
                eprintln!("Mode: traditional Web sharing over HTTPS");
                eprintln!("The default self-signed certificate may trigger a browser warning.");
            }
            SendMode::WebHttpExplicit => {
                eprintln!(
                    "WARNING: traditional Web sharing over explicitly enabled plaintext HTTP"
                );
            }
        }
    }

    fn select_peer(
        &self,
        peers: &[UnverifiedPeer],
        interaction: InteractionPolicy,
    ) -> Result<usize, AppError> {
        if peers.is_empty() {
            return Err(AppError::PeerUnavailable(
                "no receiver is available".to_owned(),
            ));
        }
        if peers.len() == 1 {
            eprintln!(
                "Using the only available peer: {} ({}) [{}]",
                terminal_safe(&peers[0].name),
                peers[0].device_id,
                endpoint_summary(&peers[0])
            );
            return Ok(0);
        }
        interaction.require_confirmation("select a receiving device")?;
        if !self.interactive {
            return Err(AppError::Usage(
                "multiple receivers require an interactive selection or explicit --peer".to_owned(),
            ));
        }
        for (index, peer) in peers.iter().enumerate() {
            eprintln!(
                "  {}) {} ({}) [{}]",
                index + 1,
                terminal_safe(&peer.name),
                peer.device_id,
                endpoint_summary(peer)
            );
        }
        eprint!("Select receiver [1-{}], or 0 to cancel: ", peers.len());
        io::stderr().flush().map_err(map_io)?;
        let selected = read_line()?
            .trim()
            .parse::<usize>()
            .map_err(|_| AppError::Usage("receiver selection must be a number".to_owned()))?;
        if selected == 0 {
            return Err(AppError::Cancelled);
        }
        selected
            .checked_sub(1)
            .filter(|index| *index < peers.len())
            .ok_or_else(|| AppError::Usage("receiver selection is outside the list".to_owned()))
    }
}

impl ReceiveTerminal for ConsoleTerminal {
    fn startup(&self, startup: &ReceiveStartup) {
        eprintln!("Quick Share receiver is ready.");
        eprintln!("  Device: {}", terminal_safe(&startup.device_name));
        eprintln!("  Listen: {}", terminal_safe(&startup.bind_description));
        eprintln!(
            "  Output: {}",
            terminal_safe(&startup.output.display().to_string())
        );
        eprintln!(
            "  Security: {}",
            if startup.encrypted {
                "encrypted Noise XX direct transfer"
            } else {
                "UNENCRYPTED"
            }
        );
        eprintln!(
            "  Trusted devices: {}",
            if startup.trusted_auto_accept {
                "automatic acceptance within limits"
            } else {
                "confirmation required"
            }
        );
        eprintln!("Press Ctrl+C once for a graceful snapshot, twice to force stop.");
    }

    fn offer(&self, view: &OfferView) {
        eprintln!("Incoming transfer request:");
        eprintln!(
            "  Sender: {} ({})",
            terminal_safe(&view.sender_name),
            view.sender_device_id
        );
        if let Some(address) = view.peer_address {
            eprintln!("  Address: {address}");
        }
        eprintln!("  SAS: {}", view.sas);
        eprintln!(
            "  Entries: {}, bytes: {}",
            view.entry_count, view.total_bytes
        );
        eprintln!(
            "  Identity: {}",
            if view.identity_changed {
                "CHANGED — do not trust without re-verifying"
            } else if view.trusted {
                "trusted full static key"
            } else {
                "unknown / TOFU until SAS is compared"
            }
        );
        const MAX_DISPLAYED_OFFER_ENTRIES: usize = 100;
        for entry in view.entries.iter().take(MAX_DISPLAYED_OFFER_ENTRIES) {
            eprintln!(
                "    {}  {}  {} bytes",
                entry.kind,
                terminal_safe(&entry.relative_path),
                entry.size
            );
        }
        if view.entries.len() > MAX_DISPLAYED_OFFER_ENTRIES {
            eprintln!(
                "    ... {} additional entries omitted from terminal display",
                view.entries.len() - MAX_DISPLAYED_OFFER_ENTRIES
            );
        }
    }

    fn choose_offer(&self, view: &OfferView) -> Result<OfferChoice, AppError> {
        if !self.interactive {
            return Ok(OfferChoice::Reject);
        }
        eprint!("Accept [o]nce, accept and [t]rust after SAS comparison, or [r]eject? ");
        io::stderr().flush().map_err(map_io)?;
        match read_line()?.trim().to_ascii_lowercase().as_str() {
            "o" | "once" => Ok(OfferChoice::AcceptOnce),
            "t" | "trust" => {
                eprint!("Type the sender's matching six-digit SAS to confirm: ");
                io::stderr().flush().map_err(map_io)?;
                let typed = read_line()?;
                Ok(OfferChoice::AcceptAndTrust {
                    sas_verified: typed.trim() == view.sas.to_string(),
                })
            }
            "r" | "reject" | "" => Ok(OfferChoice::Reject),
            _ => Err(AppError::Usage("unknown offer decision".to_owned())),
        }
    }

    fn progress(&self, event: ProgressEvent) {
        let percent = if event.total_bytes == 0 {
            100.0
        } else {
            event.current_bytes as f64 * 100.0 / event.total_bytes as f64
        };
        eprintln!(
            "Transfer: {}/{} bytes ({percent:.1}%) {:.0} B/s ETA {}",
            event.current_bytes,
            event.total_bytes,
            event.bytes_per_second,
            event
                .eta
                .map_or_else(|| "--".to_owned(), |eta| format!("{}s", eta.as_secs()))
        );
    }

    fn warning(&self, message: &str) {
        eprintln!("warning: {}", terminal_safe(message));
    }
}

fn endpoint_summary(peer: &UnverifiedPeer) -> String {
    peer.endpoints
        .iter()
        .next()
        .map_or_else(|| "no endpoint".to_owned(), ToString::to_string)
}

fn read_line() -> Result<String, AppError> {
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).map_err(map_io)?;
    Ok(answer)
}

fn map_io(error: io::Error) -> AppError {
    AppError::Filesystem(error.to_string())
}

#[must_use]
pub fn terminal_safe(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars().take(500) {
        if character.is_control() {
            use std::fmt::Write as _;
            let _ = write!(output, "\\u{{{:x}}}", character as u32);
        } else {
            output.push(character);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{ConsoleTerminal, terminal_safe};
    use crate::{InteractionPolicy, orchestration::SendTerminal};
    use quick_share_discovery::UnverifiedPeer;
    use quick_share_protocol::{Capability, DeviceId, ProtocolVersion};
    use std::{collections::BTreeSet, net::SocketAddr};

    #[test]
    fn terminal_text_escapes_controls_instead_of_emitting_ansi() {
        let safe = terminal_safe("name\n\u{1b}[31m");
        assert_eq!(safe, "name\\u{a}\\u{1b}[31m");
        assert!(!safe.contains('\u{1b}'));
    }

    #[test]
    fn the_only_available_peer_is_selected_without_confirmation() {
        let peer = UnverifiedPeer {
            device_id: DeviceId::parse("qs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").expect("device id"),
            name: "configured-peer".to_owned(),
            version: ProtocolVersion::V1_1,
            capabilities: BTreeSet::from([Capability::Files]),
            static_key_fingerprint: [1; 32],
            endpoints: BTreeSet::from([SocketAddr::from(([192, 0, 2, 1], 4242))]),
            unverified: true,
        };
        let selected = SendTerminal::select_peer(
            &ConsoleTerminal::new(false),
            &[peer],
            InteractionPolicy::new(false, false),
        )
        .expect("single peer is routing, not trust confirmation");
        assert_eq!(selected, 0);
    }

    #[test]
    fn non_interactive_update_install_requires_yes() {
        let current = semver::Version::parse("2.0.0-alpha.0").expect("current");
        let next = semver::Version::parse("2.0.0").expect("next");
        assert!(
            ConsoleTerminal::new(false)
                .confirm_update(&current, &next, false)
                .is_err()
        );
        ConsoleTerminal::new(false)
            .confirm_update(&current, &next, true)
            .expect("explicit --yes");
    }
}
