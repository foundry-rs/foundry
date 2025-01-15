use std::error::Error;

use foundry_common::version::set_build_version;
use vergen::EmitBuilder;

fn main() -> Result<(), Box<dyn Error>> {
    EmitBuilder::builder()
        .git_describe(false, true, None)
        .git_dirty(true)
        .git_sha(true)
        .build_timestamp()
        .emit_and_set()?;

    set_build_version()?;

    Ok(())
}
