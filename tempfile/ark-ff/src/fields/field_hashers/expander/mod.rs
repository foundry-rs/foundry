// The below implementation is a rework of https://github.com/armfazh/h2c-rust-ref
// With some optimisations

use ark_std::vec::Vec;
use digest::{DynDigest, ExtendableOutput, Update};
pub trait Expander {
    fn construct_dst_prime(&self) -> Vec<u8>;
    fn expand(&self, msg: &[u8], length: usize) -> Vec<u8>;
}
const MAX_DST_LENGTH: usize = 255;

const LONG_DST_PREFIX: [u8; 17] = [
    //'H', '2', 'C', '-', 'O', 'V', 'E', 'R', 'S', 'I', 'Z', 'E', '-', 'D', 'S', 'T', '-',
    0x48, 0x32, 0x43, 0x2d, 0x4f, 0x56, 0x45, 0x52, 0x53, 0x49, 0x5a, 0x45, 0x2d, 0x44, 0x53, 0x54,
    0x2d,
];

pub(super) struct ExpanderXof<T: Update + Clone + ExtendableOutput> {
    pub(super) xofer: T,
    pub(super) dst: Vec<u8>,
    pub(super) k: usize,
}

impl<T: Update + Clone + ExtendableOutput> Expander for ExpanderXof<T> {
    fn construct_dst_prime(&self) -> Vec<u8> {
        let mut dst_prime = if self.dst.len() > MAX_DST_LENGTH {
            let mut xofer = self.xofer.clone();
            xofer.update(&LONG_DST_PREFIX.clone());
            xofer.update(&self.dst);
            xofer.finalize_boxed((2 * self.k + 7) >> 3).to_vec()
        } else {
            self.dst.clone()
        };
        dst_prime.push(dst_prime.len() as u8);
        dst_prime
    }
    fn expand(&self, msg: &[u8], n: usize) -> Vec<u8> {
        let dst_prime = self.construct_dst_prime();
        let lib_str = &[((n >> 8) & 0xFF) as u8, (n & 0xFF) as u8];

        let mut xofer = self.xofer.clone();
        xofer.update(msg);
        xofer.update(lib_str);
        xofer.update(&dst_prime);
        xofer.finalize_boxed(n).to_vec()
    }
}

pub(super) struct ExpanderXmd<T: DynDigest + Clone> {
    pub(super) hasher: T,
    pub(super) dst: Vec<u8>,
    pub(super) block_size: usize,
}

impl<T: DynDigest + Clone> Expander for ExpanderXmd<T> {
    fn construct_dst_prime(&self) -> Vec<u8> {
        let mut dst_prime = if self.dst.len() > MAX_DST_LENGTH {
            let mut hasher = self.hasher.clone();
            hasher.update(&LONG_DST_PREFIX);
            hasher.update(&self.dst);
            hasher.finalize_reset().to_vec()
        } else {
            self.dst.clone()
        };
        dst_prime.push(dst_prime.len() as u8);
        dst_prime
    }
    fn expand(&self, msg: &[u8], n: usize) -> Vec<u8> {
        let mut hasher = self.hasher.clone();
        // output size of the hash function, e.g. 32 bytes = 256 bits for sha2::Sha256
        let b_len = hasher.output_size();
        let ell = (n + (b_len - 1)) / b_len;
        assert!(
            ell <= 255,
            "The ratio of desired output to the output size of hash function is too large!"
        );

        let dst_prime = self.construct_dst_prime();
        let z_pad: Vec<u8> = vec![0; self.block_size];
        // // Represent `len_in_bytes` as a 2-byte array.
        // // As per I2OSP method outlined in https://tools.ietf.org/pdf/rfc8017.pdf,
        // // The program should abort if integer that we're trying to convert is too large.
        assert!(n < (1 << 16), "Length should be smaller than 2^16");
        let lib_str: [u8; 2] = (n as u16).to_be_bytes();

        hasher.update(&z_pad);
        hasher.update(msg);
        hasher.update(&lib_str);
        hasher.update(&[0u8]);
        hasher.update(&dst_prime);
        let b0 = hasher.finalize_reset();

        hasher.update(&b0);
        hasher.update(&[1u8]);
        hasher.update(&dst_prime);
        let mut bi = hasher.finalize_reset();

        let mut uniform_bytes: Vec<u8> = Vec::with_capacity(n);
        uniform_bytes.extend_from_slice(&bi);
        for i in 2..=ell {
            // update the hasher with xor of b_0 and b_i elements
            for (l, r) in b0.iter().zip(bi.iter()) {
                hasher.update(&[*l ^ *r]);
            }
            hasher.update(&[i as u8]);
            hasher.update(&dst_prime);
            bi = hasher.finalize_reset();
            uniform_bytes.extend_from_slice(&bi);
        }
        uniform_bytes[0..n].to_vec()
    }
}

#[cfg(all(test, feature = "std"))]
mod tests;
