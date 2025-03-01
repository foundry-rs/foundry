use crate::util::crc8;
use crc_catalog::Algorithm;

mod bytewise;
mod nolookup;
mod slice16;

const fn init(algorithm: &Algorithm<u8>, initial: u8) -> u8 {
    if algorithm.refin {
        initial.reverse_bits() >> (8u8 - algorithm.width)
    } else {
        initial << (8u8 - algorithm.width)
    }
}

const fn finalize(algorithm: &Algorithm<u8>, mut crc: u8) -> u8 {
    if algorithm.refin ^ algorithm.refout {
        crc = crc.reverse_bits();
    }
    if !algorithm.refout {
        crc >>= 8u8 - algorithm.width;
    }
    crc ^ algorithm.xorout
}

const fn update_nolookup(mut crc: u8, algorithm: &Algorithm<u8>, bytes: &[u8]) -> u8 {
    let poly = if algorithm.refin {
        let poly = algorithm.poly.reverse_bits();
        poly >> (8u8 - algorithm.width)
    } else {
        algorithm.poly << (8u8 - algorithm.width)
    };

    let mut i = 0;

    while i < bytes.len() {
        crc = crc8(poly, algorithm.refin, crc ^ bytes[i]);
        i += 1;
    }

    crc
}

const fn update_bytewise(mut crc: u8, table: &[u8; 256], bytes: &[u8]) -> u8 {
    let mut i = 0;

    while i < bytes.len() {
        crc = table[(crc ^ bytes[i]) as usize];
        i += 1;
    }

    crc
}

const fn update_slice16(mut crc: u8, table: &[[u8; 256]; 16], bytes: &[u8]) -> u8 {
    let len = bytes.len();
    let mut i = 0;

    while i + 16 <= len {
        crc = table[0][bytes[i + 15] as usize]
            ^ table[1][bytes[i + 14] as usize]
            ^ table[2][bytes[i + 13] as usize]
            ^ table[3][bytes[i + 12] as usize]
            ^ table[4][bytes[i + 11] as usize]
            ^ table[5][bytes[i + 10] as usize]
            ^ table[6][bytes[i + 9] as usize]
            ^ table[7][bytes[i + 8] as usize]
            ^ table[8][bytes[i + 7] as usize]
            ^ table[9][bytes[i + 6] as usize]
            ^ table[10][bytes[i + 5] as usize]
            ^ table[11][bytes[i + 4] as usize]
            ^ table[12][bytes[i + 3] as usize]
            ^ table[13][bytes[i + 2] as usize]
            ^ table[14][bytes[i + 1] as usize]
            ^ table[15][(bytes[i] ^ crc) as usize];

        i += 16;
    }

    while i < len {
        crc = table[0][(crc ^ bytes[i]) as usize];
        i += 1;
    }

    crc
}

#[cfg(test)]
mod test {
    use crate::*;
    use crc_catalog::{Algorithm, CRC_8_BLUETOOTH};

    /// Test this optimized version against the well known implementation to ensure correctness
    #[test]
    fn correctness() {
        let data: &[&str] = &[
            "",
            "1",
            "1234",
            "123456789",
            "0123456789ABCDE",
            "01234567890ABCDEFGHIJK",
            "01234567890ABCDEFGHIJK01234567890ABCDEFGHIJK01234567890ABCDEFGHIJK01234567890ABCDEFGHIJK01234567890ABCDEFGHIJK01234567890ABCDEFGHIJK01234567890ABCDEFGHIJK01234567890ABCDEFGHIJK01234567890ABCDEFGHIJK01234567890ABCDEFGHIJK01234567890ABCDEFGHIJK01234567890ABCDEFGHIJK",
        ];

        pub const CRC_8_BLUETOOTH_NONREFLEX: Algorithm<u8> = Algorithm {
            width: 8,
            poly: 0xa7,
            init: 0x00,
            refin: false,
            refout: true,
            xorout: 0x00,
            check: 0x26,
            residue: 0x00,
        };

        let algs_to_test = [&CRC_8_BLUETOOTH, &CRC_8_BLUETOOTH_NONREFLEX];

        for alg in algs_to_test {
            for data in data {
                let crc_slice16 = Crc::<u8, Table<16>>::new(alg);
                let crc_nolookup = Crc::<u8, NoTable>::new(alg);
                let expected = Crc::<u8, Table<1>>::new(alg).checksum(data.as_bytes());

                // Check that doing all at once works as expected
                assert_eq!(crc_slice16.checksum(data.as_bytes()), expected);
                assert_eq!(crc_nolookup.checksum(data.as_bytes()), expected);

                let mut digest = crc_slice16.digest();
                digest.update(data.as_bytes());
                assert_eq!(digest.finalize(), expected);

                let mut digest = crc_nolookup.digest();
                digest.update(data.as_bytes());
                assert_eq!(digest.finalize(), expected);

                // Check that we didn't break updating from multiple sources
                if data.len() > 2 {
                    let data = data.as_bytes();
                    let data1 = &data[..data.len() / 2];
                    let data2 = &data[data.len() / 2..];
                    let mut digest = crc_slice16.digest();
                    digest.update(data1);
                    digest.update(data2);
                    assert_eq!(digest.finalize(), expected);
                    let mut digest = crc_nolookup.digest();
                    digest.update(data1);
                    digest.update(data2);
                    assert_eq!(digest.finalize(), expected);
                }
            }
        }
    }
}
