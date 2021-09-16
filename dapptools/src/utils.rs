use std::{
    fs::{File, OpenOptions},
    path::PathBuf,
};

/// Default deps path
const DEFAULT_OUT_FILE: &str = "dapp.sol.json";

/// Initializes a tracing Subscriber for logging
pub fn subscriber() {
    tracing_subscriber::FmtSubscriber::builder()
        // .with_timer(tracing_subscriber::fmt::time::uptime())
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        // don't need the target
        .with_target(false)
        .init();
}

/// Default to including all files under current directory in the allowed paths
pub fn default_path(path: Vec<String>) -> eyre::Result<Vec<String>> {
    Ok(if path.is_empty() {
        vec![".".to_owned()]
    } else {
        path
    })
}

/// merge the cli-provided remappings vector with the
/// new-line separated env var
pub fn merge(mut remappings: Vec<String>, remappings_env: Option<String>) -> Vec<String> {
    // merge the cli-provided remappings vector with the
    // new-line separated env var
    if let Some(env) = remappings_env {
        remappings.extend_from_slice(&env.split('\n').map(|x| x.to_string()).collect::<Vec<_>>());
        // deduplicate the extra remappings
        remappings.sort_unstable();
        remappings.dedup();
    }

    remappings
}

/// Opens the file at `out_path` for R/W and creates it if it doesn't exist.
pub fn open_file(out_path: PathBuf) -> eyre::Result<File> {
    Ok(if out_path.is_file() {
        // get the file if it exists
        OpenOptions::new().write(true).open(out_path)?
    } else if out_path.is_dir() {
        // get the directory if it exists & the default file path
        let out_path = out_path.join(DEFAULT_OUT_FILE);

        // get a file handler (overwrite any contents of the existing file)
        OpenOptions::new().write(true).create(true).open(out_path)?
    } else {
        // otherwise try to create the entire path

        // in case it's a directory, we must mkdir it
        let out_path = if out_path
            .to_str()
            .ok_or_else(|| eyre::eyre!("not utf-8 path"))?
            .ends_with('/')
        {
            std::fs::create_dir_all(&out_path)?;
            out_path.join(DEFAULT_OUT_FILE)
        } else {
            // if it's a file path, we must mkdir the parent
            let parent = out_path
                .parent()
                .ok_or_else(|| eyre::eyre!("could not get parent of {:?}", out_path))?;
            std::fs::create_dir_all(parent)?;
            out_path
        };

        // finally we get the handler
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out_path)?
    })
}
