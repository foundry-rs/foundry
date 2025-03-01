use crate::table::crc64_table;
use crate::*;

use super::{finalize, init, update_bytewise};

impl Crc<u64, Table<1>> {
    pub const fn new(algorithm: &'static Algorithm<u64>) -> Self {
        let table = crc64_table(algorithm.width, algorithm.poly, algorithm.refin);
        Self {
            algorithm,
            data: [table],
        }
    }

    pub const fn checksum(&self, bytes: &[u8]) -> u64 {
        let mut crc = init(self.algorithm, self.algorithm.init);
        crc = self.update(crc, bytes);
        finalize(self.algorithm, crc)
    }

    const fn update(&self, crc: u64, bytes: &[u8]) -> u64 {
        update_bytewise(crc, self.algorithm.refin, &self.data[0], bytes)
    }

    pub const fn digest(&self) -> Digest<u64, Table<1>> {
        self.digest_with_initial(self.algorithm.init)
    }

    /// Construct a `Digest` with a given initial value.
    ///
    /// This overrides the initial value specified by the algorithm.
    /// The effects of the algorithm's properties `refin` and `width`
    /// are applied to the custom initial value.
    pub const fn digest_with_initial(&self, initial: u64) -> Digest<u64, Table<1>> {
        let value = init(self.algorithm, initial);
        Digest::new(self, value)
    }

    pub const fn table(&self) -> &<Table<1> as Implementation>::Data<u64> {
        &self.data
    }
}

impl<'a> Digest<'a, u64, Table<1>> {
    const fn new(crc: &'a Crc<u64, Table<1>>, value: u64) -> Self {
        Digest { crc, value }
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.value = self.crc.update(self.value, bytes);
    }

    pub const fn finalize(self) -> u64 {
        finalize(self.crc.algorithm, self.value)
    }
}
