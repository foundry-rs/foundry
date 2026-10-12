//! Persisted invariant failure data shared by execution backends.

use crate::result::SymbolicInvariantFailureSite;
use alloy_primitives::{Address, Selector};
use foundry_evm_fuzz::{BaseCounterExample, invariant::InvariantSettings};
use foundry_evm_symbolic::SymbolicStorageAssignment;
use serde::{Deserialize, Serialize};

/// Holds data about a persisted invariant failure.
#[derive(Serialize, Deserialize)]
pub(crate) struct InvariantPersistedFailure {
    /// Recorded counterexample.
    pub(crate) call_sequence: Vec<BaseCounterExample>,
    /// Invariant settings when the counterexample was generated.
    /// Used to determine if the counterexample is still valid.
    pub(crate) settings: InvariantSettings,
    /// Whether the persisted failure came from a handler assertion instead of the invariant body.
    #[serde(default)]
    pub(crate) assertion_failure: bool,
    /// Exact concrete execution failure, when available. Older records lack this witness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) execution_failure: Option<InvariantFailureIdentity>,
    /// Concrete setup-storage assignments required before replaying this failure.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) storage: Vec<SymbolicStorageAssignment>,
    /// Exact failure site required to accept a persisted symbolic handler rerun.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) failure_site: Option<SymbolicInvariantFailureSite>,
    /// Versioned configuration used to produce a reproducible edge fingerprint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) fingerprint_provenance: Option<PersistedFingerprintProvenance>,
}

/// Reproducible edge-fingerprint algorithms and their capture configuration.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "algorithm", rename_all = "snake_case")]
pub(crate) enum PersistedFingerprintProvenance {
    CollisionFreeV1 { include_call_depth: bool },
    HashV1 { include_call_depth: bool },
}

/// Observed concrete failure identity, independent of symbolic edge fingerprints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct InvariantFailureIdentity {
    pub(crate) site: InvariantFailureSite,
    pub(crate) reason: Option<String>,
}

/// Concrete execution phase and call identity that a replay must reproduce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "site", rename_all = "snake_case")]
pub(crate) enum InvariantFailureSite {
    Handler(Address, Address, Selector),
    Predicate(Selector),
    AfterInvariant,
}
