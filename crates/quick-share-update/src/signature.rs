use crate::{ReleaseSignatureVerifier, UpdateError};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

// Rotation procedure: ship a release signed by an existing key that adds the
// next public key here, then change the CI secret only after that release is
// broadly installed. Private signing material never belongs in the repository.
const PINNED_RELEASE_KEYS_HEX: [&str; 1] =
    ["7171b3af5765d9f41cc852410f4e38fc3ad6960739ad88d04e8ccedf2cebf10d"];

#[derive(Debug, Default)]
pub struct Ed25519ReleaseVerifier;

impl ReleaseSignatureVerifier for Ed25519ReleaseVerifier {
    fn verify(&self, checksums: &[u8], signature: &[u8]) -> Result<(), UpdateError> {
        let signature =
            Signature::from_slice(signature).map_err(|_| UpdateError::SignatureInvalid)?;
        for encoded in PINNED_RELEASE_KEYS_HEX {
            let decoded = hex::decode(encoded).map_err(|_| UpdateError::SignatureInvalid)?;
            let bytes: [u8; 32] = decoded
                .try_into()
                .map_err(|_| UpdateError::SignatureInvalid)?;
            let key =
                VerifyingKey::from_bytes(&bytes).map_err(|_| UpdateError::SignatureInvalid)?;
            if key.verify(checksums, &signature).is_ok() {
                return Ok(());
            }
        }
        Err(UpdateError::SignatureInvalid)
    }
}
