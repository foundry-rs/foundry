use clap::Parser;
use foundry_common::traits::{TestFilter, FunctionFilter, TestFunctionExt};
use foundry_cli::utils::FoundryPathExt;
use foundry_common::glob::GlobMatcher;
use foundry_config::Config;
use foundry_compilers::{FileFilter, ProjectPathsConfig};
use std::{fmt, path::Path};

