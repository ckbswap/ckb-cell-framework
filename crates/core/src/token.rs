use crate::{Error, Hash, Result};
use alloc::vec::Vec;

/// Descriptor for official data1 plain xUDT, preserving its 32/36-byte args
/// identity. Script construction and on-chain validation belong to the adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlainXudtAsset {
    pub owner: Hash,
    pub extended_args: bool,
}

impl PlainXudtAsset {
    pub fn args(&self) -> Vec<u8> {
        let mut args = self.owner.to_vec();
        if self.extended_args {
            args.extend_from_slice(&[0; 4]);
        }
        args
    }
}

impl crate::wire::WireValue for PlainXudtAsset {
    const TYPE: crate::wire::WireType = crate::wire::WireType::Struct {
        name: "PlainXudtAsset",
        fields: &[
            crate::wire::WireField {
                name: "owner",
                ty: crate::wire::WireType::Bytes(32),
            },
            crate::wire::WireField {
                name: "extended_args",
                ty: crate::wire::WireType::Bool,
            },
        ],
    };
    fn write(&self, writer: &mut crate::Writer) {
        writer.bytes(&self.owner);
        crate::wire::WireValue::write(&self.extended_args, writer);
    }
    fn read(reader: &mut crate::Reader<'_>) -> Result<Self> {
        Ok(Self {
            owner: reader.array()?,
            extended_args: <bool as crate::wire::WireValue>::read(reader)?,
        })
    }
}

/// Official binary data hashes, independent of network-specific code OutPoints.
pub const XUDT_CODE_HASH: Hash = [
    0x50, 0xbd, 0x8d, 0x66, 0x80, 0xb8, 0xb9, 0xcf, 0x98, 0xb7, 0x3f, 0x3c, 0x08, 0xfa, 0xf8, 0xb2,
    0xa2, 0x19, 0x14, 0x31, 0x19, 0x54, 0x11, 0x8a, 0xd6, 0x60, 0x9b, 0xe6, 0xe7, 0x8a, 0x1b, 0x95,
];
pub const SUDT_CODE_HASH: Hash = [
    0xe1, 0xe3, 0x54, 0xd6, 0xd6, 0x43, 0xad, 0x42, 0x72, 0x4d, 0x40, 0x96, 0x7e, 0x33, 0x49, 0x84,
    0x53, 0x4e, 0x03, 0x67, 0x40, 0x5c, 0x5a, 0xe4, 0x2a, 0x9d, 0x7d, 0x63, 0xd7, 0x7d, 0xf4, 0x19,
];

/// Explicitly choose whether extensions after the 16-byte amount are acceptable.
#[derive(Clone, Copy)]
pub enum AmountEncoding {
    Exact16,
    Prefix16,
}

pub fn decode_amount(data: &[u8], encoding: AmountEncoding) -> Result<u128> {
    if matches!(encoding, AmountEncoding::Exact16) && data.len() != 16 {
        return Err(Error::Token);
    }
    let bytes = data.get(..16).ok_or(Error::Token)?;
    Ok(u128::from_le_bytes(
        bytes.try_into().map_err(|_| Error::Token)?,
    ))
}

pub fn encode_amount(amount: u128) -> [u8; 16] {
    amount.to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_args_keep_32_and_36_byte_assets_distinct() {
        let mut asset = PlainXudtAsset {
            owner: [2; 32],
            extended_args: true,
        };
        assert_eq!(asset.args().len(), 36);
        assert_eq!(&asset.args()[32..], &[0; 4]);
        asset.extended_args = false;
        assert_eq!(asset.args(), asset.owner);
    }

    #[test]
    fn token_extensions_are_never_accepted_implicitly() {
        assert_eq!(
            decode_amount(&[0; 15], AmountEncoding::Prefix16),
            Err(Error::Token)
        );
        assert_eq!(
            decode_amount(&[0; 17], AmountEncoding::Exact16),
            Err(Error::Token)
        );
        assert_eq!(decode_amount(&[0; 17], AmountEncoding::Prefix16), Ok(0));
        assert_eq!(
            decode_amount(&encode_amount(u128::MAX), AmountEncoding::Exact16),
            Ok(u128::MAX)
        );
    }
}
