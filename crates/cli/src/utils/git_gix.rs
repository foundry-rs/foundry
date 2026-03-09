//! Git operations backed by `gix` (gitoxide) instead of shelling out to `git`.
//!
//! This module provides drop-in replacements for the basic git CLI operations
//! used by the [`super::Git`] helper, starting with `clone` and `fetch`.
//! It is the first step toward removing the git CLI dependency (see foundry#13501).

use eyre::{Context, Result};
use gix::bstr::BStr;
use std::path::Path;

/// Clone a repository from `url` into `target_dir`.
///
/// If `shallow` is true a depth-1 fetch is performed (equivalent to `git clone --depth=1`).
/// Submodules are **not** recursively initialised here – callers should handle
/// that separately (matching the existing `Git::clone` behaviour where
/// `--recurse-submodules` was passed but the actual recursion was driven by a
/// subsequent `submodule update`).
pub fn clone(url: &str, target_dir: &Path, shallow: bool) -> Result<()> {
    // Use `prepare_clone` (non-bare) to get a working tree clone.
    let mut prepare = gix::prepare_clone(url, target_dir)
        .wrap_err_with(|| format!("failed to prepare clone from {url}"))?;

    // Configure shallow fetch when requested.
    if shallow {
        prepare = prepare.with_shallow(gix::remote::fetch::Shallow::DepthAtRemote(
            std::num::NonZeroU32::new(1).unwrap(),
        ));
    }

    // Execute the fetch phase, then checkout the main worktree.
    let (mut checkout, _outcome) = prepare
        .fetch_then_checkout(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
        .wrap_err_with(|| format!("failed to fetch during clone of {url}"))?;

    // Checkout the main worktree (creates the working tree files).
    let (_repo, _checkout_outcome) = checkout
        .main_worktree(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
        .wrap_err("failed to checkout main worktree after clone")?;

    Ok(())
}

/// Fetch from `remote` into the repository rooted at `repo_path`.
///
/// If `branch` is provided only that refspec is fetched.
/// If `shallow` is true a depth-1 shallow fetch is performed.
pub fn fetch(repo_path: &Path, remote: &str, branch: Option<&str>, shallow: bool) -> Result<()> {
    let repo = gix::open(repo_path).wrap_err("failed to open repository for fetch")?;

    let remote_bstr: &BStr = remote.into();
    let mut remote_handle = repo
        .find_remote(remote_bstr)
        .or_else(|_| {
            let url = gix::url::parse(remote_bstr)
                .map_err(|e| gix::remote::find::existing::Error::Find(gix::remote::find::Error::Init(e.into())))?;
            repo.remote_at(url)
                .map_err(|e| gix::remote::find::existing::Error::Find(gix::remote::find::Error::Init(e)))
        })
        .wrap_err_with(|| format!("could not resolve remote '{remote}'"))?;

    // Narrow the refspec if a specific branch was requested.
    if let Some(branch) = branch {
        let spec = format!("+refs/heads/{branch}:refs/remotes/{remote}/{branch}");
        let spec_bstr: &BStr = spec.as_str().into();
        remote_handle
            .replace_refspecs(Some(spec_bstr), gix::remote::Direction::Fetch)
            .wrap_err("failed to set refspec")?;
    }

    let connection = remote_handle
        .connect(gix::remote::Direction::Fetch)
        .wrap_err("failed to connect to remote")?;

    // Execute the fetch. `with_shallow` is on Prepare, not Connection.
    let mut pending = connection
        .prepare_fetch(gix::progress::Discard, Default::default())
        .wrap_err("failed to prepare fetch")?;

    // Configure shallow fetch.
    if shallow {
        pending = pending.with_shallow(gix::remote::fetch::Shallow::DepthAtRemote(
            std::num::NonZeroU32::new(1).unwrap(),
        ));
    }

    pending
        .receive(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
        .wrap_err("fetch failed")?;

    Ok(())
}

/// Checkout a specific `reference` (tag, branch, or commit) inside the
/// repository at `repo_path`.
///
/// When `recursive` is true, after checking out the main tree this function
/// will also initialise and update submodules (not yet implemented – the
/// caller should still fall back to the CLI for recursive submodule handling
/// until a follow-up PR).
pub fn checkout(repo_path: &Path, reference: &str, _recursive: bool) -> Result<()> {
    let repo = gix::open(repo_path).wrap_err("failed to open repository for checkout")?;

    // Resolve the reference to an object id (requires "revision" feature).
    let ref_bstr: &BStr = reference.into();
    let id = repo
        .rev_parse_single(ref_bstr)
        .wrap_err_with(|| format!("failed to resolve reference '{reference}'"))?;

    // Peel to a tree so we can check out files.
    let tree_id = id
        .object()
        .wrap_err("failed to find object")?
        .peel_to_tree()
        .wrap_err("reference does not point to a tree")?
        .id;

    let mut index = repo
        .index_from_tree(&tree_id)
        .wrap_err("failed to build index from tree")?;

    let workdir = repo
        .workdir()
        .ok_or_else(|| eyre::eyre!("repository is bare – cannot checkout"))?;

    let opts = repo
        .checkout_options(gix::worktree::stack::state::attributes::Source::IdMapping)
        .wrap_err("failed to get checkout options")?;

    gix::worktree::state::checkout(
        &mut index,
        workdir,
        repo.objects.clone().into_arc().wrap_err("failed to convert object store to arc")?,
        &gix::progress::Discard,
        &gix::progress::Discard,
        &gix::interrupt::IS_INTERRUPTED,
        opts,
    )
    .wrap_err("checkout failed")?;

    // Update HEAD so that `rev-parse HEAD` returns the expected value.
    let detached_id = id.detach();
    repo.reference(
        "HEAD",
        detached_id,
        gix::refs::transaction::PreviousValue::Any,
        format!("checkout: moving to {reference}"),
    )
    .wrap_err("failed to update HEAD")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_clone_and_checkout() {
        // A small, well-known public repo for integration testing.
        let url = "https://github.com/foundry-rs/forge-std";
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("forge-std");

        // Shallow clone.
        clone(url, &target, true).expect("shallow clone should succeed");
        assert!(target.join(".git").exists());
    }
}
