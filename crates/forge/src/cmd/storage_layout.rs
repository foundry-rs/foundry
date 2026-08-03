//! Semantic storage-layout compatibility checks.

use alloy_primitives::U256;
use eyre::{Context, Result, eyre};
use foundry_common::shell;
use foundry_compilers::{
    ProjectCompileOutput,
    artifacts::{Storage, StorageLayout, StorageType},
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
    str::FromStr,
};

/// Result of comparing a reference storage layout with a current compiler layout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageLayoutCheck {
    /// Whether compatibility was proven.
    pub compatible: bool,
    /// Overall outcome of the comparison.
    pub status: CheckStatus,
    /// Reference file used for the comparison.
    pub reference: String,
    /// Number of existing storage variables checked.
    pub checked: usize,
    /// Compatible variables appended after the reference layout.
    pub appended: Vec<AppendedStorage>,
    /// Compatibility errors or limitations discovered by the check.
    pub issues: Vec<StorageLayoutIssue>,
}

impl StorageLayoutCheck {
    /// Returns whether the current layout is compatible with the reference.
    pub const fn is_compatible(&self) -> bool {
        self.compatible
    }
}

/// Overall storage-layout check outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Compatible,
    Incompatible,
    Unverifiable,
}

/// A compatible variable appended to the reference layout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendedStorage {
    pub label: String,
    pub slot: String,
    pub offset: i64,
    #[serde(rename = "type")]
    pub storage_type: String,
}

/// One incompatibility or limitation found while checking a storage layout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageLayoutIssue {
    pub kind: IssueKind,
    pub path: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
}

impl StorageLayoutIssue {
    fn incompatible(
        kind: IssueKind,
        path: impl Into<String>,
        message: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            path: path.into(),
            message: message.into(),
            expected: Some(expected.into()),
            actual: Some(actual.into()),
        }
    }

    fn unverifiable(kind: IssueKind, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { kind, path: path.into(), message: message.into(), expected: None, actual: None }
    }

    const fn is_unverifiable(&self) -> bool {
        matches!(
            self.kind,
            IssueKind::InvalidStorageMetadata
                | IssueKind::MissingTypeMetadata
                | IssueKind::NamespacedStorageUnsupported
                | IssueKind::AmbiguousVariableIdentity
                | IssueKind::UnknownTypeMetadata
        )
    }
}

/// Machine-readable category for a storage-layout issue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    VariableRemoved,
    SlotChanged,
    OffsetChanged,
    TypeChanged,
    InheritanceChanged,
    UnsafeAppend,
    InvalidStorageMetadata,
    MissingTypeMetadata,
    AmbiguousVariableIdentity,
    UnknownTypeMetadata,
    NamespacedStorageUnsupported,
}

/// Reads either a raw compiler storage layout or the `storageLayout` field of `.clone.meta`.
pub fn read_storage_layout(path: &Path) -> Result<StorageLayout> {
    let input = fs::read_to_string(path).wrap_err_with(|| {
        format!("failed to read storage layout reference `{}`", path.display())
    })?;
    let value: Value = serde_json::from_str(&input).wrap_err_with(|| {
        format!("failed to parse storage layout reference `{}`", path.display())
    })?;
    let value = value.get("storageLayout").unwrap_or(&value).clone();
    serde_json::from_value(value).wrap_err_with(|| {
        format!(
            "`{}` is neither a compiler storage layout nor a `.clone.meta` file",
            path.display()
        )
    })
}

/// Returns whether the target source or one of its imports declares EIP-7201 storage.
pub fn contains_namespaced_storage(
    output: &ProjectCompileOutput,
    target_path: &Path,
) -> Result<bool> {
    static MARKER: &str = "@custom:storage-location";
    let paths = std::iter::once(target_path).chain(output.graph().imports(target_path));
    for path in paths {
        let source = fs::read_to_string(path)
            .wrap_err_with(|| format!("failed to inspect source `{}`", path.display()))?;
        if source
            .match_indices(MARKER)
            .any(|(index, _)| source[index + MARKER.len()..].trim_start().starts_with("erc7201:"))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Compares two compiler storage layouts semantically.
pub fn check_storage_layout(
    reference: &StorageLayout,
    current: &StorageLayout,
    reference_path: &Path,
    has_namespaced_storage: bool,
) -> StorageLayoutCheck {
    let mut comparison = Comparison::new(reference, current);
    comparison.compare();

    if has_namespaced_storage {
        comparison.issues.push(StorageLayoutIssue::unverifiable(
            IssueKind::NamespacedStorageUnsupported,
            "storage",
            "EIP-7201 storage was found, but compiler storage-layout output does not expose \
             namespace roots; namespaced compatibility cannot be proven",
        ));
    }

    let has_unverifiable = comparison.issues.iter().any(StorageLayoutIssue::is_unverifiable);
    let has_incompatible = comparison.issues.iter().any(|issue| !issue.is_unverifiable());
    let status = if has_incompatible {
        CheckStatus::Incompatible
    } else if has_unverifiable {
        CheckStatus::Unverifiable
    } else {
        CheckStatus::Compatible
    };

    StorageLayoutCheck {
        compatible: status == CheckStatus::Compatible,
        status,
        reference: reference_path.display().to_string(),
        checked: comparison.checked,
        appended: comparison.appended,
        issues: comparison.issues,
    }
}

/// Writes the compatibility report to stdout.
pub fn write_check_report(report: &StorageLayoutCheck) -> Result<()> {
    if shell::is_json() {
        sh_println!("{}", serde_json::to_string_pretty(report)?)?;
        return Ok(());
    }

    match report.status {
        CheckStatus::Compatible => {
            sh_println!("Storage layout is compatible.")?;
            sh_println!(
                "Checked {} existing variable(s); found {} compatible append(s).",
                report.checked,
                report.appended.len()
            )?;
        }
        CheckStatus::Incompatible => {
            sh_println!("Storage layout is incompatible:")?;
            write_issues(&report.issues)?;
        }
        CheckStatus::Unverifiable => {
            sh_println!("Storage layout compatibility could not be verified:")?;
            write_issues(&report.issues)?;
        }
    }
    Ok(())
}

fn write_issues(issues: &[StorageLayoutIssue]) -> Result<()> {
    for issue in issues {
        sh_println!("- {}: {}", issue.path, issue.message)?;
    }
    Ok(())
}

struct Comparison<'a> {
    reference: &'a StorageLayout,
    current: &'a StorageLayout,
    issues: Vec<StorageLayoutIssue>,
    appended: Vec<AppendedStorage>,
    compared_types: HashSet<(String, String)>,
    matched_current: HashSet<usize>,
    checked: usize,
}

impl<'a> Comparison<'a> {
    fn new(reference: &'a StorageLayout, current: &'a StorageLayout) -> Self {
        Self {
            reference,
            current,
            issues: Vec::new(),
            appended: Vec::new(),
            compared_types: HashSet::new(),
            matched_current: HashSet::new(),
            checked: 0,
        }
    }

    fn compare(&mut self) {
        self.check_ambiguous_variables();

        for (index, expected) in self.reference.storage.iter().enumerate() {
            let path = storage_path(index, expected);
            let matching = self.current.storage.iter().enumerate().find(|(index, actual)| {
                !self.matched_current.contains(index) && actual.label == expected.label
            });
            let Some((current_index, actual)) = matching else {
                self.issues.push(StorageLayoutIssue::incompatible(
                    IssueKind::VariableRemoved,
                    path,
                    format!("existing variable `{}` was removed", expected.label),
                    describe_storage(expected),
                    "<missing>",
                ));
                continue;
            };

            self.matched_current.insert(current_index);
            self.checked += 1;
            if current_index != index {
                self.issues.push(StorageLayoutIssue::incompatible(
                    IssueKind::InheritanceChanged,
                    &path,
                    "storage declaration order changed; this can indicate an unsafe inheritance change",
                    index.to_string(),
                    current_index.to_string(),
                ));
            }
            self.compare_entry(expected, actual, &path);
        }

        self.compare_appends();
    }

    fn check_ambiguous_variables(&mut self) {
        for (source, layout) in [("reference", self.reference), ("current", self.current)] {
            let mut counts = BTreeMap::new();
            for storage in &layout.storage {
                *counts.entry(storage.label.as_str()).or_insert(0) += 1;
            }
            for (label, count) in counts {
                if count > 1 {
                    self.issues.push(StorageLayoutIssue::unverifiable(
                        IssueKind::AmbiguousVariableIdentity,
                        "storage",
                        format!(
                            "{source} layout contains {count} variables named `{label}`; compiler \
                             metadata cannot distinguish them"
                        ),
                    ));
                }
            }
        }
    }

    fn compare_entry(&mut self, expected: &Storage, actual: &Storage, path: &str) {
        match (parse_slot(&expected.slot), parse_slot(&actual.slot)) {
            (Ok(expected_slot), Ok(actual_slot)) if expected_slot != actual_slot => {
                self.issues.push(StorageLayoutIssue::incompatible(
                    IssueKind::SlotChanged,
                    path,
                    "storage slot changed",
                    expected.slot.clone(),
                    actual.slot.clone(),
                ));
            }
            (Err(error), _) => self.invalid_metadata(path, error),
            (_, Err(error)) => self.invalid_metadata(path, error),
            _ => {}
        }

        if expected.offset != actual.offset {
            self.issues.push(StorageLayoutIssue::incompatible(
                IssueKind::OffsetChanged,
                path,
                "storage offset changed",
                expected.offset.to_string(),
                actual.offset.to_string(),
            ));
        }

        self.compare_type(&expected.storage_type, &actual.storage_type, path);
    }

    fn compare_appends(&mut self) {
        let appended = self
            .current
            .storage
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.matched_current.contains(index))
            .collect::<Vec<_>>();
        if appended.is_empty() {
            return;
        }

        let boundary = if self.reference.storage.is_empty() {
            Some(StoragePosition { slot: U256::ZERO, offset: 0 })
        } else {
            self.reference
                .storage
                .iter()
                .filter_map(|storage| {
                    storage_end(storage, &self.reference.types)
                        .map_err(|error| {
                            self.invalid_metadata(&storage.label, error);
                        })
                        .ok()
                })
                .max()
        };

        let Some(boundary) = boundary else {
            self.issues.push(StorageLayoutIssue::unverifiable(
                IssueKind::InvalidStorageMetadata,
                "storage",
                "could not determine the end of the reference layout",
            ));
            return;
        };

        for (index, storage) in appended {
            let path = storage_path(index, storage);
            match storage_start(storage) {
                Ok(start) if start >= boundary => {
                    self.appended.push(AppendedStorage {
                        label: storage.label.clone(),
                        slot: storage.slot.clone(),
                        offset: storage.offset,
                        storage_type: type_label(&storage.storage_type, &self.current.types),
                    });
                }
                Ok(_) => self.issues.push(StorageLayoutIssue::incompatible(
                    IssueKind::UnsafeAppend,
                    path,
                    "new variable overlaps or precedes existing storage",
                    format!("at or after {}", boundary.describe()),
                    describe_storage(storage),
                )),
                Err(error) => self.invalid_metadata(&path, error),
            }
        }
    }

    fn compare_type(&mut self, expected_id: &str, actual_id: &str, path: &str) {
        if !self.compared_types.insert((expected_id.to_string(), actual_id.to_string())) {
            return;
        }

        let Some(expected) = self.reference.types.get(expected_id) else {
            self.missing_type(path, expected_id, "reference");
            return;
        };
        let Some(actual) = self.current.types.get(actual_id) else {
            self.missing_type(path, actual_id, "current");
            return;
        };

        let expected_kind = type_kind(expected_id, expected);
        let actual_kind = type_kind(actual_id, actual);
        if expected_kind != actual_kind {
            self.type_changed(path, "storage type kind changed", expected_kind, actual_kind);
            return;
        }

        if expected.encoding != actual.encoding {
            self.type_changed(
                path,
                "storage encoding changed",
                &expected.encoding,
                &actual.encoding,
            );
        }

        match (expected.number_of_bytes.parse::<u64>(), actual.number_of_bytes.parse::<u64>()) {
            (Ok(expected_bytes), Ok(actual_bytes)) if expected_bytes != actual_bytes => {
                self.type_changed(
                    path,
                    "storage byte width changed",
                    expected.number_of_bytes.as_str(),
                    actual.number_of_bytes.as_str(),
                );
            }
            (Err(_), _) | (_, Err(_)) => self.issues.push(StorageLayoutIssue::unverifiable(
                IssueKind::InvalidStorageMetadata,
                path,
                "storage type has an invalid `numberOfBytes` value",
            )),
            _ => {}
        }

        if should_compare_label(expected_kind, expected) && expected.label != actual.label {
            self.type_changed(
                path,
                "storage type changed",
                expected.label.as_str(),
                actual.label.as_str(),
            );
        }

        if matches!(expected_kind, "enum" | "user_defined_value_type") {
            self.issues.push(StorageLayoutIssue::unverifiable(
                IssueKind::UnknownTypeMetadata,
                path,
                format!(
                    "compiler storage-layout output does not describe the definition of \
                     `{expected_kind}` types"
                ),
            ));
        }

        self.compare_type_reference(expected.key.as_deref(), actual.key.as_deref(), path, "key");
        self.compare_type_reference(
            expected.value.as_deref(),
            actual.value.as_deref(),
            path,
            "value",
        );
        self.compare_other_type_reference(expected, actual, path, "base");
        self.compare_members(expected, actual, path);
        self.check_unknown_metadata(expected, path, "reference");
        self.check_unknown_metadata(actual, path, "current");
    }

    fn compare_type_reference(
        &mut self,
        expected: Option<&str>,
        actual: Option<&str>,
        path: &str,
        component: &str,
    ) {
        match (expected, actual) {
            (Some(expected), Some(actual)) => {
                self.compare_type(expected, actual, &format!("{path}.{component}"));
            }
            (None, None) => {}
            _ => self.type_changed(
                path,
                format!("storage type {component} changed"),
                expected.unwrap_or("<none>"),
                actual.unwrap_or("<none>"),
            ),
        }
    }

    fn compare_other_type_reference(
        &mut self,
        expected: &StorageType,
        actual: &StorageType,
        path: &str,
        key: &str,
    ) {
        let expected = metadata_string(expected, key);
        let actual = metadata_string(actual, key);
        match (expected, actual) {
            (Ok(expected), Ok(actual)) => {
                self.compare_type_reference(expected, actual, path, key);
            }
            (Err(error), _) | (_, Err(error)) => {
                self.issues.push(StorageLayoutIssue::unverifiable(
                    IssueKind::InvalidStorageMetadata,
                    path,
                    error.to_string(),
                ))
            }
        }
    }

    fn compare_members(&mut self, expected: &StorageType, actual: &StorageType, path: &str) {
        let expected = storage_members(expected);
        let actual = storage_members(actual);
        let (Ok(expected), Ok(actual)) = (expected, actual) else {
            self.issues.push(StorageLayoutIssue::unverifiable(
                IssueKind::InvalidStorageMetadata,
                path,
                "storage type has invalid `members` metadata",
            ));
            return;
        };

        if expected.len() != actual.len() {
            self.type_changed(
                path,
                "storage member count changed",
                expected.len().to_string(),
                actual.len().to_string(),
            );
        }

        let mut matched = HashSet::new();
        for (index, expected) in expected.iter().enumerate() {
            let member_path = format!("{path}.member[{index}]");
            let matching = actual
                .iter()
                .enumerate()
                .find(|(index, actual)| !matched.contains(index) && actual.label == expected.label);
            let Some((actual_index, actual)) = matching else {
                self.type_changed(
                    &member_path,
                    format!("storage member `{}` was removed", expected.label),
                    describe_storage(expected),
                    "<missing>",
                );
                continue;
            };
            matched.insert(actual_index);
            self.compare_entry(expected, actual, &member_path);
        }
    }

    fn check_unknown_metadata(&mut self, storage_type: &StorageType, path: &str, source: &str) {
        let unknown = storage_type
            .other
            .keys()
            .filter(|key| key.as_str() != "base" && key.as_str() != "members")
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            self.issues.push(StorageLayoutIssue::unverifiable(
                IssueKind::UnknownTypeMetadata,
                path,
                format!(
                    "{source} compiler layout contains unsupported type metadata: {}",
                    unknown.join(", ")
                ),
            ));
        }
    }

    fn invalid_metadata(&mut self, path: &str, error: impl std::fmt::Display) {
        self.issues.push(StorageLayoutIssue::unverifiable(
            IssueKind::InvalidStorageMetadata,
            path,
            error.to_string(),
        ));
    }

    fn missing_type(&mut self, path: &str, storage_type: &str, source: &str) {
        self.issues.push(StorageLayoutIssue::unverifiable(
            IssueKind::MissingTypeMetadata,
            path,
            format!("{source} layout is missing metadata for type `{storage_type}`"),
        ));
    }

    fn type_changed(
        &mut self,
        path: &str,
        message: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) {
        self.issues.push(StorageLayoutIssue::incompatible(
            IssueKind::TypeChanged,
            path,
            message,
            expected,
            actual,
        ));
    }
}

fn parse_slot(slot: &str) -> Result<U256> {
    U256::from_str(slot).map_err(|_| eyre!("invalid storage slot `{slot}`"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct StoragePosition {
    slot: U256,
    offset: u64,
}

impl StoragePosition {
    fn describe(self) -> String {
        format!("slot {}, offset {}", self.slot, self.offset)
    }
}

fn storage_start(storage: &Storage) -> Result<StoragePosition> {
    let offset = u64::try_from(storage.offset)
        .map_err(|_| eyre!("invalid negative storage offset `{}`", storage.offset))?;
    if offset >= 32 {
        return Err(eyre!("invalid storage offset `{offset}`"));
    }
    Ok(StoragePosition { slot: parse_slot(&storage.slot)?, offset })
}

fn storage_end(
    storage: &Storage,
    types: &BTreeMap<String, StorageType>,
) -> Result<StoragePosition> {
    let start = storage_start(storage)?;
    let storage_type = types
        .get(&storage.storage_type)
        .ok_or_else(|| eyre!("missing metadata for storage type `{}`", storage.storage_type))?;
    let bytes = storage_type
        .number_of_bytes
        .parse::<u64>()
        .map_err(|_| eyre!("invalid storage byte width `{}`", storage_type.number_of_bytes))?;
    let total =
        start.offset.checked_add(bytes).ok_or_else(|| eyre!("storage byte width overflow"))?;
    let slot = start
        .slot
        .checked_add(U256::from(total / 32))
        .ok_or_else(|| eyre!("storage slot overflow"))?;
    Ok(StoragePosition { slot, offset: total % 32 })
}

fn storage_path(index: usize, storage: &Storage) -> String {
    format!("storage[{index}] (`{}`)", storage.label)
}

fn describe_storage(storage: &Storage) -> String {
    format!("slot {}, offset {}, type {}", storage.slot, storage.offset, storage.storage_type)
}

fn type_label(storage_type: &str, types: &BTreeMap<String, StorageType>) -> String {
    types.get(storage_type).map_or_else(|| storage_type.to_string(), |ty| ty.label.clone())
}

fn type_kind<'a>(id: &'a str, storage_type: &'a StorageType) -> &'a str {
    if id.starts_with("t_struct(") {
        "struct"
    } else if id.starts_with("t_enum(") {
        "enum"
    } else if id.starts_with("t_contract(") {
        "contract"
    } else if id.starts_with("t_userDefinedValueType(") {
        "user_defined_value_type"
    } else if id.starts_with("t_array(") {
        "array"
    } else if id.starts_with("t_mapping(") {
        "mapping"
    } else {
        storage_type.label.as_str()
    }
}

fn should_compare_label(kind: &str, storage_type: &StorageType) -> bool {
    kind != "struct"
        && storage_type.encoding != "mapping"
        && storage_type.encoding != "dynamic_array"
}

fn metadata_string<'a>(storage_type: &'a StorageType, key: &str) -> Result<Option<&'a str>> {
    storage_type
        .other
        .get(key)
        .map(|value| {
            value.as_str().ok_or_else(|| eyre!("storage type metadata `{key}` is not a string"))
        })
        .transpose()
}

fn storage_members(storage_type: &StorageType) -> Result<Vec<Storage>> {
    storage_type
        .other
        .get("members")
        .map(|value| serde_json::from_value(value.clone()).map_err(Into::into))
        .transpose()
        .map(Option::unwrap_or_default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn storage(label: &str, slot: &str, offset: i64, storage_type: &str) -> Storage {
        Storage {
            ast_id: 1,
            contract: "src/Example.sol:Example".to_string(),
            label: label.to_string(),
            offset,
            slot: slot.to_string(),
            storage_type: storage_type.to_string(),
        }
    }

    fn primitive(label: &str, bytes: &str) -> StorageType {
        StorageType {
            encoding: "inplace".to_string(),
            key: None,
            label: label.to_string(),
            number_of_bytes: bytes.to_string(),
            value: None,
            other: BTreeMap::new(),
        }
    }

    fn layout(
        storage: Vec<Storage>,
        types: impl IntoIterator<Item = (&'static str, StorageType)>,
    ) -> StorageLayout {
        StorageLayout {
            storage,
            types: types.into_iter().map(|(id, ty)| (id.to_string(), ty)).collect(),
        }
    }

    fn check(reference: &StorageLayout, current: &StorageLayout) -> StorageLayoutCheck {
        check_storage_layout(reference, current, Path::new("layout.json"), false)
    }

    #[test]
    fn accepts_packed_append() {
        let reference = layout(
            vec![storage("first", "0", 0, "t_uint128")],
            [("t_uint128", primitive("uint128", "16"))],
        );
        let mut current = layout(
            vec![storage("first", "0x0", 0, "t_uint128"), storage("second", "0", 16, "t_uint128")],
            [("t_uint128", primitive("uint128", "16"))],
        );
        current.storage[0].ast_id = 999;
        current.storage[0].contract = "contracts/Renamed.sol:ExampleV2".to_string();

        let report = check(&reference, &current);
        assert_eq!(report.status, CheckStatus::Compatible);
        assert_eq!(report.appended.len(), 1);
    }

    #[test]
    fn rejects_inheritance_reordering_with_equal_width_types() {
        let reference = layout(
            vec![storage("fromA", "0", 0, "t_uint256"), storage("fromB", "1", 0, "t_uint256")],
            [("t_uint256", primitive("uint256", "32"))],
        );
        let current = layout(
            vec![storage("fromB", "0", 0, "t_uint256"), storage("fromA", "1", 0, "t_uint256")],
            [("t_uint256", primitive("uint256", "32"))],
        );

        let report = check(&reference, &current);
        assert_eq!(report.status, CheckStatus::Incompatible);
        assert_eq!(
            report.issues.iter().filter(|issue| issue.kind == IssueKind::SlotChanged).count(),
            2
        );
        assert_eq!(
            report
                .issues
                .iter()
                .filter(|issue| issue.kind == IssueKind::InheritanceChanged)
                .count(),
            2
        );
    }

    #[test]
    fn rejects_slot_offset_and_type_changes() {
        let reference = layout(
            vec![storage("value", "0", 0, "t_uint256")],
            [("t_uint256", primitive("uint256", "32"))],
        );
        let current = layout(
            vec![storage("value", "1", 1, "t_bytes32")],
            [("t_bytes32", primitive("bytes32", "32"))],
        );

        let report = check(&reference, &current);
        assert_eq!(report.status, CheckStatus::Incompatible);
        assert_eq!(
            report.issues.iter().map(|issue| issue.kind).collect::<Vec<_>>(),
            [IssueKind::SlotChanged, IssueKind::OffsetChanged, IssueKind::TypeChanged]
        );
    }

    #[test]
    fn compares_structs_without_ast_ids_or_type_names() {
        let old_members =
            json!([storage("value", "0", 0, "t_uint256"), storage("flag", "1", 0, "t_bool")]);
        let new_members =
            json!([storage("value", "0", 0, "t_uint256"), storage("flag", "1", 0, "t_bool")]);
        let mut old_struct = primitive("struct Old.Data", "64");
        old_struct.other.insert("members".to_string(), old_members);
        let mut new_struct = primitive("struct New.Data", "64");
        new_struct.other.insert("members".to_string(), new_members);
        let reference = layout(
            vec![storage("data", "0", 0, "t_struct(Data)10_storage")],
            [
                ("t_struct(Data)10_storage", old_struct),
                ("t_uint256", primitive("uint256", "32")),
                ("t_bool", primitive("bool", "1")),
            ],
        );
        let current = layout(
            vec![storage("data", "0", 0, "t_struct(Renamed)99_storage")],
            [
                ("t_struct(Renamed)99_storage", new_struct),
                ("t_uint256", primitive("uint256", "32")),
                ("t_bool", primitive("bool", "1")),
            ],
        );

        assert_eq!(check(&reference, &current).status, CheckStatus::Compatible);
    }

    #[test]
    fn rejects_struct_member_change() {
        let mut old_struct = primitive("struct Data", "32");
        old_struct
            .other
            .insert("members".to_string(), json!([storage("value", "0", 0, "t_uint256")]));
        let mut new_struct = primitive("struct Data", "32");
        new_struct
            .other
            .insert("members".to_string(), json!([storage("value", "0", 0, "t_bytes32")]));
        let reference = layout(
            vec![storage("data", "0", 0, "t_struct(Data)10_storage")],
            [("t_struct(Data)10_storage", old_struct), ("t_uint256", primitive("uint256", "32"))],
        );
        let current = layout(
            vec![storage("data", "0", 0, "t_struct(Data)99_storage")],
            [("t_struct(Data)99_storage", new_struct), ("t_bytes32", primitive("bytes32", "32"))],
        );

        let report = check(&reference, &current);
        assert_eq!(report.status, CheckStatus::Incompatible);
        assert!(report.issues.iter().any(
            |issue| issue.path.ends_with(".member[0]") && issue.kind == IssueKind::TypeChanged
        ));
    }

    #[test]
    fn missing_type_metadata_is_unverifiable() {
        let reference = layout(vec![storage("value", "0", 0, "t_uint256")], []);
        let current = layout(
            vec![storage("value", "0", 0, "t_uint256")],
            [("t_uint256", primitive("uint256", "32"))],
        );

        let report = check(&reference, &current);
        assert_eq!(report.status, CheckStatus::Unverifiable);
        assert!(!report.compatible);
    }

    #[test]
    fn opaque_user_defined_types_are_unverifiable() {
        let reference = layout(
            vec![storage("status", "0", 0, "t_enum(Status)10")],
            [("t_enum(Status)10", primitive("enum Example.Status", "1"))],
        );
        let current = layout(
            vec![storage("status", "0", 0, "t_enum(Status)99")],
            [("t_enum(Status)99", primitive("enum Example.Status", "1"))],
        );

        let report = check(&reference, &current);
        assert_eq!(report.status, CheckStatus::Unverifiable);
        assert_eq!(report.issues[0].kind, IssueKind::UnknownTypeMetadata);
    }
}
