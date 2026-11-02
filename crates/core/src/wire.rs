//! Portable descriptions of strict, fixed-field little-endian encodings.
use crate::{BinaryCodec, Error, Reader, Result, Writer};
use alloc::vec::Vec;

pub type EncodedBytes = Vec<u8>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireType {
    U8,
    U32,
    U64,
    U128,
    Bool,
    Bytes(usize),
    Struct {
        name: &'static str,
        fields: &'static [WireField],
    },
    Enum {
        name: &'static str,
        variants: &'static [WireVariant],
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WireField {
    pub name: &'static str,
    pub ty: WireType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WireVariant {
    pub name: &'static str,
    pub tag: u8,
    pub fields: &'static [WireField],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodingSchema {
    pub prefix: &'static [u8],
    pub body: WireType,
}

pub trait CodecSchema: BinaryCodec {
    const SCHEMA: EncodingSchema;
}

/// Inline encoding excludes the type's top-level prefix. Composite fields use
/// this trait; BinaryCodec adds the prefix and checks for trailing bytes.
pub trait WireValue: Sized {
    const TYPE: WireType;
    fn write(&self, writer: &mut Writer);
    fn read(reader: &mut Reader<'_>) -> Result<Self>;
}

pub fn encode<T: WireValue>(value: &T, prefix: &[u8]) -> EncodedBytes {
    let mut writer = Writer::new(prefix);
    value.write(&mut writer);
    writer.finish()
}

pub fn decode<T: WireValue>(data: &[u8], prefix: &[u8]) -> Result<T> {
    let mut reader = Reader::new(data);
    reader.expect(prefix)?;
    let value = T::read(&mut reader)?;
    reader.finish()?;
    Ok(value)
}

macro_rules! integer {
    ($ty:ty, $kind:ident, $method:ident) => {
        impl WireValue for $ty {
            const TYPE: WireType = WireType::$kind;
            fn write(&self, writer: &mut Writer) {
                writer.$method(*self);
            }
            fn read(reader: &mut Reader<'_>) -> Result<Self> {
                reader.$method()
            }
        }
    };
}
integer!(u8, U8, u8);
integer!(u32, U32, u32);
integer!(u64, U64, u64);
integer!(u128, U128, u128);

impl WireValue for bool {
    const TYPE: WireType = WireType::Bool;
    fn write(&self, writer: &mut Writer) {
        writer.u8(u8::from(*self));
    }
    fn read(reader: &mut Reader<'_>) -> Result<Self> {
        match reader.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::Encoding),
        }
    }
}

impl<const N: usize> WireValue for [u8; N] {
    const TYPE: WireType = WireType::Bytes(N);
    fn write(&self, writer: &mut Writer) {
        writer.bytes(self);
    }
    fn read(reader: &mut Reader<'_>) -> Result<Self> {
        reader.array()
    }
}
