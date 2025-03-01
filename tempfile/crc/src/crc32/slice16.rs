use crate::table::crc32_table_slice_16;
use crate::*;

use super::{finalize, init, update_slice16};

impl Crc<u32, Table<16>> {
    pub const fn new(algorithm: &'static Algorithm<u32>) -> Self {
        let data = crc32_table_slice_16(algorithm.width, algorithm.poly, algorithm.refin);
        Self { algorithm, data }
    }

    pub const fn checksum(&self, bytes: &[u8]) -> u32 {
        let mut crc = init(self.algorithm, self.algorithm.init);
        crc = self.update(crc, bytes);
        finalize(self.algorithm, crc)
    }

    const fn update(&self, crc: u32, bytes: &[u8]) -> u32 {
        update_slice16(crc, self.algorithm.refin, &self.data, bytes)
    }

    pub const fn digest(&self) -> Digest<u32, Table<16>> {
        self.digest_with_initial(self.algorithm.init)
    }

    /// Construct a `Digest` with a given initial value.
    ///
    /// This overrides the initial value specified by the algorithm.
    /// The effects of the algorithm's properties `refin` and `width`
    /// are applied to the custom initial value.
    pub const fn digest_with_initial(&self, initial: u32) -> Digest<u32, Table<16>> {
        let value = init(self.algorithm, initial);
        Digest::new(self, value)
    }

    pub const fn table(&self) -> &<Table<16> as Implementation>::Data<u32> {
        &self.data
    }
}

impl<'a> Digest<'a, u32, Table<16>> {
    const fn new(crc: &'a Crc<u32, Table<16>>, value: u32) -> Self {
        Digest { crc, value }
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.value = self.crc.update(self.value, bytes);
    }

    pub const fn finalize(self) -> u32 {
        finalize(self.crc.algorithm, self.value)
    }
}
