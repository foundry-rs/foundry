//! Meta crate reexporting all artifacts types.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

pub use foundry_compilers_artifacts_solc as solc;
pub use foundry_compilers_artifacts_vyper as vyper;
pub use solc::*;
