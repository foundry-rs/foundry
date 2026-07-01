//! Shared helpers for reading on-disk corpus directories.

use eyre::{Result, eyre};
use foundry_evm_fuzz::BasicTxDetails;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const WORKER_DIR_PREFIX: &str = "worker";
const CORPUS_SUBDIR: &str = "corpus";
const MAX_CORPUS_TREE_DEPTH: usize = 64;
const MAX_CORPUS_TREE_DIRS: usize = 10_000;
const MAX_CORPUS_ENTRIES: usize = 1_000_000;

/// Returns every `worker*/corpus/` under `root`, or `[root]` if none exist.
pub fn canonical_replay_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_str()?;
            (e.file_type().ok()?.is_dir() && name.starts_with(WORKER_DIR_PREFIX))
                .then(|| p.join(CORPUS_SUBDIR))
                .filter(|d| is_dir_no_symlink(d))
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
        if self
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("gz"))
        {
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
        if !entry.file_type().ok()?.is_file() {
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

/// Reads corpus files from a file, corpus directory, worker corpus directory, or generated corpus
/// root such as `<root>/<contract>/<test>/worker0/corpus`.
pub fn read_corpus_tree(path: &Path) -> Result<Vec<CorpusDirEntry>> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| eyre!("corpus path does not exist or is not readable: {}", path.display()))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(eyre!("corpus path must not be a symlink: {}", path.display()));
    }
    if file_type.is_file() {
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        let (uuid, timestamp) = parse_corpus_filename(name).unwrap_or((Uuid::nil(), 0));
        return Ok(vec![CorpusDirEntry { path: path.to_path_buf(), uuid, timestamp }]);
    }

    if !file_type.is_dir() {
        return Err(eyre!("corpus path does not exist or is not readable: {}", path.display()));
    }

    let mut seen_entries = HashSet::new();
    let mut visited_dirs = HashSet::new();
    let mut entries = Vec::new();
    let mut stack = vec![(path.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let canonical = match dir.canonicalize() {
            Ok(canonical) => canonical,
            Err(err) => {
                debug!(%err, ?dir, "failed to canonicalize corpus tree directory");
                continue;
            }
        };
        if !visited_dirs.insert(canonical) {
            continue;
        }
        if visited_dirs.len() > MAX_CORPUS_TREE_DIRS {
            return Err(eyre!(
                "corpus tree exceeds directory limit of {MAX_CORPUS_TREE_DIRS}: {}",
                path.display()
            ));
        }

        for replay_dir in canonical_replay_dirs(&dir) {
            for entry in read_corpus_dir(&replay_dir) {
                if seen_entries.insert((entry.uuid, entry.timestamp)) {
                    entries.push(entry);
                    if entries.len() > MAX_CORPUS_ENTRIES {
                        return Err(eyre!(
                            "corpus tree exceeds entry limit of {MAX_CORPUS_ENTRIES}: {}",
                            path.display()
                        ));
                    }
                }
            }
        }

        if depth >= MAX_CORPUS_TREE_DEPTH {
            debug!(?dir, "skipping corpus tree directory beyond depth limit");
            continue;
        }
        let children = match std::fs::read_dir(&dir) {
            Ok(children) => children,
            Err(err) => {
                debug!(%err, ?dir, "failed to read corpus tree directory");
                continue;
            }
        };
        for child in children {
            let Ok(child) =
                child.inspect_err(|err| debug!(%err, "failed to read corpus tree entry"))
            else {
                continue;
            };
            let child_path = child.path();
            let Ok(child_type) = child
                .file_type()
                .inspect_err(|err| debug!(%err, "failed to read corpus tree entry type"))
            else {
                continue;
            };
            if child_type.is_dir() {
                stack.push((child_path, depth + 1));
            }
        }
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

fn is_dir_no_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
}

/// Strips a trailing `suffix` from `name`, comparing case-insensitively.
fn strip_suffix_ci<'a>(name: &'a str, suffix: &str) -> Option<&'a str> {
    let split = name.len().checked_sub(suffix.len())?;
    name.is_char_boundary(split)
        .then(|| name.split_at(split))
        .filter(|(_, tail)| tail.eq_ignore_ascii_case(suffix))
        .map(|(head, _)| head)
}

/// Parses a corpus filename of the form `<uuid>-<timestamp>.json[.gz]`.
///
/// The `.json` / `.gz` extensions are matched case-insensitively so corpus files
/// written with upper-case extensions are still discovered.
pub fn parse_corpus_filename(name: &str) -> Result<(Uuid, u64)> {
    let name = strip_suffix_ci(name, ".gz").unwrap_or(name);
    let name = strip_suffix_ci(name, ".json").unwrap_or(name);
    let (uuid_str, timestamp_str) =
        name.rsplit_once('-').ok_or_else(|| eyre!("invalid corpus filename format: {name}"))?;
    Ok((Uuid::parse_str(uuid_str)?, timestamp_str.parse()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("foundry-corpus-io-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_corpus_tree_finds_generated_layout() {
        let dir = temp_dir();
        let corpus = dir.join("ExampleTest").join("testFuzz_value").join("worker0").join("corpus");
        std::fs::create_dir_all(&corpus).unwrap();
        let entry = corpus.join("00000000-0000-0000-0000-000000000001-1.json");
        std::fs::write(&entry, "[]").unwrap();

        let entries = read_corpus_tree(&dir).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, entry);
    }

    #[test]
    fn read_corpus_tree_dedups_worker_entries_by_uuid() {
        let dir = temp_dir();
        let name = "00000000-0000-0000-0000-000000000001-1.json";
        for worker in ["worker0", "worker1"] {
            let corpus = dir.join("ExampleTest").join("testFuzz_value").join(worker).join("corpus");
            std::fs::create_dir_all(&corpus).unwrap();
            std::fs::write(corpus.join(name), "[]").unwrap();
        }

        let entries = read_corpus_tree(&dir).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn read_corpus_tree_keeps_same_uuid_with_different_timestamps() {
        let dir = temp_dir();
        for timestamp in [1, 2] {
            let name = format!("00000000-0000-0000-0000-000000000001-{timestamp}.json");
            std::fs::write(dir.join(name), "[]").unwrap();
        }

        let entries = read_corpus_tree(&dir).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp, 1);
        assert_eq!(entries[1].timestamp, 2);
    }

    #[cfg(unix)]
    #[test]
    fn read_corpus_tree_rejects_top_level_symlink() {
        let dir = temp_dir();
        let corpus = dir.join("corpus");
        let link = dir.join("link");
        std::fs::create_dir_all(&corpus).unwrap();
        std::os::unix::fs::symlink(&corpus, &link).unwrap();

        let err = match read_corpus_tree(&link) {
            Ok(_) => panic!("top-level symlink should be rejected"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("must not be a symlink"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn read_corpus_tree_skips_symlinked_directories() {
        let dir = temp_dir();
        let corpus = dir.join("corpus");
        let outside = dir.join("outside");
        std::fs::create_dir_all(&corpus).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let outside_entry = outside.join("00000000-0000-0000-0000-000000000001-1.json");
        std::fs::write(&outside_entry, "[]").unwrap();
        std::os::unix::fs::symlink(&outside, corpus.join("link")).unwrap();

        let entries = read_corpus_tree(&corpus).unwrap();
        assert!(
            entries.is_empty(),
            "{:?}",
            entries.iter().map(|entry| &entry.path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn parse_corpus_filename_is_case_insensitive_for_extensions() {
        let uuid = "00000000-0000-0000-0000-000000000001";
        let (parsed_uuid, ts) = parse_corpus_filename(&format!("{uuid}-7.JSON.GZ")).unwrap();
        assert_eq!(parsed_uuid, Uuid::parse_str(uuid).unwrap());
        assert_eq!(ts, 7);

        let (parsed_uuid, ts) = parse_corpus_filename(&format!("{uuid}-9.Json")).unwrap();
        assert_eq!(parsed_uuid, Uuid::parse_str(uuid).unwrap());
        assert_eq!(ts, 9);
    }

    #[test]
    fn read_corpus_tree_discovers_uppercase_extensions() {
        let dir = temp_dir();
        let corpus = dir.join("ExampleTest").join("testFuzz_value").join("worker0").join("corpus");
        std::fs::create_dir_all(&corpus).unwrap();
        let entry = corpus.join("00000000-0000-0000-0000-000000000001-1.JSON.GZ");
        std::fs::write(&entry, "[]").unwrap();

        let entries = read_corpus_tree(&dir).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, entry);
    }

    #[test]
    fn read_corpus_tree_accepts_explicit_single_file_with_arbitrary_name() {
        let dir = temp_dir();
        let entry = dir.join("min.json");
        std::fs::write(&entry, "[]").unwrap();

        let entries = read_corpus_tree(&entry).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, entry);
        assert_eq!(entries[0].uuid, Uuid::nil());
        assert_eq!(entries[0].timestamp, 0);
    }
}
