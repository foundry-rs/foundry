//! Versioned, typed artifacts with caller-owned metadata and payloads.
//!
//! This module defines only the common storage envelope. Callers retain ownership of paths,
//! artifact schema versions, validity decisions, and merge or locking behavior. This allows unit
//! test failures, fuzz and invariant counterexamples, symbolic results, mutation checkpoints, and
//! corpora to share publication mechanics without pretending their reuse rules are equivalent.

use crate::{
    errors::FsPathError,
    fs::{self, PublishMode},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};

const STORE_VERSION: u32 = 1;

/// Identifies a caller-owned artifact schema.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtifactSchema {
    /// Stable schema identifier.
    pub id: &'static str,
    /// Version of the caller-owned metadata and payload format.
    pub version: u32,
}

impl ArtifactSchema {
    /// Creates an artifact schema descriptor.
    pub const fn new(id: &'static str, version: u32) -> Self {
        Self { id, version }
    }
}

/// A versioned artifact containing caller-owned validity metadata and payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactFile<M, T> {
    schema: ArtifactSchema,
    /// Metadata used by the caller to decide whether this artifact remains valid.
    pub metadata: M,
    /// Mode-specific artifact contents.
    pub payload: T,
}

impl<M, T> ArtifactFile<M, T> {
    /// Creates an artifact using `schema`.
    pub const fn new(schema: ArtifactSchema, metadata: M, payload: T) -> Self {
        Self { schema, metadata, payload }
    }

    /// Returns this artifact's schema.
    pub const fn schema(&self) -> ArtifactSchema {
        self.schema
    }
}

impl<M: DeserializeOwned, T: DeserializeOwned> ArtifactFile<M, T> {
    /// Reads an artifact if `path` exists and its store and caller schemas are supported.
    ///
    /// Metadata and payload compatibility beyond the declared schema is intentionally left to the
    /// caller. The header is validated before either caller-owned type is deserialized.
    pub fn read(path: &Path, expected: ArtifactSchema) -> Result<Option<Self>, ArtifactReadError> {
        let encoded = match std::fs::read(path) {
            Ok(encoded) => encoded,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(FsPathError::read(err, path).into()),
        };
        let header = serde_json::from_slice::<ArtifactHeader>(&encoded)
            .map_err(|source| FsPathError::ReadJson { source, path: path.into() })?;

        if header.store_version != STORE_VERSION {
            return Err(ArtifactReadError::UnsupportedStoreVersion {
                path: path.into(),
                expected: STORE_VERSION,
                found: header.store_version,
            });
        }
        if header.schema != expected.id {
            return Err(ArtifactReadError::UnexpectedSchema {
                path: path.into(),
                expected: expected.id,
                found: header.schema,
            });
        }
        if header.schema_version != expected.version {
            return Err(ArtifactReadError::UnsupportedSchemaVersion {
                path: path.into(),
                schema: expected.id,
                expected: expected.version,
                found: header.schema_version,
            });
        }

        let envelope = serde_json::from_slice::<ArtifactEnvelope<M, T>>(&encoded)
            .map_err(|source| FsPathError::ReadJson { source, path: path.into() })?;
        Ok(Some(Self::new(expected, envelope.metadata, envelope.payload)))
    }
}

impl<M: Serialize, T: Serialize> ArtifactFile<M, T> {
    /// Atomically publishes this artifact at `path`.
    pub fn write_atomic(&self, path: &Path, mode: PublishMode) -> fs::Result<()> {
        fs::write_json_file_atomic(
            path,
            &ArtifactEnvelopeRef {
                store_version: STORE_VERSION,
                schema: self.schema.id,
                schema_version: self.schema.version,
                metadata: &self.metadata,
                payload: &self.payload,
            },
            mode,
        )
    }
}

#[derive(Deserialize)]
struct ArtifactHeader {
    store_version: u32,
    schema: String,
    schema_version: u32,
}

#[derive(Deserialize)]
struct ArtifactEnvelope<M, T> {
    metadata: M,
    payload: T,
}

#[derive(Serialize)]
struct ArtifactEnvelopeRef<'a, M, T> {
    store_version: u32,
    schema: &'static str,
    schema_version: u32,
    metadata: &'a M,
    payload: &'a T,
}

/// Errors produced while reading a versioned artifact.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum ArtifactReadError {
    /// The artifact could not be read or decoded.
    #[error(transparent)]
    File(#[from] FsPathError),
    /// The common artifact envelope version is unsupported.
    #[error("unsupported artifact store version {found} in {path:?}; expected version {expected}")]
    UnsupportedStoreVersion { path: PathBuf, expected: u32, found: u32 },
    /// The file contains a different artifact schema.
    #[error("unexpected artifact schema {found:?} in {path:?}; expected {expected:?}")]
    UnexpectedSchema { path: PathBuf, expected: &'static str, found: String },
    /// The caller-owned artifact schema version is unsupported.
    #[error(
        "unsupported {schema:?} artifact schema version {found} in {path:?}; expected version {expected}"
    )]
    UnsupportedSchemaVersion { path: PathBuf, schema: &'static str, expected: u32, found: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    const SCHEMA: ArtifactSchema = ArtifactSchema::new("forge.test", 3);

    #[derive(Debug, Deserialize, Serialize)]
    struct Metadata {
        build: String,
    }

    #[test]
    fn artifact_round_trip_supports_caller_owned_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifact.json");
        let artifact = ArtifactFile::new(
            SCHEMA,
            Metadata { build: "abc".to_string() },
            vec!["failure", "counterexample"],
        );

        artifact.write_atomic(&path, PublishMode::Replace).unwrap();
        let loaded = ArtifactFile::<Metadata, Vec<String>>::read(&path, SCHEMA).unwrap().unwrap();

        assert_eq!(loaded.schema(), SCHEMA);
        assert_eq!(loaded.metadata.build, "abc");
        assert_eq!(loaded.payload, ["failure", "counterexample"]);
    }

    #[test]
    fn missing_artifact_is_distinct_from_invalid_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.json");
        assert!(ArtifactFile::<(), ()>::read(&path, SCHEMA).unwrap().is_none());

        std::fs::write(&path, b"not json").unwrap();
        assert!(matches!(
            ArtifactFile::<(), ()>::read(&path, SCHEMA),
            Err(ArtifactReadError::File(FsPathError::ReadJson { .. }))
        ));
    }

    #[test]
    fn header_is_validated_before_payload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifact.json");
        std::fs::write(
            &path,
            r#"{"store_version":2,"schema":"other","schema_version":9,"metadata":{},"payload":{}}"#,
        )
        .unwrap();

        assert!(matches!(
            ArtifactFile::<String, String>::read(&path, SCHEMA),
            Err(ArtifactReadError::UnsupportedStoreVersion { found: 2, .. })
        ));

        std::fs::write(
            &path,
            r#"{"store_version":1,"schema":"other","schema_version":9,"metadata":{},"payload":{}}"#,
        )
        .unwrap();
        assert!(matches!(
            ArtifactFile::<String, String>::read(&path, SCHEMA),
            Err(ArtifactReadError::UnexpectedSchema { .. })
        ));

        std::fs::write(
            &path,
            r#"{"store_version":1,"schema":"forge.test","schema_version":9,"metadata":{},"payload":{}}"#,
        )
        .unwrap();
        assert!(matches!(
            ArtifactFile::<String, String>::read(&path, SCHEMA),
            Err(ArtifactReadError::UnsupportedSchemaVersion { found: 9, .. })
        ));
    }
}
