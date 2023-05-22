use ethereum_forkid::{ForkHash, ForkId};
use ethers::types::BlockNumber;
use foundry_evm::revm::primitives::SpecId;
use std::str::FromStr;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum Hardfork {
    Frontier,
    Homestead,
    Dao,
    Tangerine,
    SpuriousDragon,
    Byzantium,
    Constantinople,
    Petersburg,
    Istanbul,
    Muirglacier,
    Berlin,
    London,
    ArrowGlacier,
    GrayGlacier,
    Paris,
    Shanghai,
    #[default]
    Latest,
}

impl Hardfork {
    /// Get the first block number of the hardfork.
    pub fn fork_block(&self) -> u64 {
        match *self {
            Hardfork::Frontier => 0,
            Hardfork::Homestead => 1150000,
            Hardfork::Dao => 1920000,
            Hardfork::Tangerine => 2463000,
            Hardfork::SpuriousDragon => 2675000,
            Hardfork::Byzantium => 4370000,
            Hardfork::Constantinople | Hardfork::Petersburg => 7280000,
            Hardfork::Istanbul => 9069000,
            Hardfork::Muirglacier => 9200000,
            Hardfork::Berlin => 12244000,
            Hardfork::London => 12965000,
            Hardfork::ArrowGlacier => 13773000,
            Hardfork::GrayGlacier => 15050000,
            Hardfork::Paris => 15537394,
            Hardfork::Shanghai | Hardfork::Latest => 17034870,
        }
    }

    /// Get the EIP-2124 fork id for a given hardfork
    ///
    /// The [`ForkId`](ethereum_forkid::ForkId) includes a CRC32 checksum of the all fork block
    /// numbers from genesis, and the next upcoming fork block number.
    /// If the next fork block number is not yet known, it is set to 0.
    pub fn fork_id(&self) -> ForkId {
        match *self {
            Hardfork::Frontier => {
                ForkId { hash: ForkHash([0xfc, 0x64, 0xec, 0x04]), next: 1150000 }
            }
            Hardfork::Homestead => {
                ForkId { hash: ForkHash([0x97, 0xc2, 0xc3, 0x4c]), next: 1920000 }
            }
            Hardfork::Dao => ForkId { hash: ForkHash([0x91, 0xd1, 0xf9, 0x48]), next: 2463000 },
            Hardfork::Tangerine => {
                ForkId { hash: ForkHash([0x7a, 0x64, 0xda, 0x13]), next: 2675000 }
            }
            Hardfork::SpuriousDragon => {
                ForkId { hash: ForkHash([0x3e, 0xdd, 0x5b, 0x10]), next: 4370000 }
            }
            Hardfork::Byzantium => {
                ForkId { hash: ForkHash([0xa0, 0x0b, 0xc3, 0x24]), next: 7280000 }
            }
            Hardfork::Constantinople | Hardfork::Petersburg => {
                ForkId { hash: ForkHash([0x66, 0x8d, 0xb0, 0xaf]), next: 9069000 }
            }
            Hardfork::Istanbul => {
                ForkId { hash: ForkHash([0x87, 0x9d, 0x6e, 0x30]), next: 9200000 }
            }
            Hardfork::Muirglacier => {
                ForkId { hash: ForkHash([0xe0, 0x29, 0xe9, 0x91]), next: 12244000 }
            }
            Hardfork::Berlin => ForkId { hash: ForkHash([0x0e, 0xb4, 0x40, 0xf6]), next: 12965000 },
            Hardfork::London => ForkId { hash: ForkHash([0xb7, 0x15, 0x07, 0x7d]), next: 13773000 },
            Hardfork::ArrowGlacier => {
                ForkId { hash: ForkHash([0x20, 0xc3, 0x27, 0xfc]), next: 15050000 }
            }
            Hardfork::GrayGlacier => {
                ForkId { hash: ForkHash([0xf0, 0xaf, 0xd0, 0xe3]), next: 15537394 }
            }
            Hardfork::Paris => ForkId { hash: ForkHash([0x4f, 0xb8, 0xa8, 0x72]), next: 17034870 },
            Hardfork::Shanghai | Hardfork::Latest => {
                // update `next` when another fork block num is known
                ForkId { hash: ForkHash([0xc1, 0xfd, 0xf1, 0x81]), next: 0 }
            }
        }
    }
}

impl FromStr for Hardfork {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_lowercase();
        let hardfork = match s.as_str() {
            "frontier" | "1" => Hardfork::Frontier,
            "homestead" | "2" => Hardfork::Homestead,
            "dao" | "3" => Hardfork::Dao,
            "tangerine" | "4" => Hardfork::Tangerine,
            "spuriousdragon" | "5" => Hardfork::SpuriousDragon,
            "byzantium" | "6" => Hardfork::Byzantium,
            "constantinople" | "7" => Hardfork::Constantinople,
            "petersburg" | "8" => Hardfork::Petersburg,
            "istanbul" | "9" => Hardfork::Istanbul,
            "muirglacier" | "10" => Hardfork::Muirglacier,
            "berlin" | "11" => Hardfork::Berlin,
            "london" | "12" => Hardfork::London,
            "arrowglacier" | "13" => Hardfork::ArrowGlacier,
            "grayglacier" => Hardfork::GrayGlacier,
            "latest" | "14" => Hardfork::Latest,
            _ => return Err(format!("Unknown hardfork {s}")),
        };
        Ok(hardfork)
    }
}

impl From<Hardfork> for SpecId {
    fn from(fork: Hardfork) -> Self {
        match fork {
            Hardfork::Frontier => SpecId::FRONTIER,
            Hardfork::Homestead => SpecId::HOMESTEAD,
            Hardfork::Dao => SpecId::HOMESTEAD,
            Hardfork::Tangerine => SpecId::TANGERINE,
            Hardfork::SpuriousDragon => SpecId::SPURIOUS_DRAGON,
            Hardfork::Byzantium => SpecId::BYZANTIUM,
            Hardfork::Constantinople => SpecId::CONSTANTINOPLE,
            Hardfork::Petersburg => SpecId::PETERSBURG,
            Hardfork::Istanbul => SpecId::ISTANBUL,
            Hardfork::Muirglacier => SpecId::MUIR_GLACIER,
            Hardfork::Berlin => SpecId::BERLIN,
            Hardfork::London => SpecId::LONDON,
            Hardfork::ArrowGlacier => SpecId::LONDON,
            Hardfork::GrayGlacier => SpecId::GRAY_GLACIER,
            Hardfork::Paris => SpecId::MERGE,
            Hardfork::Shanghai | Hardfork::Latest => SpecId::SHANGHAI,
        }
    }
}

impl<T: Into<BlockNumber>> From<T> for Hardfork {
    fn from(block: T) -> Hardfork {
        let num = match block.into() {
            BlockNumber::Earliest => 0,
            BlockNumber::Number(num) => num.as_u64(),
            _ => u64::MAX,
        };

        match num {
            _i if num < 1_150_000 => Hardfork::Frontier,
            _i if num < 1_920_000 => Hardfork::Dao,
            _i if num < 2_463_000 => Hardfork::Homestead,
            _i if num < 2_675_000 => Hardfork::Tangerine,
            _i if num < 4_370_000 => Hardfork::SpuriousDragon,
            _i if num < 7_280_000 => Hardfork::Byzantium,
            _i if num < 9_069_000 => Hardfork::Constantinople,
            _i if num < 9_200_000 => Hardfork::Istanbul,
            _i if num < 12_244_000 => Hardfork::Muirglacier,
            _i if num < 12_965_000 => Hardfork::Berlin,
            _i if num < 13_773_000 => Hardfork::London,
            _i if num < 15_050_000 => Hardfork::ArrowGlacier,

            _ => Hardfork::Latest,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Hardfork;
    use crc::{Crc, CRC_32_ISO_HDLC};
    use ethers::utils::hex;

    #[test]
    fn test_hardfork_blocks() {
        let hf: Hardfork = 12_965_000u64.into();
        assert_eq!(hf, Hardfork::London);

        let hf: Hardfork = 4370000u64.into();
        assert_eq!(hf, Hardfork::Byzantium);

        let hf: Hardfork = 12244000u64.into();
        assert_eq!(hf, Hardfork::Berlin);
    }

    #[test]
    // this test checks that the fork hash assigned to forks accurately map to the fork_id method
    fn test_forkhash_from_fork_blocks() {
        // set the genesis hash
        let genesis =
            hex::decode("d4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3")
                .unwrap();

        // instantiate the crc "hasher"
        let crc_hasher = Crc::<u32>::new(&CRC_32_ISO_HDLC);
        let mut crc_digest = crc_hasher.digest();

        // check frontier forkhash
        crc_digest.update(&genesis);

        // now we go through enum members
        let frontier_forkid = Hardfork::Frontier.fork_id();
        let frontier_forkhash = u32::from_be_bytes(frontier_forkid.hash.0);
        // clone the digest for finalization so we can update it again
        assert_eq!(crc_digest.clone().finalize(), frontier_forkhash);

        // list of the above hardforks
        let hardforks = vec![
            Hardfork::Homestead,
            Hardfork::Dao,
            Hardfork::Tangerine,
            Hardfork::SpuriousDragon,
            Hardfork::Byzantium,
            Hardfork::Constantinople,
            Hardfork::Istanbul,
            Hardfork::Muirglacier,
            Hardfork::Berlin,
            Hardfork::London,
            Hardfork::ArrowGlacier,
            Hardfork::GrayGlacier,
        ];

        // now loop through each hardfork, conducting each forkhash test
        for hardfork in hardforks {
            // this could also be done with frontier_forkhash.next, but fork_block is used for more
            // coverage
            let fork_block = hardfork.fork_block().to_be_bytes();
            crc_digest.update(&fork_block);
            let fork_hash = u32::from_be_bytes(hardfork.fork_id().hash.0);
            assert_eq!(crc_digest.clone().finalize(), fork_hash);
        }
    }
}
