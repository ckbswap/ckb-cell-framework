//! Canonical absolute timestamp since values. Timestamps are Unix seconds;
//! consensus compares milliseconds, so the conversion must also fit u64.
use crate::{Error, Result};

pub const MAX_TIMESTAMP: u64 = u64::MAX / 1000;
const TIMESTAMP_FLAG: u64 = 0x4000_0000_0000_0000;
const VALUE_MASK: u64 = 0x00ff_ffff_ffff_ffff;

pub fn absolute_timestamp(seconds: u64) -> Result<u64> {
    if seconds > MAX_TIMESTAMP {
        return Err(Error::Encoding);
    }
    Ok(TIMESTAMP_FLAG | seconds)
}

pub fn decode_absolute_timestamp(since: u64) -> Result<u64> {
    let seconds = since & VALUE_MASK;
    if absolute_timestamp(seconds)? != since {
        return Err(Error::Encoding);
    }
    Ok(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_encoding_is_exact_and_checks_consensus_conversion_overflow() {
        for seconds in [0, 1, 1_700_000_000, MAX_TIMESTAMP] {
            let since = absolute_timestamp(seconds).unwrap();
            assert_eq!(since, TIMESTAMP_FLAG | seconds);
            assert_eq!(decode_absolute_timestamp(since), Ok(seconds));
        }
        assert_eq!(absolute_timestamp(MAX_TIMESTAMP + 1), Err(Error::Encoding));
        assert_eq!(absolute_timestamp(u64::MAX), Err(Error::Encoding));
        assert_eq!(
            decode_absolute_timestamp(TIMESTAMP_FLAG | (MAX_TIMESTAMP + 1)),
            Err(Error::Encoding)
        );
    }

    #[test]
    fn rejects_relative_other_metrics_and_every_reserved_flag() {
        for flags in [0, 0x20, 0x60, 0x80, 0xc0] {
            assert_eq!(
                decode_absolute_timestamp((flags << 56) | 1000),
                Err(Error::Encoding)
            );
        }
        for bit in 56..61 {
            assert_eq!(
                decode_absolute_timestamp(TIMESTAMP_FLAG | (1 << bit) | 1000),
                Err(Error::Encoding)
            );
        }
    }
}
