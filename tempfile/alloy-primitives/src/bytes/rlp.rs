use super::Bytes;
use alloy_rlp::{Decodable, Encodable};

impl Encodable for Bytes {
    #[inline]
    fn length(&self) -> usize {
        self.0.length()
    }

    #[inline]
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        self.0.encode(out);
    }
}

impl Decodable for Bytes {
    #[inline]
    fn decode(buf: &mut &[u8]) -> Result<Self, alloy_rlp::Error> {
        bytes::Bytes::decode(buf).map(Self)
    }
}
