//! Utilities for handling git repositories.

// Adapted from https://github.com/rust-lang/cargo/blob/f51e799636fcba6aeb98dc2ca7e440ecd9afe909/src/cargo/sources/git/utils.rs

use eyre::Context;
use git2::{self, ErrorClass, ObjectType};
use std::{
    env,
    fmt::Write,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use tracing::{debug, info};
use url::Url;

/// Represents a remote repository.
/// It gets cloned into a local `GitLocal`.
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct GitRemote {
    url: Url,
}

// === impl GitRemote ===

impl GitRemote {
    pub fn new(url: Url) -> GitRemote {
        GitRemote { url }
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn rev_for(
        &self,
        path: impl AsRef<Path>,
        reference: &GitReference,
    ) -> eyre::Result<git2::Oid> {
        reference.resolve(&self.open_local(path)?.repo)
    }

    /// opens a local repository
    pub fn open_local(&self, path: impl AsRef<Path>) -> eyre::Result<GitLocal> {
        let path = path.as_ref();
        let repo = git2::Repository::open(path)?;
        Ok(GitLocal { remote: self.clone(), path: path.to_path_buf(), repo })
    }

    pub fn checkout(
        &self,
        into: &Path,
        reference: &GitReference,
        db: Option<GitLocal>,
    ) -> eyre::Result<(GitLocal, git2::Oid)> {
        // If we have a previous instance of `GitDatabase` then fetch into that
        // if we can. If that can successfully load our revision then we've
        // populated the database with the latest version of `reference`, so
        // return that database and the rev we resolve to.
        if let Some(mut db) = db {
            fetch(&mut db.repo, self.url.as_str(), reference, false)
                .context(format!("failed to fetch into: {}", into.display()))?;

            if let Ok(rev) = reference.resolve(&db.repo) {
                return Ok((db, rev))
            }
        }

        // Otherwise, start from scratch to handle corrupt git repositories.
        // After our fetch (which is interpreted as a clone now) we do the same
        // resolution to figure out what we cloned.
        if into.exists() {
            fs::remove_dir_all(into)?;
        }
        fs::create_dir_all(into)?;
        let mut repo = init(into, true)?;
        fetch(&mut repo, self.url.as_str(), reference, false)
            .context(format!("failed to clone into: {}", into.display()))?;

        let rev = reference.resolve(&repo)?;

        Ok((GitLocal { remote: self.clone(), path: into.to_path_buf(), repo }, rev))
    }
}

/// Represents a local clone of a remote repository's database.
///
/// Supports multiple `GitCheckouts` than can be cloned from this type.
pub struct GitLocal {
    pub remote: GitRemote,
    pub path: PathBuf,
    pub repo: git2::Repository,
}

// === impl GitLocal ===

impl GitLocal {
    pub fn contains(&self, oid: git2::Oid) -> bool {
        self.repo.revparse_single(&oid.to_string()).is_ok()
    }

    pub fn copy_to(&self, rev: git2::Oid, dest: impl AsRef<Path>) -> eyre::Result<GitCheckout<'_>> {
        let dest = dest.as_ref();
        let mut checkout = None;
        if let Ok(repo) = git2::Repository::open(dest) {
            let mut co = GitCheckout::new(dest, self, rev, repo);
            // After a successful fetch operation the subsequent reset can
            // fail sometimes for corrupt repositories where the fetch
            // operation succeeds but the object isn't actually there in one
            // way or another. In these situations just skip the error and
            // try blowing away the whole repository and trying with a
            // clone.
            co.fetch()?;
            match co.reset() {
                Ok(()) => {
                    checkout = Some(co);
                }
                Err(e) => debug!("failed reset after fetch {:?}", e),
            }
        };
        let checkout = match checkout {
            Some(c) => c,
            None => GitCheckout::clone_into(dest, self, rev)?,
        };
        checkout.update_submodules()?;
        Ok(checkout)
    }
}

/// Represents a local checkout of a particular revision. Calling
/// `clone_into` with a reference will resolve the reference into a revision,
pub struct GitCheckout<'a> {
    database: &'a GitLocal,
    _location: PathBuf,
    revision: git2::Oid,
    repo: git2::Repository,
}

// === impl GitCheckout ===

impl<'a> GitCheckout<'a> {
    pub fn new(
        location: impl Into<PathBuf>,
        database: &'a GitLocal,
        revision: git2::Oid,
        repo: git2::Repository,
    ) -> GitCheckout<'a> {
        GitCheckout { _location: location.into(), database, revision, repo }
    }

    fn fetch(&mut self) -> eyre::Result<()> {
        info!("fetch {}", self.repo.path().display());
        let url = Url::from_file_path(&self.database.path)
            .map_err(|_| eyre::eyre!("Invalid file url {}", self.database.path.display()))?;
        let reference = GitReference::Rev(self.revision.to_string());
        fetch(&mut self.repo, url.as_str(), &reference, false)?;
        Ok(())
    }

    pub fn clone_into(
        into: &Path,
        local: &'a GitLocal,
        revision: git2::Oid,
    ) -> eyre::Result<GitCheckout<'a>> {
        let dirname = into.parent().unwrap();
        fs::create_dir_all(dirname)?;
        if into.exists() {
            fs::remove_dir_all(into)?;
        }

        // we're doing a local filesystem-to-filesystem clone so there should
        // be no need to respect global configuration options, so pass in
        // an empty instance of `git2::Config` below.
        let git_config = git2::Config::new()?;

        // Clone the repository, but make sure we use the "local" option in
        // libgit2 which will attempt to use hardlinks to set up the database.
        // This should speed up the clone operation quite a bit if it works.
        //
        // Note that we still use the same fetch options because while we don't
        // need authentication information we may want progress bars and such.
        let url = Url::from_file_path(&local.path)
            .map_err(|_| eyre::eyre!("Invalid file url {}", local.path.display()))?;

        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.dry_run(); // we'll do this below during a `reset`
        let mut checkout = Some(checkout);
        let mut repo = None;

        with_retry(|| {
            with_authentication(url.as_str(), &git_config, |_| {
                let r = git2::build::RepoBuilder::new()
                    // use hard links and/or copy the database, we're doing a
                    // filesystem clone so this'll speed things up quite a bit.
                    .clone_local(git2::build::CloneLocal::Local)
                    .with_checkout(checkout.take().unwrap())
                    .fetch_options(git2::FetchOptions::new())
                    .clone(url.as_str(), into)?;
                repo = Some(r);
                Ok(())
            })
        })?;

        let repo = repo.unwrap();

        let checkout = GitCheckout::new(into, local, revision, repo);
        checkout.reset()?;
        Ok(checkout)
    }

    /// This will perform a reset
    fn reset(&self) -> eyre::Result<()> {
        info!("reset {} to {}", self.repo.path().display(), self.revision);
        // Ensure libgit2 won't mess with newlines when we vendor.
        if let Ok(mut git_config) = self.repo.config() {
            git_config.set_bool("core.autocrlf", false)?;
        }

        let object = self.repo.find_object(self.revision, None)?;
        reset(&self.repo, &object)?;
        Ok(())
    }

    fn update_submodules(&self) -> eyre::Result<()> {
        fn update_submodules(repo: &git2::Repository) -> eyre::Result<()> {
            debug!("update submodules for: {:?}", repo.workdir().unwrap());

            for mut child in repo.submodules()? {
                update_submodule(repo, &mut child).with_context(|| {
                    format!("failed to update submodule `{}`", child.name().unwrap_or(""))
                })?;
            }
            Ok(())
        }

        fn update_submodule(
            parent: &git2::Repository,
            child: &mut git2::Submodule<'_>,
        ) -> eyre::Result<()> {
            child.init(false)?;
            let url = child
                .url()
                .ok_or_else(|| eyre::eyre!("non-utf8 url for submodule {:?}?", child.path()))?;

            // A submodule which is listed in .gitmodules but not actually
            // checked out will not have a head id, so we should ignore it.
            let head = match child.head_id() {
                Some(head) => head,
                None => return Ok(()),
            };

            // If the submodule hasn't been checked out yet, we need to
            // clone it. If it has been checked out and the head is the same
            // as the submodule's head, then we can skip an update and keep
            // recursing.
            let head_and_repo = child.open().and_then(|repo| {
                let target = repo.head()?.target();
                Ok((target, repo))
            });
            let mut repo = match head_and_repo {
                Ok((head, repo)) => {
                    if child.head_id() == head {
                        return update_submodules(&repo)
                    }
                    repo
                }
                Err(..) => {
                    let path = parent.workdir().unwrap().join(child.path());
                    let _ = fs::remove_dir_all(&path);
                    init(&path, false)?
                }
            };
            // Fetch data from origin and reset to the head commit
            let reference = GitReference::Rev(head.to_string());

            fetch(&mut repo, url, &reference, false).with_context(|| {
                format!("failed to fetch submodule `{}` from {url}", child.name().unwrap_or(""))
            })?;

            let obj = repo.find_object(head, None)?;
            reset(&repo, &obj)?;
            update_submodules(&repo)
        }

        update_submodules(&self.repo)
    }
}

/// Represents a specific commit in a git repository
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum GitReference {
    /// Tag, like a release v0.0.1
    Tag(String),
    /// Specific Branch
    Branch(String),
    /// Specific revision.
    Rev(String),
    /// Default branch
    #[default]
    DefaultBranch,
}

// === impl GitReference ===

impl GitReference {
    /// Resolves the unique identify of the reference for the given [Repository](git2::Repository)
    pub fn resolve(&self, repo: &git2::Repository) -> eyre::Result<git2::Oid> {
        let id = match self {
            GitReference::Tag(s) => {
                let resolve_tag = move || -> eyre::Result<git2::Oid> {
                    let refname = format!("refs/remotes/origin/tags/{s}");
                    let id = repo.refname_to_id(&refname)?;
                    let obj = repo.find_object(id, None)?;
                    let obj = obj.peel(ObjectType::Commit)?;
                    Ok(obj.id())
                };
                resolve_tag().with_context(|| format!("failed to find tag `{s}`"))?
            }
            GitReference::Branch(s) => {
                let name = format!("origin/{s}");
                let b = repo
                    .find_branch(&name, git2::BranchType::Remote)
                    .with_context(|| format!("failed to find branch `{s}`"))?;
                b.get().target().ok_or_else(|| eyre::eyre!("branch `{s}` did not have a target"))?
            }

            // use the HEAD commit
            GitReference::DefaultBranch => {
                let head_id = repo.refname_to_id("refs/remotes/origin/HEAD")?;
                let head = repo.find_object(head_id, None)?;
                head.peel(ObjectType::Commit)?.id()
            }

            GitReference::Rev(s) => {
                let obj = repo.revparse_single(s)?;
                if let Some(tag) = obj.as_tag() {
                    tag.target_id()
                } else {
                    obj.id()
                }
            }
        };
        Ok(id)
    }
}

fn reinitialize(repo: &mut git2::Repository) -> eyre::Result<()> {
    // Here we want to drop the current repository object pointed to by `repo`,
    // so we initialize temporary repository in a sub-folder, blow away the
    // existing git folder, and then recreate the git repo. Finally we blow away
    // the `tmp` folder we allocated.
    let path = repo.path().to_path_buf();
    debug!("reinitializing git repo at {:?}", path);
    let tmp = path.join("tmp");
    let bare = !repo.path().ends_with(".git");
    *repo = init(&tmp, false)?;
    for entry in path.read_dir()? {
        let entry = entry?;
        if entry.file_name().to_str() == Some("tmp") {
            continue
        }
        let path = entry.path();
        let _ = fs::remove_file(&path).or_else(|_| fs::remove_dir_all(&path));
    }
    *repo = init(&path, bare)?;
    fs::remove_dir_all(&tmp)?;
    Ok(())
}

fn init(path: &Path, bare: bool) -> eyre::Result<git2::Repository> {
    let mut opts = git2::RepositoryInitOptions::new();
    // Skip anything related to templates, they just call all sorts of issues as
    // we really don't want to use them yet they insist on being used. See #6240
    // for an example issue that comes up.
    opts.external_template(false);
    opts.bare(bare);
    Ok(git2::Repository::init_opts(path, &opts)?)
}

fn reset(repo: &git2::Repository, obj: &git2::Object<'_>) -> eyre::Result<()> {
    let mut opts = git2::build::CheckoutBuilder::new();
    debug!("doing git reset");
    repo.reset(obj, git2::ResetType::Hard, Some(&mut opts))?;
    debug!("git reset done");
    Ok(())
}

pub struct Retry {
    remaining: u32,
}

impl Retry {
    pub fn new(remaining: u32) -> Self {
        Self { remaining }
    }

    pub fn r#try<T>(&mut self, f: impl FnOnce() -> eyre::Result<T>) -> eyre::Result<Option<T>> {
        match f() {
            Err(ref e) if maybe_spurious(e) && self.remaining > 0 => {
                let msg = format!(
                    "spurious network error ({} tries remaining): {}",
                    self.remaining,
                    e.root_cause(),
                );
                println!("{msg}");
                self.remaining -= 1;
                Ok(None)
            }
            other => other.map(Some),
        }
    }
}

fn maybe_spurious(err: &eyre::Error) -> bool {
    if let Some(git_err) = err.downcast_ref::<git2::Error>() {
        match git_err.class() {
            git2::ErrorClass::Net |
            git2::ErrorClass::Os |
            git2::ErrorClass::Zlib |
            git2::ErrorClass::Http => return true,
            _ => (),
        }
    }
    if let Some(curl_err) = err.downcast_ref::<curl::Error>() {
        if curl_err.is_couldnt_connect() ||
            curl_err.is_couldnt_resolve_proxy() ||
            curl_err.is_couldnt_resolve_host() ||
            curl_err.is_operation_timedout() ||
            curl_err.is_recv_error() ||
            curl_err.is_send_error() ||
            curl_err.is_http2_error() ||
            curl_err.is_http2_stream_error() ||
            curl_err.is_ssl_connect_error() ||
            curl_err.is_partial_file()
        {
            return true
        }
    }
    false
}

pub fn with_retry<T, F>(mut callback: F) -> eyre::Result<T>
where
    F: FnMut() -> eyre::Result<T>,
{
    let mut retry = Retry::new(2);
    loop {
        if let Some(ret) = retry.r#try(&mut callback)? {
            return Ok(ret)
        }
    }
}

/// Prepare the authentication callbacks for cloning a git repository.
///
/// The main purpose of this function is to construct the "authentication
/// callback" which is used to clone a repository. This callback will attempt to
/// find the right authentication on the system (without user input) and will
/// guide libgit2 in doing so.
///
/// The callback is provided `allowed` types of credentials, and we try to do as
/// much as possible based on that:
///
/// * Prioritize SSH keys from the local ssh agent as they're likely the most reliable. The username
///   here is prioritized from the credential callback, then from whatever is configured in git
///   itself, and finally we fall back to the generic user of `git`.
///
/// * If a username/password is allowed, then we fallback to git2-rs's implementation of the
///   credential helper. This is what is configured with `credential.helper` in git, and is the
///   interface for the macOS keychain, for example.
///
/// * After the above two have failed, we just kinda grapple attempting to return *something*.
///
/// If any form of authentication fails, libgit2 will repeatedly ask us for
/// credentials until we give it a reason to not do so. To ensure we don't
/// just sit here looping forever we keep track of authentications we've
/// attempted and we don't try the same ones again.
fn with_authentication<T, F>(url: &str, cfg: &git2::Config, mut f: F) -> eyre::Result<T>
where
    F: FnMut(&mut git2::Credentials<'_>) -> eyre::Result<T>,
{
    let mut cred_helper = git2::CredentialHelper::new(url);
    cred_helper.config(cfg);

    let mut ssh_username_requested = false;
    let mut cred_helper_bad = None;
    let mut ssh_agent_attempts = Vec::new();
    let mut any_attempts = false;
    let mut tried_sshkey = false;
    let mut url_attempt = None;

    let orig_url = url;
    let mut res = f(&mut |url, username, allowed| {
        any_attempts = true;
        if url != orig_url {
            url_attempt = Some(url.to_string());
        }
        // libgit2's "USERNAME" authentication actually means that it's just
        // asking us for a username to keep going. This is currently only really
        // used for SSH authentication and isn't really an authentication type.
        // The logic currently looks like:
        //
        //      let user = ...;
        //      if (user.is_null())
        //          user = callback(USERNAME, null, ...);
        //
        //      callback(SSH_KEY, user, ...)
        //
        // So if we're being called here then we know that (a) we're using ssh
        // authentication and (b) no username was specified in the URL that
        // we're trying to clone. We need to guess an appropriate username here,
        // but that may involve a few attempts. Unfortunately we can't switch
        // usernames during one authentication session with libgit2, so to
        // handle this we bail out of this authentication session after setting
        // the flag `ssh_username_requested`, and then we handle this below.
        if allowed.contains(git2::CredentialType::USERNAME) {
            debug_assert!(username.is_none());
            ssh_username_requested = true;
            return Err(git2::Error::from_str("gonna try usernames later"))
        }

        // An "SSH_KEY" authentication indicates that we need some sort of SSH
        // authentication. This can currently either come from the ssh-agent
        // process or from a raw in-memory SSH key. We only support using
        // ssh-agent currently.
        //
        // If we get called with this then the only way that should be possible
        // is if a username is specified in the URL itself (e.g., `username` is
        // Some), hence the unwrap() here. We try custom usernames down below.
        if allowed.contains(git2::CredentialType::SSH_KEY) && !tried_sshkey {
            // If ssh-agent authentication fails, libgit2 will keep
            // calling this callback asking for other authentication
            // methods to try. Make sure we only try ssh-agent once,
            // to avoid looping forever.
            tried_sshkey = true;
            let username = username.unwrap();
            debug_assert!(!ssh_username_requested);
            ssh_agent_attempts.push(username.to_string());
            return git2::Cred::ssh_key_from_agent(username)
        }

        // Sometimes libgit2 will ask for a username/password in plaintext.
        //
        // If ssh-agent authentication fails, libgit2 will keep calling this
        // callback asking for other authentication methods to try. Check
        // cred_helper_bad to make sure we only try the git credentail helper
        // once, to avoid looping forever.
        if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) && cred_helper_bad.is_none()
        {
            let r = git2::Cred::credential_helper(cfg, url, username);
            cred_helper_bad = Some(r.is_err());
            return r
        }

        // I'm... not sure what the DEFAULT kind of authentication is, but seems
        // easy to support?
        if allowed.contains(git2::CredentialType::DEFAULT) {
            return git2::Cred::default()
        }

        // Whelp, we tried our best
        Err(git2::Error::from_str("no authentication available"))
    });

    // Ok, so if it looks like we're going to be doing ssh authentication, we
    // want to try a few different usernames as one wasn't specified in the URL
    // for us to use. In order, we'll try:
    //
    // * A credential helper's username for this URL, if available.
    // * This account's username.
    // * "git"
    //
    // We have to restart the authentication session each time (due to
    // constraints in libssh2 I guess? maybe this is inherent to ssh?), so we
    // call our callback, `f`, in a loop here.
    if ssh_username_requested {
        debug_assert!(res.is_err());
        let mut attempts = vec![String::from("git")];
        if let Ok(s) = env::var("USER").or_else(|_| env::var("USERNAME")) {
            attempts.push(s);
        }
        if let Some(ref s) = cred_helper.username {
            attempts.push(s.clone());
        }

        while let Some(s) = attempts.pop() {
            // We should get `USERNAME` first, where we just return our attempt,
            // and then after that we should get `SSH_KEY`. If the first attempt
            // fails we'll get called again, but we don't have another option so
            // we bail out.
            let mut attempts = 0;
            res = f(&mut |_url, username, allowed| {
                if allowed.contains(git2::CredentialType::USERNAME) {
                    return git2::Cred::username(&s)
                }
                if allowed.contains(git2::CredentialType::SSH_KEY) {
                    debug_assert_eq!(Some(&s[..]), username);
                    attempts += 1;
                    if attempts == 1 {
                        ssh_agent_attempts.push(s.to_string());
                        return git2::Cred::ssh_key_from_agent(&s)
                    }
                }
                Err(git2::Error::from_str("no authentication available"))
            });

            // If we made two attempts then that means:
            //
            // 1. A username was requested, we returned `s`.
            // 2. An ssh key was requested, we returned to look up `s` in the ssh agent.
            // 3. For whatever reason that lookup failed, so we were asked again for another mode of
            //    authentication.
            //
            // Essentially, if `attempts == 2` then in theory the only error was
            // that this username failed to authenticate (e.g., no other network
            // errors happened). Otherwise something else is funny so we bail
            // out.
            if attempts != 2 {
                break
            }
        }
    }
    let mut err = match res {
        Ok(e) => return Ok(e),
        Err(e) => e,
    };

    // In the case of an authentication failure (where we tried something) then
    // we try to give a more helpful error message about precisely what we
    // tried.
    if any_attempts {
        let mut msg = "failed to authenticate when downloading \
                       repository"
            .to_string();

        if let Some(attempt) = &url_attempt {
            if url != attempt {
                msg.push_str(": ");
                msg.push_str(attempt);
            }
        }
        msg.push('\n');
        if !ssh_agent_attempts.is_empty() {
            let names =
                ssh_agent_attempts.iter().map(|s| format!("`{s}`")).collect::<Vec<_>>().join(", ");
            write!(
                &mut msg,
                "\n* attempted ssh-agent authentication, but \
                 no usernames succeeded: {names}"
            )
            .expect("could not write to msg");
        }
        if let Some(failed_cred_helper) = cred_helper_bad {
            if failed_cred_helper {
                msg.push_str(
                    "\n* attempted to find username/password via \
                     git's `credential.helper` support, but failed",
                );
            } else {
                msg.push_str(
                    "\n* attempted to find username/password via \
                     `credential.helper`, but maybe the found \
                     credentials were incorrect",
                );
            }
        }
        err = err.wrap_err(msg);

        // Otherwise if we didn't even get to the authentication phase them we may
        // have failed to set up a connection, in these cases hint on the
        // `net.git-fetch-with-cli` configuration option.
    } else if let Some(e) = err.downcast_ref::<git2::Error>() {
        match e.class() {
            ErrorClass::Net |
            ErrorClass::Ssl |
            ErrorClass::Submodule |
            ErrorClass::FetchHead |
            ErrorClass::Ssh |
            ErrorClass::Callback |
            ErrorClass::Http => {
                err = err.wrap_err("network failure seems to have happened");
            }
            _ => {}
        }
    }

    Err(err)
}

pub fn fetch(
    repo: &mut git2::Repository,
    url: &str,
    reference: &GitReference,
    git_fetch_with_cli: bool,
) -> eyre::Result<()> {
    // Translate the reference desired here into an actual list of refspecs
    // which need to get fetched. Additionally record if we're fetching tags.
    let mut refspecs = Vec::new();
    let mut tags = false;
    // The `+` symbol on the refspec means to allow a forced (fast-forward)
    // update which is needed if there is ever a force push that requires a
    // fast-forward.
    match reference {
        // For branches and tags we can fetch simply one reference and copy it
        // locally, no need to fetch other branches/tags.
        GitReference::Branch(b) => {
            refspecs.push(format!("+refs/heads/{b}:refs/remotes/origin/{b}"));
        }
        GitReference::Tag(t) => {
            refspecs.push(format!("+refs/tags/{t}:refs/remotes/origin/tags/{t}"));
        }

        GitReference::DefaultBranch => {
            refspecs.push(String::from("+HEAD:refs/remotes/origin/HEAD"));
        }

        GitReference::Rev(rev) => {
            if rev.starts_with("refs/") {
                refspecs.push(format!("+{rev}:{rev}"));
            } else {
                // We don't know what the rev will point to. To handle this
                // situation we fetch all branches and tags, and then we pray
                // it's somewhere in there.
                refspecs.push(String::from("+refs/heads/*:refs/remotes/origin/*"));
                refspecs.push(String::from("+HEAD:refs/remotes/origin/HEAD"));
                tags = true;
            }
        }
    }

    // Unfortunately `libgit2` is notably lacking in the realm of authentication
    // when compared to the `git` command line. As a result, allow an escape
    // hatch for users that would prefer to use `git`-the-CLI for fetching
    // repositories instead of `libgit2`-the-library. This should make more
    // flavors of authentication possible while also still giving us all the
    // speed and portability of using `libgit2`.
    if git_fetch_with_cli {
        return fetch_with_cli(repo, url, &refspecs, tags)
    }

    debug!("doing a fetch for {url}");
    let git_config = git2::Config::open_default()?;

    with_retry(|| {
        with_authentication(url, &git_config, |f| {
            let mut opts = git2::FetchOptions::new();
            let mut rcb = git2::RemoteCallbacks::new();
            rcb.credentials(f);
            opts.remote_callbacks(rcb);
            if tags {
                opts.download_tags(git2::AutotagOption::All);
            }
            // The `fetch` operation here may fail spuriously due to a corrupt
            // repository. It could also fail, however, for a whole slew of other
            // reasons (aka network related reasons).
            //
            // Consequently, we save off the error of the `fetch` operation and if it
            // looks like a "corrupt repo" error then we blow away the repo and try
            // again. If it looks like any other kind of error, or if we've already
            // blown away the repository, then we want to return the error as-is.
            let mut repo_reinitialized = false;
            loop {
                debug!("initiating fetch of {:?} from {}", refspecs, url);
                let res = repo.remote_anonymous(url)?.fetch(&refspecs, Some(&mut opts), None);
                let err = match res {
                    Ok(()) => break,
                    Err(e) => e,
                };
                debug!("fetch failed: {err}");

                if !repo_reinitialized &&
                    matches!(err.class(), ErrorClass::Reference | ErrorClass::Odb)
                {
                    repo_reinitialized = true;
                    debug!(
                        "looks like this is a corrupt repository, reinitializing \
                     and trying again"
                    );
                    if reinitialize(repo).is_ok() {
                        continue
                    }
                }

                return Err(err.into())
            }
            Ok(())
        })
    })?;
    Ok(())
}

fn fetch_with_cli(
    repo: &git2::Repository,
    url: &str,
    refspecs: &[String],
    tags: bool,
) -> eyre::Result<()> {
    let mut cmd = Command::new("git");
    cmd.arg("fetch");
    if tags {
        cmd.arg("--tags");
    }
    cmd.arg("--force") // handle force pushes
        .arg("--update-head-ok")
        .arg(url)
        .args(refspecs)
        .env_remove("GIT_DIR")
        // The reset of these may not be necessary, but I'm including them
        // just to be extra paranoid and avoid any issues.
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .current_dir(repo.path());

    cmd.output()?;
    Ok(())
}
