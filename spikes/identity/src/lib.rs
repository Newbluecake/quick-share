//! Disposable Noise XX feasibility experiment.
//! This is not production authentication code.

use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use snow::{Builder, HandshakeState, params::NoiseParams};

const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const MAX_HANDSHAKE_MESSAGE: usize = 65_535;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HandshakeEvidence {
    pub pattern: String,
    pub initiator_sas: String,
    pub responder_sas: String,
    pub initiator_remote_static: String,
    pub responder_remote_static: String,
    pub initiator_expected_remote_static: String,
    pub responder_expected_remote_static: String,
    pub pinning_succeeded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticKeypair {
    pub private: Vec<u8>,
    pub public: Vec<u8>,
}

pub fn generate_static_keypair() -> Result<StaticKeypair> {
    let params: NoiseParams = NOISE_PATTERN.parse().context("parse Noise pattern")?;
    let keypair = Builder::new(params)
        .generate_keypair()
        .context("generate Noise static keypair")?;
    Ok(StaticKeypair {
        private: keypair.private,
        public: keypair.public,
    })
}

pub fn build_initiator(keypair: &StaticKeypair) -> Result<HandshakeState> {
    let params: NoiseParams = NOISE_PATTERN.parse().context("parse Noise pattern")?;
    Builder::new(params)
        .local_private_key(&keypair.private)
        .context("set initiator static key")?
        .build_initiator()
        .context("build initiator")
}

pub fn build_responder(keypair: &StaticKeypair) -> Result<HandshakeState> {
    let params: NoiseParams = NOISE_PATTERN.parse().context("parse Noise pattern")?;
    Builder::new(params)
        .local_private_key(&keypair.private)
        .context("set responder static key")?
        .build_responder()
        .context("build responder")
}

fn pass_message(sender: &mut HandshakeState, receiver: &mut HandshakeState) -> Result<()> {
    let mut wire = vec![0_u8; MAX_HANDSHAKE_MESSAGE];
    let written = sender
        .write_message(&[], &mut wire)
        .context("write Noise handshake message")?;
    ensure!(
        written <= MAX_HANDSHAKE_MESSAGE,
        "handshake message exceeded bound"
    );

    let mut payload = vec![0_u8; MAX_HANDSHAKE_MESSAGE];
    let payload_len = receiver
        .read_message(&wire[..written], &mut payload)
        .context("read Noise handshake message")?;
    ensure!(payload_len == 0, "unexpected handshake payload");
    Ok(())
}

pub fn sas(handshake_hash: &[u8]) -> String {
    let digest = Sha256::digest(handshake_hash);
    let number = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    format!("{number:06}")
}

pub fn complete_xx(
    initiator_keys: &StaticKeypair,
    responder_keys: &StaticKeypair,
) -> Result<HandshakeEvidence> {
    let mut initiator = build_initiator(initiator_keys)?;
    let mut responder = build_responder(responder_keys)?;

    // Noise XX: -> e, <- e ee s es, -> s se
    pass_message(&mut initiator, &mut responder)?;
    pass_message(&mut responder, &mut initiator)?;
    pass_message(&mut initiator, &mut responder)?;

    ensure!(
        initiator.is_handshake_finished(),
        "initiator did not finish"
    );
    ensure!(
        responder.is_handshake_finished(),
        "responder did not finish"
    );

    let initiator_remote = initiator
        .get_remote_static()
        .ok_or_else(|| anyhow!("initiator did not learn responder static key"))?;
    let responder_remote = responder
        .get_remote_static()
        .ok_or_else(|| anyhow!("responder did not learn initiator static key"))?;

    let initiator_sas = sas(initiator.get_handshake_hash());
    let responder_sas = sas(responder.get_handshake_hash());
    let pinning_succeeded = initiator_remote == responder_keys.public
        && responder_remote == initiator_keys.public
        && initiator_sas == responder_sas;

    Ok(HandshakeEvidence {
        pattern: NOISE_PATTERN.to_owned(),
        initiator_sas,
        responder_sas,
        initiator_remote_static: hex::encode(initiator_remote),
        responder_remote_static: hex::encode(responder_remote),
        initiator_expected_remote_static: hex::encode(&responder_keys.public),
        responder_expected_remote_static: hex::encode(&initiator_keys.public),
        pinning_succeeded,
    })
}

/// A relay that terminates two independent handshakes cannot preserve the SAS.
/// This only detects the relay if the humans actually compare both codes.
pub fn mitm_sas_evidence() -> Result<(String, String)> {
    let initiator_keys = generate_static_keypair()?;
    let responder_keys = generate_static_keypair()?;
    let attacker_left = generate_static_keypair()?;
    let attacker_right = generate_static_keypair()?;

    let left = complete_xx(&initiator_keys, &attacker_left)?;
    let right = complete_xx(&attacker_right, &responder_keys)?;
    Ok((left.initiator_sas, right.responder_sas))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xx_handshake_derives_same_sas_and_remote_static_keys() {
        // Arrange
        let initiator = generate_static_keypair().expect("initiator keypair");
        let responder = generate_static_keypair().expect("responder keypair");

        // Act
        let evidence = complete_xx(&initiator, &responder).expect("Noise XX handshake");

        // Assert
        assert_eq!(evidence.initiator_sas, evidence.responder_sas);
        assert_eq!(
            evidence.initiator_remote_static,
            hex::encode(responder.public)
        );
        assert_eq!(
            evidence.responder_remote_static,
            hex::encode(initiator.public)
        );
        assert!(evidence.pinning_succeeded);
    }

    #[test]
    fn replacing_responder_identity_breaks_existing_pin() {
        // Arrange
        let initiator = generate_static_keypair().expect("initiator keypair");
        let original = generate_static_keypair().expect("original responder");
        let replacement = generate_static_keypair().expect("replacement responder");
        let original_evidence = complete_xx(&initiator, &original).expect("original handshake");

        // Act
        let replacement_evidence =
            complete_xx(&initiator, &replacement).expect("replacement handshake");

        // Assert
        assert_ne!(
            original_evidence.initiator_remote_static,
            replacement_evidence.initiator_remote_static
        );
    }

    #[test]
    fn terminating_mitm_produces_different_sas_on_the_two_real_devices() {
        // A random collision is possible with a six-digit display code. Repeat to make
        // this test deterministic enough while preserving the real collision property.
        for _ in 0..8 {
            let (initiator_sas, responder_sas) = mitm_sas_evidence().expect("MITM evidence");
            if initiator_sas != responder_sas {
                return;
            }
        }
        panic!("unexpected repeated six-digit SAS collisions");
    }
}
