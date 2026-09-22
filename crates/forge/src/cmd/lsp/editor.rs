//! Launch the bundled VS Code client without requiring a Foundry checkout.

use alloy_primitives::{Keccak256, keccak256};
use eyre::{Context, Result, ensure, eyre};
use flate2::read::GzDecoder;
use foundry_config::Config;
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt, symlink};

const CLIENT: &[u8] = include_bytes!("../../../../../editors/vscode/dist/extension.js.gz");
const ASSETS: &[(&str, &[u8])] = &[
    ("package.json", include_bytes!("../../../../../editors/vscode/package.json")),
    (
        "language-configuration.json",
        include_bytes!("../../../../../editors/vscode/language-configuration.json"),
    ),
    (
        "syntaxes/solidity.json",
        include_bytes!("../../../../../editors/vscode/syntaxes/solidity.json"),
    ),
    (
        "syntaxes/solidity-markdown-injection.json",
        include_bytes!("../../../../../editors/vscode/syntaxes/solidity-markdown-injection.json"),
    ),
    ("syntaxes/LICENSE", include_bytes!("../../../../../editors/vscode/syntaxes/LICENSE")),
    ("LICENSE", include_bytes!("../../../../../editors/vscode/LICENSE")),
    ("LICENSE-MIT", include_bytes!("../../../../../LICENSE-MIT")),
    ("LICENSE-APACHE", include_bytes!("../../../../../LICENSE-APACHE")),
    ("NOTICE.md", include_bytes!("../../../../../editors/vscode/NOTICE.md")),
    (
        "THIRD_PARTY_NOTICES.txt",
        include_bytes!("../../../../../editors/vscode/dist/THIRD_PARTY_NOTICES.txt"),
    ),
];

pub(super) fn launch(path: Option<&Path>, code_path: Option<&Path>) -> Result<()> {
    let project =
        dunce::canonicalize(path.map(Path::to_path_buf).unwrap_or(std::env::current_dir()?))
            .wrap_err("Could not open the project directory")?;
    ensure!(project.is_dir(), "Project path must be a directory: {}", project.display());
    let forge = std::env::current_exe()?;
    let profile = Config::selected_profile().to_string();
    let code = code_path.map(Path::to_path_buf).unwrap_or_else(default_code_path);
    let launch_error = || {
        format!(
            "Could not launch VS Code using {}. Install VS Code and its `code` command, \
             or pass --code-path <PATH> to the VS Code CLI. \
             Use `forge lsp --stdio` for another editor.",
            code.display()
        )
    };
    let code = which::which(&code).wrap_err_with(launch_error)?;
    let code_target = dunce::canonicalize(&code).wrap_err_with(launch_error)?;
    let cache = Config::foundry_cache_dir()
        .ok_or_else(|| eyre!("Could not find the Foundry cache directory"))?
        .join("lsp");
    let extension = prepare_extension(&cache)?;

    // Keep launcher paths distinct for dispatchers such as Snap, even when their targets match.
    // Include the target too so repointing a launcher symlink cannot reuse another editor's state.
    let session_key =
        keccak256(serde_json::to_vec(&(&project, &forge, &profile, &code, &code_target))?);
    let session =
        vscode_session_dir(&Config::data_dir()?.join("lsp"), &format!("{session_key:x}")[..16])?;
    // Portable VS Code appends a Unix socket name to user-data even when XDG_RUNTIME_DIR is set.
    // A short link keeps sockets below platform limits without storing editor state in /tmp.
    #[cfg(unix)]
    let session = vscode_session_link(&session, Path::new("/tmp"))?;
    let user_data = session.join("user-data");
    let extensions = session.join("extensions");
    fs::create_dir_all(user_data.join("User"))?;
    fs::create_dir_all(&extensions)?;
    let settings = user_data.join("User/settings.json");
    if !settings.exists() {
        let mut file = tempfile::NamedTempFile::new_in(user_data.join("User"))?;
        serde_json::to_writer_pretty(
            &mut file,
            &serde_json::json!({
                "solarLsp.forgePath": forge,
                "workbench.startupEditor": "none",
            }),
        )?;
        file.write_all(b"\n")?;
        if let Err(error) = file.persist_noclobber(&settings)
            && error.error.kind() != io::ErrorKind::AlreadyExists
        {
            return Err(error.into());
        }
    }

    sh_status!("Opening VS Code with Forge Solidity support: {}", project.display())?;
    let status = Command::new(&code)
        .arg("--new-window")
        .arg("--extensionDevelopmentPath")
        .arg(&extension)
        .arg("--user-data-dir")
        .arg(&user_data)
        .arg("--extensions-dir")
        .arg(&extensions)
        .arg(&project)
        .env_remove("VSCODE_APPDATA")
        .env_remove("VSCODE_EXTENSIONS")
        // Portable installations otherwise override --user-data-dir during autodetection.
        .env("VSCODE_PORTABLE", &session)
        .env_remove("VSCODE_IPC_HOOK_CLI")
        .env("FOUNDRY_LSP_FORGE", &forge)
        .env("FOUNDRY_PROFILE", &profile)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .wrap_err_with(launch_error)?;
    ensure!(status.success(), "VS Code launcher exited with {status}");
    Ok(())
}

fn vscode_session_dir(data: &Path, key: &str) -> Result<PathBuf> {
    let root = data.join("vscode");
    let session = root.join(key);
    #[cfg(unix)]
    {
        fs::create_dir_all(data)?;
        let uid = rustix::process::geteuid().as_raw();
        create_private_dir(&root, uid)?;
        create_private_dir(&session, uid)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(&session)?;
    Ok(session)
}

#[cfg(unix)]
fn vscode_session_link(session: &Path, temp: &Path) -> Result<PathBuf> {
    let session = dunce::canonicalize(session)?;
    let uid = rustix::process::geteuid().as_raw();
    let root = temp.join(format!("foundry-lsp-{uid}"));
    create_private_dir(&root, uid)?;
    // Include the data location so separate homes cannot share a running editor. The prefix also
    // avoids colliding with older launchers' temporary profile directories.
    let key = keccak256(session.as_os_str().as_encoded_bytes());
    let link = root.join(format!("p-{}", &format!("{key:x}")[..16]));
    if let Err(error) = symlink(&session, &link)
        && error.kind() != io::ErrorKind::AlreadyExists
    {
        return Err(error.into());
    }
    let metadata = fs::symlink_metadata(&link)?;
    ensure!(
        metadata.file_type().is_symlink() && metadata.uid() == uid,
        "VS Code session link is not a symlink owned by the current user: {}",
        link.display()
    );
    ensure!(
        fs::read_link(&link)? == session,
        "VS Code session link points to an unexpected directory: {}",
        link.display()
    );
    Ok(link)
}

#[cfg(unix)]
fn create_private_dir(path: &Path, uid: u32) -> Result<()> {
    // Atomic creation and no-follow metadata reject directories planted by another user.
    if let Err(error) = fs::DirBuilder::new().mode(0o700).create(path)
        && error.kind() != io::ErrorKind::AlreadyExists
    {
        return Err(error.into());
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.file_type().is_dir(),
        "VS Code session path is not a directory: {}",
        path.display()
    );
    ensure!(
        metadata.uid() == uid,
        "VS Code session path is not owned by the current user: {}",
        path.display()
    );
    ensure!(
        metadata.permissions().mode() & 0o777 == 0o700,
        "VS Code session path is not private: {}",
        path.display()
    );
    Ok(())
}

fn prepare_extension(cache: &Path) -> Result<PathBuf> {
    let mut hash = Keccak256::new();
    hash.update(CLIENT);
    for (name, bytes) in ASSETS {
        hash.update(name.as_bytes());
        hash.update(bytes);
    }
    let parent = cache.join("extensions");
    let directory = parent.join(format!("{:x}", hash.finalize()));
    if directory.is_dir() {
        return Ok(directory);
    }

    fs::create_dir_all(&parent)?;
    let staging = tempfile::Builder::new().prefix(".extract-").tempdir_in(&parent)?;
    fs::create_dir_all(staging.path().join("out"))?;
    io::copy(
        &mut GzDecoder::new(CLIENT),
        &mut fs::File::create(staging.path().join("out/extension.js"))?,
    )?;
    for (name, bytes) in ASSETS {
        let path = staging.path().join(name);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, bytes)?;
    }
    // Publish a complete directory atomically; concurrent launches may finish extraction first.
    if let Err(error) = fs::rename(staging.path(), &directory)
        && !directory.is_dir()
    {
        return Err(error).wrap_err("Could not cache the bundled VS Code extension");
    }
    Ok(directory)
}

fn default_code_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|path| path.join("code").is_file())
        }) {
            return PathBuf::from("code");
        }
        // The application includes a CLI even when `code` has not been added to PATH.
        let cli =
            PathBuf::from("/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code");
        if cli.is_file() {
            return cli;
        }
    }
    PathBuf::from(if cfg!(windows) { "code.cmd" } else { "code" })
}

#[cfg(test)]
mod tests {
    use std::fs;

    #[cfg(unix)]
    use std::os::unix::{
        fs::{PermissionsExt, symlink},
        net::UnixListener,
    };

    #[test]
    fn vscode_session_uses_durable_storage() {
        let data = tempfile::tempdir().unwrap();
        let session = super::vscode_session_dir(data.path(), "0123456789abcdef").unwrap();
        assert_eq!(session, data.path().join("vscode/0123456789abcdef"));
        assert!(session.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn vscode_session_rejects_symlink_root() {
        let temp = tempfile::Builder::new().prefix("fl-").tempdir_in("/tmp").unwrap();
        let data = tempfile::tempdir().unwrap();
        let redirected = tempfile::tempdir().unwrap();
        let session = super::vscode_session_dir(data.path(), "0123456789abcdef").unwrap();
        let link = super::vscode_session_link(&session, temp.path()).unwrap();
        let root = link.parent().unwrap();
        fs::remove_dir_all(root).unwrap();
        symlink(redirected.path(), root).unwrap();

        assert!(super::vscode_session_link(&session, temp.path()).is_err());
        assert_eq!(fs::read_dir(redirected.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn vscode_session_rejects_symlink_session() {
        let data = tempfile::tempdir().unwrap();
        let redirected = tempfile::tempdir().unwrap();
        let session = super::vscode_session_dir(data.path(), "0123456789abcdef").unwrap();
        fs::remove_dir(&session).unwrap();
        symlink(redirected.path(), &session).unwrap();

        assert!(super::vscode_session_dir(data.path(), "0123456789abcdef").is_err());
        assert_eq!(fs::read_dir(redirected.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn vscode_session_rejects_unexpected_link() {
        let temp = tempfile::Builder::new().prefix("fl-").tempdir_in("/tmp").unwrap();
        let data = tempfile::tempdir().unwrap();
        let redirected = tempfile::tempdir().unwrap();
        let session = super::vscode_session_dir(data.path(), "0123456789abcdef").unwrap();
        let link = super::vscode_session_link(&session, temp.path()).unwrap();
        fs::remove_file(&link).unwrap();
        symlink(redirected.path(), &link).unwrap();
        assert!(super::vscode_session_link(&session, temp.path()).is_err());
        assert_eq!(fs::read_dir(redirected.path()).unwrap().count(), 0);

        fs::remove_file(&link).unwrap();
        fs::create_dir(&link).unwrap();
        assert!(super::vscode_session_link(&session, temp.path()).is_err());
        assert!(link.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn vscode_session_rejects_shared_permissions() {
        let temp = tempfile::Builder::new().prefix("fl-").tempdir_in("/tmp").unwrap();
        let data = tempfile::tempdir().unwrap();
        let session = super::vscode_session_dir(data.path(), "0123456789abcdef").unwrap();
        for directory in [&session, session.parent().unwrap()] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(super::vscode_session_dir(data.path(), "0123456789abcdef").is_err());
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let link = super::vscode_session_link(&session, temp.path()).unwrap();
        fs::set_permissions(link.parent().unwrap(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(super::vscode_session_link(&session, temp.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn vscode_session_rejects_another_owner() {
        let temp = tempfile::Builder::new().prefix("fl-").tempdir_in("/tmp").unwrap();
        let uid = rustix::process::geteuid().as_raw();
        let error = super::create_private_dir(temp.path(), uid.wrapping_add(1)).unwrap_err();
        assert!(
            error.to_string().starts_with("VS Code session path is not owned by the current user:")
        );
    }

    #[cfg(unix)]
    #[test]
    fn vscode_session_allows_concurrent_launches() {
        let temp = tempfile::Builder::new().prefix("fl-").tempdir_in("/tmp").unwrap();
        let data = tempfile::tempdir().unwrap();
        std::thread::scope(|scope| {
            let threads = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        let session =
                            super::vscode_session_dir(data.path(), "0123456789abcdef").unwrap();
                        super::vscode_session_link(&session, temp.path()).unwrap()
                    })
                })
                .collect::<Vec<_>>();
            let session = super::vscode_session_dir(data.path(), "0123456789abcdef").unwrap();
            let expected = super::vscode_session_link(&session, temp.path()).unwrap();
            for thread in threads {
                assert_eq!(thread.join().unwrap(), expected);
            }
        });
    }

    #[cfg(unix)]
    #[test]
    fn vscode_session_path_fits_unix_socket_limit() {
        let temp = tempfile::Builder::new().prefix("fl-").tempdir_in("/tmp").unwrap();
        let data = tempfile::tempdir().unwrap();
        let session = super::vscode_session_dir(
            &data.path().join("long-home-directory".repeat(8)),
            "0123456789abcdef",
        )
        .unwrap();
        fs::create_dir(session.join("user-data")).unwrap();
        assert!(session.as_os_str().len() > 103);
        let link = super::vscode_session_link(&session, temp.path()).unwrap();
        let socket = link.join("user-data/1.13-main.sock");
        let length = socket.as_os_str().len();
        assert!(length < 103, "{length} bytes");
        let _listener = UnixListener::bind(socket).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn vscode_session_uses_private_stable_root() {
        let temp = tempfile::Builder::new().prefix("fl-").tempdir_in("/tmp").unwrap();
        let data = tempfile::tempdir().unwrap();
        let session = super::vscode_session_dir(data.path(), "fedcba9876543210").unwrap();
        let link = super::vscode_session_link(&session, temp.path()).unwrap();
        assert_eq!(super::vscode_session_dir(data.path(), "fedcba9876543210").unwrap(), session);
        assert_eq!(super::vscode_session_link(&session, temp.path()).unwrap(), link);
        for directory in [&session, session.parent().unwrap(), link.parent().unwrap()] {
            assert_eq!(fs::metadata(directory).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }

    #[cfg(unix)]
    #[test]
    fn vscode_session_survives_temp_cleanup() {
        let temp = tempfile::Builder::new().prefix("fl-").tempdir_in("/tmp").unwrap();
        let data = tempfile::tempdir().unwrap();
        let session = super::vscode_session_dir(data.path(), "0123456789abcdef").unwrap();
        let link = super::vscode_session_link(&session, temp.path()).unwrap();
        let files =
            ["user-data/User/settings.json", "user-data/User/History/entry", "extensions/entry"];
        for file in files {
            let path = link.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, file).unwrap();
        }
        fs::remove_dir_all(temp.path()).unwrap();
        for file in files {
            assert_eq!(fs::read_to_string(session.join(file)).unwrap(), file);
        }
        fs::create_dir(temp.path()).unwrap();
        assert_eq!(super::vscode_session_link(&session, temp.path()).unwrap(), link);
        for file in files {
            assert_eq!(fs::read_to_string(link.join(file)).unwrap(), file);
        }
    }
}
