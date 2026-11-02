use crate::{Error, Result};
use alloc::vec::Vec;

pub trait BinaryCodec: Sized {
    /// Use the framework Error directly, or wrap it to retain business errors.
    type Error: From<Error>;

    fn encode(&self) -> Vec<u8>;

    fn decode(data: &[u8]) -> core::result::Result<Self, Self::Error>;
}

/// Bounded, allocation-free reader. Call finish() to reject trailing bytes.
pub struct Reader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    pub fn bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.offset.checked_add(len).ok_or(Error::Encoding)?;
        let value = self.data.get(self.offset..end).ok_or(Error::Encoding)?;
        self.offset = end;
        Ok(value)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.bytes(N)?.try_into().map_err(|_| Error::Encoding)
    }

    pub fn expect(&mut self, magic: &[u8]) -> Result<()> {
        if self.bytes(magic.len())? != magic {
            return Err(Error::Encoding);
        }
        Ok(())
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub fn u128(&mut self) -> Result<u128> {
        Ok(u128::from_le_bytes(self.array()?))
    }

    pub fn finish(self) -> Result<()> {
        if self.offset != self.data.len() {
            return Err(Error::Encoding);
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    pub fn new(magic: &[u8]) -> Self {
        Self(magic.to_vec())
    }

    pub fn bytes(&mut self, value: &[u8]) {
        self.0.extend_from_slice(value);
    }

    pub fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    pub fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    pub fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    pub fn u128(&mut self, value: u128) {
        self.bytes(&value.to_le_bytes());
    }

    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_rejects_truncation_trailing_bytes_and_length_overflow() {
        let mut writer = Writer::new(b"TEST");
        writer.u64(42);
        writer.u128(u128::MAX);
        let bytes = writer.finish();
        let mut reader = Reader::new(&bytes);
        reader.expect(b"TEST").unwrap();
        assert_eq!(reader.u64(), Ok(42));
        assert_eq!(reader.u128(), Ok(u128::MAX));
        assert_eq!(reader.finish(), Ok(()));
        assert_eq!(Reader::new(&bytes).finish(), Err(Error::Encoding));
        assert_eq!(Reader::new(&bytes[..7]).u64(), Err(Error::Encoding));
        let mut reader = Reader::new(&bytes);
        reader.u8().unwrap();
        assert_eq!(reader.bytes(usize::MAX), Err(Error::Encoding));
    }
}
