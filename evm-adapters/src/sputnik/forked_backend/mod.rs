pub mod cache;
pub use cache::{new_shared_cache, MemCache, SharedBackend, SharedCache};
pub mod rpc;
pub use rpc::ForkMemoryBackend;

#[derive(thiserror::Error, Debug)]
pub enum ForkError {
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("no cache file was specified")]
    NoCache,
}

use ethers::types::{BlockId, BlockNumber, H160};
use sputnik::backend::MemoryAccount;
use std::{collections::BTreeMap, path::PathBuf};

pub fn dump(
    path: Option<&Option<PathBuf>>,
    pin_block: Option<BlockId>,
    cache: &BTreeMap<H160, MemoryAccount>,
) {
    let pin_block = match pin_block.expect("no pin block found; this should never happen") {
        BlockId::Number(BlockNumber::Number(num)) => num,
        _ => panic!("non block number pin blocks not supported"),
    }
    .as_u64();

    let res = match path {
        Some(Some(path)) => dump_cache(&path, cache),
        Some(None) => {
            let path = dirs_next::home_dir()
                .map(|p| p.join(".foundry").join("cache").join(pin_block.to_string()))
                .expect("could not construct pin block path");
            dump_cache(&path, cache)
        }
        None => Ok(()),
    };

    if let Err(err) = res {
        tracing::error!("could not store fork cache to file. err: {}", err);
    }
}

fn dump_cache(path: &PathBuf, cache: &BTreeMap<H160, MemoryAccount>) -> Result<(), ForkError> {
    std::fs::create_dir_all(path.parent().expect("no parent found"))?;
    let file = std::fs::File::create(path)?;
    let file = std::io::BufWriter::new(file);
    serde_json::to_writer(file, cache)?;
    Ok(())
}

pub fn load_cache(
    path: Option<&Option<PathBuf>>,
    pin_block: Option<BlockId>,
) -> Result<BTreeMap<H160, MemoryAccount>, ForkError> {
    let pin_block = match pin_block {
        Some(inner) => inner,
        None => return Err(ForkError::NoCache),
    };

    let pin_block = match pin_block {
        BlockId::Number(BlockNumber::Number(num)) => num,
        _ => panic!("non block number pin blocks not supported"),
    }
    .as_u64();

    let path = match path {
        Some(Some(path)) => path.clone(),
        Some(None) => {
            let path = dirs_next::home_dir()
                .map(|p| p.join(".foundry").join("cache").join(pin_block.to_string()))
                .expect("could not construct pin block path");
            path
        }
        None => return Err(ForkError::NoCache),
    };

    let file = std::fs::File::open(path)?;
    let file = std::io::BufReader::new(file);
    Ok(serde_json::from_reader(file)?)
}
