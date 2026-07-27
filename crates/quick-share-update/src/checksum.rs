use thiserror::Error;

pub fn expected_sha256(bytes: &[u8], asset_name: &str) -> Result<[u8; 32], ChecksumError> {
    if bytes.len() > super::MAX_CHECKSUMS_BYTES as usize {
        return Err(ChecksumError::TooLarge);
    }
    if asset_name.is_empty()
        || asset_name.contains('/')
        || asset_name.contains('\\')
        || asset_name.chars().any(char::is_control)
    {
        return Err(ChecksumError::InvalidAssetName);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ChecksumError::InvalidFormat)?;
    let mut found = None;
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let (digest, name) = line
            .split_once(char::is_whitespace)
            .ok_or(ChecksumError::InvalidFormat)?;
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ChecksumError::InvalidFormat);
        }
        let name = name
            .trim_start()
            .strip_prefix('*')
            .unwrap_or(name.trim_start());
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            return Err(ChecksumError::InvalidFormat);
        }
        if name == asset_name {
            if found.is_some() {
                return Err(ChecksumError::DuplicateAsset);
            }
            let decoded = hex::decode(digest).map_err(|_| ChecksumError::InvalidFormat)?;
            found = Some(
                decoded
                    .try_into()
                    .map_err(|_| ChecksumError::InvalidFormat)?,
            );
        }
    }
    found.ok_or(ChecksumError::MissingAsset)
}

#[derive(Debug, Error)]
pub enum ChecksumError {
    #[error("checksum manifest exceeds the hard size limit")]
    TooLarge,
    #[error("checksum manifest asset name is invalid")]
    InvalidAssetName,
    #[error("checksum manifest has an invalid format")]
    InvalidFormat,
    #[error("checksum manifest contains a duplicate asset")]
    DuplicateAsset,
    #[error("checksum manifest does not contain the requested asset")]
    MissingAsset,
}
