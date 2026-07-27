//! Windows subprocess text decoding at the operating-system boundary.
//!
//! Product data remains UTF-8. This adapter exists only for legacy Windows
//! command output that may still use UTF-16LE or the system GBK code page.

use thiserror::Error;

pub fn decode_windows_command_output(bytes: &[u8]) -> Result<String, WindowsTextError> {
    if bytes.is_empty() {
        return Ok(String::new());
    }
    if let Ok(value) = std::str::from_utf8(bytes) {
        return Ok(value.to_owned());
    }
    if bytes.starts_with(&[0xff, 0xfe]) {
        return decode_utf16_le(&bytes[2..]);
    }
    if looks_like_utf16_le(bytes) {
        return decode_utf16_le(bytes);
    }
    let (decoded, _, malformed) = encoding_rs::GBK.decode(bytes);
    if malformed {
        return Err(WindowsTextError::InvalidEncoding);
    }
    Ok(decoded.into_owned())
}

fn looks_like_utf16_le(bytes: &[u8]) -> bool {
    if !bytes.len().is_multiple_of(2) || bytes.len() < 4 {
        return false;
    }
    let odd_bytes = bytes.iter().skip(1).step_by(2);
    let count = bytes.len() / 2;
    odd_bytes.filter(|byte| **byte == 0).count() * 2 >= count
}

fn decode_utf16_le(bytes: &[u8]) -> Result<String, WindowsTextError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(WindowsTextError::InvalidEncoding);
    }
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]));
    std::char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| WindowsTextError::InvalidEncoding)
}

#[derive(Debug, Error)]
pub enum WindowsTextError {
    #[error("Windows command output is not valid UTF-8, UTF-16LE, or GBK text")]
    InvalidEncoding,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_utf16_and_gbk_are_normalized_to_utf8() {
        let expected = "公共网络 UTF-8";
        assert_eq!(
            decode_windows_command_output(expected.as_bytes()).expect("UTF-8"),
            expected
        );

        let utf16 = expected
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        assert_eq!(
            decode_windows_command_output(&utf16).expect("UTF-16LE"),
            expected
        );

        let gbk_expected = "公共网络";
        let (gbk, _, malformed) = encoding_rs::GBK.encode(gbk_expected);
        assert!(!malformed);
        assert_eq!(
            decode_windows_command_output(&gbk).expect("GBK"),
            gbk_expected
        );
    }

    #[test]
    fn malformed_utf16_and_unmappable_gbk_fail_closed() {
        assert!(decode_windows_command_output(&[0xff, 0xfe, 0x00]).is_err());
        assert!(decode_windows_command_output(&[0x81]).is_err());
    }
}
