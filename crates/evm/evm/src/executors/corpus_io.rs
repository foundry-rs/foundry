//! Shared helpers for reading on-disk corpus directories.

use eyre::{Result, eyre};
use foundry_evm_fuzz::BasicTxDetails;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const WORKER_DIR_PREFIX: &str = "worker";
const CORPUS_SUBDIR: &str = "corpus";

/// Returns every `worker*/corpus/` under `root`, or `[root]` if none exist.
pub fn canonical_replay_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_str()?;
            (p.is_dir() && name.starts_with(WORKER_DIR_PREFIX))
                .then(|| p.join(CORPUS_SUBDIR))
                .filter(|d| d.is_dir())
        })
        .collect();
    dirs.sort();
    if dirs.is_empty() {
        dirs.push(root.to_path_buf());
    }
    dirs
}

/// A single corpus file on disk.
pub struct CorpusDirEntry {
    pub path: PathBuf,
    pub uuid: Uuid,
    pub timestamp: u64,
}

impl CorpusDirEntry {
    pub fn name(&self) -> &str {
        self.path.file_name().unwrap().to_str().unwrap()
    }

    pub fn read_tx_seq(&self) -> foundry_common::fs::Result<Vec<BasicTxDetails>> {
        if self.path.extension() == Some("gz".as_ref()) {
            foundry_common::fs::read_json_gzip_file(&self.path)
        } else {
            foundry_common::fs::read_json_file(&self.path)
        }
    }
}

/// Iterate corpus files in `path`, ignoring entries with unparsable names.
pub fn read_corpus_dir(path: &Path) -> impl Iterator<Item = CorpusDirEntry> {
    let dir = match std::fs::read_dir(path) {
        Ok(dir) => dir,
        Err(err) => {
            debug!(%err, ?path, "failed to read corpus directory");
            return vec![].into_iter();
        }
    };

    dir.filter_map(|res| {
        let entry =
            res.inspect_err(|err| debug!(%err, "failed to read corpus directory entry")).ok()?;
        let path = entry.path();
        if !path.is_file() {
            return None;
        }
        let name = path.file_name()?.to_str()?;
        match parse_corpus_filename(name) {
            Ok((uuid, timestamp)) => Some(CorpusDirEntry { path, uuid, timestamp }),
            Err(_) => {
                debug!(target: "corpus", ?path, "failed to parse corpus filename");
                None
            }
        }
    })
    .collect::<Vec<_>>()
    .into_iter()
}

/// Parses a corpus filename of the form `<uuid>-<timestamp>.json[.gz]`.
pub fn parse_corpus_filename(name: &str) -> Result<(Uuid, u64)> {
    let name = name.trim_end_matches(".gz").trim_end_matches(".json");
    let (uuid_str, timestamp_str) =
        name.rsplit_once('-').ok_or_else(|| eyre!("invalid corpus filename format: {name}"))?;
    Ok((Uuid::parse_str(uuid_str)?, timestamp_str.parse()?))
}
