use foundry_compilers::{
    Project, ProjectCompileOutput, Vyper, project_util::copy_dir, utils::RuntimeOrHandle,
};
use foundry_config::Config;
use std::{
    env,
    fs::{self, File},
    io::{IsTerminal, Read, Seek, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::LazyLock,
};

pub use crate::{ext::*, prj::*};

/// The commit of forge-std to use.
pub const FORGE_STD_REVISION: &str = include_str!("../../../testdata/forge-std-rev");

/// Stores whether `stdout` is a tty / terminal.
pub static IS_TTY: LazyLock<bool> = LazyLock::new(|| std::io::stdout().is_terminal());

/// Global default template path. Contains the global template project from which all other
/// temp projects are initialized. See [`initialize()`] for more info.
static TEMPLATE_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| env::temp_dir().join("foundry-forge-test-template"));

/// Global default template lock. If its contents are not exactly `"1"`, the global template will
/// be re-initialized. See [`initialize()`] for more info.
static TEMPLATE_LOCK: LazyLock<PathBuf> =
    LazyLock::new(|| env::temp_dir().join("foundry-forge-test-template.lock"));

/// The default Solc version used when compiling tests.
pub const SOLC_VERSION: &str = "0.8.30";

/// Another Solc version used when compiling tests.
///
/// Necessary to avoid downloading multiple versions.
pub const OTHER_SOLC_VERSION: &str = "0.8.26";

/// Initializes a project with `forge init` at the given path from a template directory.
///
/// This should be called after an empty project is created like in
/// [some of this crate's macros](crate::forgetest_init).
///
/// ## Note
///
/// This doesn't always run `forge init`, instead opting to copy an already-initialized template
/// project from a global template path. This is done to speed up tests.
///
/// This used to use a `static` `Lazy`, but this approach does not with `cargo-nextest` because it
/// runs each test in a separate process. Instead, we use a global lock file to ensure that only one
/// test can initialize the template at a time.
///
/// This sets the project's solc version to the [`SOLC_VERSION`].
pub fn initialize(target: &Path) {
    test_debug!("initializing {}", target.display());

    let tpath = TEMPLATE_PATH.as_path();
    pretty_err(tpath, fs::create_dir_all(tpath));

    // Initialize the global template if necessary.
    let mut lock = crate::fd_lock::new_lock(TEMPLATE_LOCK.as_path());
    let mut _read = lock.read().unwrap();
    if !crate::fd_lock::lock_exists(TEMPLATE_LOCK.as_path()) {
        // We are the first to acquire the lock:
        // - initialize a new empty temp project;
        // - run `forge init`;
        // - run `forge build`;
        // - copy it over to the global template;
        // Ideally we would be able to initialize a temp project directly in the global template,
        // but `TempProject` does not currently allow this: https://github.com/foundry-rs/compilers/issues/22

        // Release the read lock and acquire a write lock, initializing the lock file.
        drop(_read);
        let mut write = lock.write().unwrap();

        let mut data = Vec::new();
        write.read_to_end(&mut data).unwrap();
        if data != crate::fd_lock::LOCK_TOKEN {
            // Initialize and build.
            let (prj, mut cmd) = setup_forge("template", foundry_compilers::PathStyle::Dapptools);
            test_debug!("- initializing template dir in {}", prj.root().display());

            cmd.args(["init", "--force", "--empty"]).assert_success();
            prj.write_config(Config {
                solc: Some(foundry_config::SolcReq::Version(SOLC_VERSION.parse().unwrap())),
                ..Default::default()
            });

            // Checkout forge-std.
            let output = Command::new("git")
                .current_dir(prj.root().join("lib/forge-std"))
                .args(["checkout", FORGE_STD_REVISION])
                .output()
                .expect("failed to checkout forge-std");
            assert!(output.status.success(), "{output:#?}");

            // Build the project.
            cmd.forge_fuse().arg("build").assert_success();

            // Remove the existing template, if any.
            let _ = fs::remove_dir_all(tpath);

            // Copy the template to the global template path.
            pretty_err(tpath, copy_dir(prj.root(), tpath));

            // Update lockfile to mark that template is initialized.
            write.set_len(0).unwrap();
            write.seek(std::io::SeekFrom::Start(0)).unwrap();
            write.write_all(crate::fd_lock::LOCK_TOKEN).unwrap();
        }

        // Release the write lock and acquire a new read lock.
        drop(write);
        _read = lock.read().unwrap();
    }

    test_debug!("- copying template dir from {}", tpath.display());
    pretty_err(target, fs::create_dir_all(target));
    pretty_err(target, copy_dir(tpath, target));
}

/// Compile the project with a lock for the cache.
pub fn get_compiled(project: &mut Project) -> ProjectCompileOutput {
    let lock_file_path = project.sources_path().join(".lock");
    // We need to use a file lock because `cargo-nextest` runs tests in different processes.
    // This is similar to `initialize`, see its comments for more details.
    let mut lock = crate::fd_lock::new_lock(&lock_file_path);
    let read = lock.read().unwrap();
    let out;

    let mut write = None;
    if !project.cache_path().exists() || !crate::fd_lock::lock_exists(&lock_file_path) {
        drop(read);
        write = Some(lock.write().unwrap());
        test_debug!("cache miss for {}", lock_file_path.display());
    } else {
        test_debug!("cache hit for {}", lock_file_path.display());
    }

    if project.compiler.vyper.is_none() {
        project.compiler.vyper = Some(get_vyper());
    }

    test_debug!("compiling {}", lock_file_path.display());
    out = project.compile().unwrap();
    test_debug!("compiled {}", lock_file_path.display());

    if out.has_compiler_errors() {
        panic!("Compiled with errors:\n{out}");
    }

    if let Some(write) = &mut write {
        write.write_all(crate::fd_lock::LOCK_TOKEN).unwrap();
    }

    out
}

/// Installs Vyper if it's not already present.
pub fn get_vyper() -> Vyper {
    static VYPER: LazyLock<PathBuf> = LazyLock::new(|| std::env::temp_dir().join("vyper"));

    if let Ok(vyper) = Vyper::new("vyper") {
        return vyper;
    }
    if let Ok(vyper) = Vyper::new(&*VYPER) {
        return vyper;
    }
    return RuntimeOrHandle::new().block_on(install());

    async fn install() -> Vyper {
        #[cfg(target_family = "unix")]
        use std::{fs::Permissions, os::unix::fs::PermissionsExt};

        let path = VYPER.as_path();
        let mut file = File::create(path).unwrap();
        if let Err(e) = file.try_lock() {
            if let fs::TryLockError::WouldBlock = e {
                file.lock().unwrap();
                assert!(path.exists());
                return Vyper::new(path).unwrap();
            }
            file.lock().unwrap();
        }

        let suffix = match svm::platform() {
            svm::Platform::MacOsAarch64 => "darwin",
            svm::Platform::LinuxAmd64 => "linux",
            svm::Platform::WindowsAmd64 => "windows.exe",
            platform => panic!(
                "unsupported platform {platform:?} for installing vyper, \
                 install it manually and add it to $PATH"
            ),
        };
        let url = format!(
            "https://github.com/vyperlang/vyper/releases/download/v0.4.3/vyper.0.4.3+commit.bff19ea2.{suffix}"
        );

        test_debug!("downloading vyper from {url}");
        let res = reqwest::Client::builder().build().unwrap().get(url).send().await.unwrap();

        assert!(res.status().is_success());

        let bytes = res.bytes().await.unwrap();

        file.write_all(&bytes).unwrap();

        #[cfg(target_family = "unix")]
        file.set_permissions(Permissions::from_mode(0o755)).unwrap();

        Vyper::new(path).unwrap()
    }
}

#[track_caller]
pub fn pretty_err<T, E: std::error::Error>(path: impl AsRef<Path>, res: Result<T, E>) -> T {
    match res {
        Ok(t) => t,
        Err(err) => panic!("{}: {err}", path.as_ref().display()),
    }
}

pub fn read_string(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    pretty_err(path, std::fs::read_to_string(path))
}
