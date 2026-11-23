//! Checksums used by on-disk formats.

/// CRC-32 as used by GPT partition tables (IEEE 802.3, reflected, initial
/// value and final XOR 0xFFFFFFFF).
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// CRC-16 as used by UDF descriptor tags (ITU-T V.41: polynomial 0x1021,
/// initial value 0, not reflected).
pub(crate) fn crc16_itu(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in bytes {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::{crc16_itu, crc32};

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn crc16_matches_the_standard_check_value() {
        // CRC-16/XMODEM, the same algorithm UDF uses.
        assert_eq!(crc16_itu(b"123456789"), 0x31C3);
        assert_eq!(crc16_itu(b""), 0);
    }
}
