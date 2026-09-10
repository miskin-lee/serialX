//! The two checks the modem protocols close their blocks with.
//!
//! XMODEM-CRC, YMODEM and ZMODEM's 16-bit frames use CRC-16/XMODEM —
//! polynomial `0x1021`, nothing reflected, starting from zero. ZMODEM's
//! 32-bit frames use the CRC-32 that zip and Ethernet use: reflected,
//! starting from all ones and inverted at the end. Both are small enough
//! to compute bit by bit; a table would save time the port never gives us.

/// CRC-16/XMODEM of `bytes`, continuing from `crc`.
pub(super) fn crc16_update(mut crc: u16, bytes: &[u8]) -> u16 {
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

/// CRC-16/XMODEM of `bytes` on their own.
pub(super) fn crc16(bytes: &[u8]) -> u16 {
    crc16_update(0, bytes)
}

/// CRC-32 of `bytes`, continuing from a running value: start with
/// [`CRC32_INIT`] and finish with [`crc32_finish`].
pub(super) fn crc32_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    crc
}

pub(super) const CRC32_INIT: u32 = 0xFFFF_FFFF;

pub(super) fn crc32_finish(crc: u32) -> u32 {
    !crc
}

/// CRC-32 of `bytes` on their own.
pub(super) fn crc32(bytes: &[u8]) -> u32 {
    crc32_finish(crc32_update(CRC32_INIT, bytes))
}

/// The plain sum XMODEM used before it had a CRC: the low byte of the
/// block's bytes added up.
pub(super) fn checksum(bytes: &[u8]) -> u8 {
    bytes
        .iter()
        .fold(0_u8, |sum, &byte| sum.wrapping_add(byte))
}

#[cfg(test)]
mod tests {
    use super::{checksum, crc16, crc32};

    /// The check values every CRC catalogue lists for `123456789`.
    #[test]
    fn the_catalogue_values_come_out() {
        assert_eq!(crc16(b"123456789"), 0x31C3);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(checksum(b"123456789"), 0xDD);
    }

    #[test]
    fn nothing_checks_to_nothing() {
        assert_eq!(crc16(b""), 0);
        assert_eq!(crc32(b""), 0);
    }
}
