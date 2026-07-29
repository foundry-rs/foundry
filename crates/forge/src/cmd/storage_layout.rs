use alloy_primitives::U256;
use clap::Parser;
use eyre::{Context, Result};
use foundry_cli::opts::{BuildOpts, CompilerOpts};
use foundry_common::{
    compile::{PathOrContractInfo, ProjectCompiler},
    find_matching_contract_artifact, find_target_path, shell,
};
use foundry_compilers::artifacts::{
    Storage, StorageLayout, StorageType,
    output_selection::{ContractOutputSelection, OutputSelection},
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

const LAYOUT_SCOPE: &str = "compilerStorageLayout";
const LAYOUT_LIMITATION: &str = "Only entries in the compiler-reported `storage` array are \
    compared. Namespaced (including EIP-7201) and manually computed slots are outside this check \
    unless represented in that array; enum member changes and state-variable behavior are not \
    checked.";

/// CLI arguments for `forge storage-layout`.
#[derive(Clone, Debug, Parser)]
pub struct StorageLayoutArgs {
    /// The contract whose current storage layout should be checked.
    #[arg(value_parser = PathOrContractInfo::from_str)]
    pub contract: Option<PathOrContractInfo>,

    /// A previous compiler storage layout, artifact, or `.clone.meta` file.
    ///
    /// Defaults to `.clone.meta` in the project root.
    #[arg(long, short, value_hint = clap::ValueHint::FilePath, value_name = "PATH")]
    pub reference: Option<PathBuf>,

    /// Treat variable and struct-member renames as compatible.
    ///
    /// Without this flag, label changes fail because compiler metadata cannot distinguish a safe
    /// rename from reusing an existing slot for different state.
    #[arg(long)]
    pub allow_renames: bool,

    /// All build arguments are supported.
    #[command(flatten)]
    build: BuildOpts,
}

impl StorageLayoutArgs {
    pub fn run(self) -> Result<()> {
        let Self { contract, reference, allow_renames, build } = self;
        trace!(target: "forge", ?contract, ?reference, "checking storage layout compatibility");

        let user_extra_output = !build.compiler.extra_output.is_empty()
            || !build.compiler.extra_output_files.is_empty();
        let mut extra_output = build.compiler.extra_output;
        if !extra_output.contains(&ContractOutputSelection::StorageLayout) {
            extra_output.push(ContractOutputSelection::StorageLayout);
        }
        let modified_build_args =
            BuildOpts { compiler: CompilerOpts { extra_output, ..build.compiler }, ..build };

        let mut project = modified_build_args.project()?;
        if !user_extra_output && !project.build_info {
            project.no_artifacts = true;
            project.update_output_selection(|selection| {
                *selection = OutputSelection::common_output_selection([
                    ContractOutputSelection::StorageLayout.to_string(),
                ]);
            });
        }

        let reference = reference.unwrap_or_else(|| project.root().join(".clone.meta"));
        let previous = read_storage_layout(&reference)?;
        let contract = contract.or(previous.contract).ok_or_else(|| {
            eyre::eyre!(
                "contract must be provided unless the reference is a `.clone.meta` file with \
                 `path` and `targetContract`"
            )
        })?;

        let target_path = find_target_path(&project, &contract)?;
        let contract_name =
            contract.name().map(str::to_owned).unwrap_or_else(|| target_path.display().to_string());

        let compiler = ProjectCompiler::new().quiet(true);
        let mut output = compiler.files([target_path.clone()]).compile(&project)?;
        let artifact = find_matching_contract_artifact(&mut output, &target_path, contract.name())?;
        let current = artifact
            .storage_layout
            .as_ref()
            .ok_or_else(|| eyre::eyre!("Could not get storage layout"))?;

        let report = compare_storage_layouts(
            &previous.layout,
            current,
            contract_name,
            reference.display().to_string(),
            allow_renames,
        );
        print_report(&report)?;
        eyre::ensure!(report.compatible, "compiler storage layout is incompatible");
        Ok(())
    }
}

/// A machine-readable semantic storage layout compatibility report.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompatibilityReport {
    compatible: bool,
    contract: String,
    reference: String,
    scope: &'static str,
    changes: Vec<LayoutChange>,
    limitations: [&'static str; 1],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum ChangeKind {
    LabelChanged,
    SlotChanged,
    OffsetChanged,
    TypeChanged,
    InheritanceChanged,
    Removed,
    Appended,
    AddedOrReordered,
}

impl ChangeKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::LabelChanged => "label changed",
            Self::SlotChanged => "slot changed",
            Self::OffsetChanged => "offset changed",
            Self::TypeChanged => "type changed",
            Self::InheritanceChanged => "inheritance changed",
            Self::Removed => "removed",
            Self::Appended => "appended",
            Self::AddedOrReordered => "added or reordered",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Severity {
    Error,
    Info,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LayoutChange {
    severity: Severity,
    kind: ChangeKind,
    previous: Option<LayoutItem>,
    current: Option<LayoutItem>,
    message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LayoutItem {
    label: String,
    contract: String,
    slot: String,
    offset: i64,
    #[serde(rename = "type")]
    type_name: String,
}

impl LayoutItem {
    fn new(layout: &StorageLayout, storage: &Storage) -> Self {
        Self {
            label: storage.label.clone(),
            contract: storage.contract.clone(),
            slot: storage.slot.clone(),
            offset: storage.offset,
            type_name: storage_type_label(layout, storage).to_owned(),
        }
    }
}

fn compare_storage_layouts(
    previous: &StorageLayout,
    current: &StorageLayout,
    contract: String,
    reference: String,
    allow_renames: bool,
) -> CompatibilityReport {
    let mut changes = Vec::new();
    let mut prefix_compatible = true;

    for (index, previous_storage) in previous.storage.iter().enumerate() {
        let Some(current_storage) = current.storage.get(index) else {
            prefix_compatible = false;
            changes.push(LayoutChange {
                severity: Severity::Error,
                kind: ChangeKind::Removed,
                previous: Some(LayoutItem::new(previous, previous_storage)),
                current: None,
                message: format!(
                    "State variable `{}` was removed from slot {} offset {}.",
                    previous_storage.label, previous_storage.slot, previous_storage.offset
                ),
            });
            continue;
        };

        let previous_item = || Some(LayoutItem::new(previous, previous_storage));
        let current_item = || Some(LayoutItem::new(current, current_storage));

        if previous_storage.label != current_storage.label {
            if !allow_renames {
                prefix_compatible = false;
            }
            changes.push(LayoutChange {
                severity: if allow_renames { Severity::Info } else { Severity::Error },
                kind: ChangeKind::LabelChanged,
                previous: previous_item(),
                current: current_item(),
                message: if allow_renames {
                    format!(
                        "State variable at slot {} offset {} was renamed from `{}` to `{}`.",
                        previous_storage.slot,
                        previous_storage.offset,
                        previous_storage.label,
                        current_storage.label
                    )
                } else {
                    format!(
                        "State variable at slot {} offset {} changed label from `{}` to `{}`; use \
                     `--allow-renames` only for an intentional rename.",
                        previous_storage.slot,
                        previous_storage.offset,
                        previous_storage.label,
                        current_storage.label
                    )
                },
            });
        }

        if previous_storage.slot != current_storage.slot {
            prefix_compatible = false;
            changes.push(LayoutChange {
                severity: Severity::Error,
                kind: ChangeKind::SlotChanged,
                previous: previous_item(),
                current: current_item(),
                message: format!(
                    "State variable `{}` moved from slot {} to slot {}.",
                    previous_storage.label, previous_storage.slot, current_storage.slot
                ),
            });
        }

        if previous_storage.offset != current_storage.offset {
            prefix_compatible = false;
            changes.push(LayoutChange {
                severity: Severity::Error,
                kind: ChangeKind::OffsetChanged,
                previous: previous_item(),
                current: current_item(),
                message: format!(
                    "State variable `{}` moved from offset {} to offset {} in slot {}.",
                    previous_storage.label,
                    previous_storage.offset,
                    current_storage.offset,
                    current_storage.slot
                ),
            });
        }

        if !storage_types_compatible(
            previous,
            &previous_storage.storage_type,
            current,
            &current_storage.storage_type,
            allow_renames,
        ) {
            prefix_compatible = false;
            changes.push(LayoutChange {
                severity: Severity::Error,
                kind: ChangeKind::TypeChanged,
                previous: previous_item(),
                current: current_item(),
                message: format!(
                    "State variable `{}` changed type from `{}` to `{}`.",
                    previous_storage.label,
                    storage_type_label(previous, previous_storage),
                    storage_type_label(current, current_storage)
                ),
            });
        }

        let previous_contract = declaring_contract(&previous_storage.contract);
        let current_contract = declaring_contract(&current_storage.contract);
        if previous_contract != current_contract {
            prefix_compatible = false;
            changes.push(LayoutChange {
                severity: Severity::Error,
                kind: ChangeKind::InheritanceChanged,
                previous: previous_item(),
                current: current_item(),
                message: format!(
                    "State variable `{}` changed declaring contract from `{previous_contract}` to \
                     `{current_contract}`.",
                    previous_storage.label
                ),
            });
        }
    }

    for current_storage in current.storage.iter().skip(previous.storage.len()) {
        let is_append = prefix_compatible
            && storage_range(current, current_storage).is_some_and(|current_range| {
                previous.storage.iter().all(|previous_storage| {
                    storage_range(previous, previous_storage)
                        .is_some_and(|previous_range| !current_range.overlaps(previous_range))
                })
            });
        let kind = if is_append { ChangeKind::Appended } else { ChangeKind::AddedOrReordered };
        let severity = if is_append { Severity::Info } else { Severity::Error };
        changes.push(LayoutChange {
            severity,
            kind,
            previous: None,
            current: Some(LayoutItem::new(current, current_storage)),
            message: if is_append {
                format!(
                    "State variable `{}` was appended at slot {} offset {}.",
                    current_storage.label, current_storage.slot, current_storage.offset
                )
            } else {
                format!(
                    "State variable `{}` was added at slot {} offset {}, but the previous layout \
                     is not an unchanged prefix.",
                    current_storage.label, current_storage.slot, current_storage.offset
                )
            },
        });
    }

    CompatibilityReport {
        compatible: !changes.iter().any(|change| change.severity == Severity::Error),
        contract,
        reference,
        scope: LAYOUT_SCOPE,
        changes,
        limitations: [LAYOUT_LIMITATION],
    }
}

fn storage_types_compatible(
    previous: &StorageLayout,
    previous_id: &str,
    current: &StorageLayout,
    current_id: &str,
    allow_renames: bool,
) -> bool {
    let mut visited = HashSet::new();
    storage_types_compatible_inner(
        previous,
        previous_id,
        current,
        current_id,
        allow_renames,
        &mut visited,
    )
}

fn storage_types_compatible_inner(
    previous: &StorageLayout,
    previous_id: &str,
    current: &StorageLayout,
    current_id: &str,
    allow_renames: bool,
    visited: &mut HashSet<(String, String)>,
) -> bool {
    if !visited.insert((previous_id.to_owned(), current_id.to_owned())) {
        return true;
    }
    let (Some(previous_type), Some(current_type)) =
        (previous.types.get(previous_id), current.types.get(current_id))
    else {
        return false;
    };
    if previous_type.encoding != current_type.encoding
        || previous_type.number_of_bytes != current_type.number_of_bytes
    {
        return false;
    }

    let previous_members = storage_members(previous_type);
    let current_members = storage_members(current_type);
    let has_members = previous_members.is_some();
    match (previous_members, current_members) {
        (Some(previous_members), Some(current_members)) => {
            let (Ok(previous_members), Ok(current_members)) = (previous_members, current_members)
            else {
                return false;
            };
            if previous_members.len() != current_members.len() {
                return false;
            }
            for (previous_member, current_member) in previous_members.iter().zip(&current_members) {
                if previous_member.slot != current_member.slot
                    || previous_member.offset != current_member.offset
                    || (!allow_renames && previous_member.label != current_member.label)
                    || declaring_contract(&previous_member.contract)
                        != declaring_contract(&current_member.contract)
                    || !storage_types_compatible_inner(
                        previous,
                        &previous_member.storage_type,
                        current,
                        &current_member.storage_type,
                        allow_renames,
                        visited,
                    )
                {
                    return false;
                }
            }
        }
        (None, None) => {}
        _ => return false,
    }

    for field in ["base", "key", "value"] {
        let previous_id = type_reference(previous_type, field);
        let current_id = type_reference(current_type, field);
        match (previous_id, current_id) {
            (Some(previous_id), Some(current_id)) => {
                if !storage_types_compatible_inner(
                    previous,
                    previous_id,
                    current,
                    current_id,
                    allow_renames,
                    visited,
                ) {
                    return false;
                }
            }
            (None, None) => {}
            _ => return false,
        }
    }

    let has_structure = has_members
        || ["base", "key", "value"]
            .iter()
            .any(|field| type_reference(previous_type, field).is_some());
    if !has_structure && previous_type.label != current_type.label {
        return false;
    }

    let mut previous_other = previous_type.other.clone();
    let mut current_other = current_type.other.clone();
    for field in ["members", "base", "key", "value"] {
        previous_other.remove(field);
        current_other.remove(field);
    }
    previous_other == current_other
}

fn storage_members(storage_type: &StorageType) -> Option<serde_json::Result<Vec<Storage>>> {
    storage_type.other.get("members").cloned().map(serde_json::from_value::<Vec<Storage>>)
}

fn type_reference<'a>(storage_type: &'a StorageType, field: &str) -> Option<&'a str> {
    match field {
        "key" => storage_type.key.as_deref(),
        "value" => storage_type.value.as_deref(),
        _ => storage_type.other.get(field)?.as_str(),
    }
}

fn storage_type_label<'a>(layout: &'a StorageLayout, storage: &'a Storage) -> &'a str {
    layout
        .types
        .get(&storage.storage_type)
        .map_or(&storage.storage_type, |storage_type| &storage_type.label)
}

fn declaring_contract(contract: &str) -> &str {
    contract.rsplit_once(':').map_or(contract, |(_, name)| name)
}

#[derive(Clone, Copy, Debug)]
struct StorageRange {
    start: StorageCoordinate,
    end: StorageCoordinate,
}

impl StorageRange {
    fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct StorageCoordinate {
    slot: U256,
    offset: u8,
}

fn storage_range(layout: &StorageLayout, storage: &Storage) -> Option<StorageRange> {
    let storage_type = layout.types.get(&storage.storage_type)?;
    let slot = storage.slot.parse::<U256>().ok()?;
    let offset = u8::try_from(storage.offset).ok()?;
    let bytes = storage_type.number_of_bytes.parse::<U256>().ok()?;
    let total = U256::from(offset) + bytes;
    let slot_delta = total / U256::from(32);
    let end_slot = slot.checked_add(slot_delta)?;
    let end_offset = u8::try_from(total % U256::from(32)).ok()?;
    Some(StorageRange {
        start: StorageCoordinate { slot, offset },
        end: StorageCoordinate { slot: end_slot, offset: end_offset },
    })
}

struct StorageLayoutReference {
    layout: StorageLayout,
    contract: Option<PathOrContractInfo>,
}

fn read_storage_layout(path: &Path) -> Result<StorageLayoutReference> {
    let contents = fs::read_to_string(path)
        .wrap_err_with(|| format!("failed to read storage layout reference {}", path.display()))?;
    let value: Value = serde_json::from_str(&contents)
        .wrap_err_with(|| format!("failed to parse storage layout reference {}", path.display()))?;
    let contract = value
        .get("path")
        .and_then(Value::as_str)
        .zip(value.get("targetContract").and_then(Value::as_str))
        .map(|(path, contract)| PathOrContractInfo::from_str(&format!("{path}:{contract}")))
        .transpose()
        .wrap_err("invalid contract identifier in clone metadata")?;
    let layout = value
        .get("storageLayout")
        .or_else(|| value.get("storage_layout"))
        .unwrap_or(&value)
        .clone();
    let layout = serde_json::from_value(layout).wrap_err_with(|| {
        format!("{} does not contain a Solidity compiler storage layout", path.display())
    })?;
    Ok(StorageLayoutReference { layout, contract })
}

fn print_report(report: &CompatibilityReport) -> Result<()> {
    if shell::is_json() {
        sh_println!("{}", serde_json::to_string_pretty(report)?)?;
        return Ok(());
    }

    if report.compatible {
        sh_println!("Compiler-reported storage entries are compatible.")?;
    } else {
        sh_println!("Compiler-reported storage entries are incompatible.")?;
    }
    sh_println!("Contract: {}", report.contract)?;
    sh_println!("Reference: {}", report.reference)?;
    if report.changes.is_empty() {
        sh_println!("Changes: none")?;
    } else {
        sh_println!("Changes:")?;
        for change in &report.changes {
            let marker = if change.severity == Severity::Error { "error" } else { "info" };
            sh_println!("  - [{marker}] {}: {}", change.kind.as_str(), change.message)?;
        }
    }
    sh_println!("Scope: {}", report.limitations[0])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn layout(value: Value) -> StorageLayout {
        serde_json::from_value(value).unwrap()
    }

    fn simple_layout(contract: &str, label: &str, slot: &str, ty: &str) -> StorageLayout {
        let mut layout = layout(json!({
            "storage": [{
                "astId": 1,
                "contract": contract,
                "label": label,
                "offset": 0,
                "slot": slot,
                "type": ty
            }],
            "types": {}
        }));
        layout.types.insert(
            ty.to_string(),
            serde_json::from_value(json!({
                "encoding": "inplace",
                "label": "uint256",
                "numberOfBytes": "32"
            }))
            .unwrap(),
        );
        layout
    }

    fn compare(previous: &StorageLayout, current: &StorageLayout) -> CompatibilityReport {
        compare_storage_layouts(
            previous,
            current,
            "Current".to_string(),
            "previous.json".to_string(),
            false,
        )
    }

    #[test]
    fn ignores_unstable_type_and_ast_ids() {
        let previous = layout(json!({
            "storage": [{
                "astId": 1,
                "contract": "src/Old.sol:Vault",
                "label": "account",
                "offset": 0,
                "slot": "0",
                "type": "t_struct(Account)12_storage"
            }],
            "types": {
                "t_struct(Account)12_storage": {
                    "encoding": "inplace",
                    "label": "struct Old.Account",
                    "members": [{
                        "astId": 2,
                        "contract": "src/Old.sol:Vault",
                        "label": "balance",
                        "offset": 0,
                        "slot": "0",
                        "type": "t_uint256"
                    }],
                    "numberOfBytes": "32"
                },
                "t_uint256": {
                    "encoding": "inplace",
                    "label": "uint256",
                    "numberOfBytes": "32"
                }
            }
        }));
        let current = layout(json!({
            "storage": [{
                "astId": 99,
                "contract": "contracts/New.sol:Vault",
                "label": "account",
                "offset": 0,
                "slot": "0",
                "type": "t_struct(Account)777_storage"
            }],
            "types": {
                "t_struct(Account)777_storage": {
                    "encoding": "inplace",
                    "label": "struct New.Account",
                    "members": [{
                        "astId": 100,
                        "contract": "contracts/New.sol:Vault",
                        "label": "balance",
                        "offset": 0,
                        "slot": "0",
                        "type": "t_uint256"
                    }],
                    "numberOfBytes": "32"
                },
                "t_uint256": {
                    "encoding": "inplace",
                    "label": "uint256",
                    "numberOfBytes": "32"
                }
            }
        }));

        assert!(compare(&previous, &current).compatible);
    }

    #[test]
    fn accepts_non_overlapping_append() {
        let previous = layout(json!({
            "storage": [{
                "astId": 1,
                "contract": "src/Vault.sol:Vault",
                "label": "enabled",
                "offset": 0,
                "slot": "0",
                "type": "t_bool"
            }],
            "types": {
                "t_bool": {
                    "encoding": "inplace",
                    "label": "bool",
                    "numberOfBytes": "1"
                },
                "t_uint248": {
                    "encoding": "inplace",
                    "label": "uint248",
                    "numberOfBytes": "31"
                }
            }
        }));
        let mut current = previous.clone();
        current.storage.push(Storage {
            ast_id: 2,
            contract: "src/Vault.sol:Vault".to_string(),
            label: "balance".to_string(),
            offset: 1,
            slot: "0".to_string(),
            storage_type: "t_uint248".to_string(),
        });

        let report = compare(&previous, &current);
        assert!(report.compatible);
        assert_eq!(report.changes.len(), 1);
        assert_eq!(report.changes[0].kind, ChangeKind::Appended);
    }

    #[test]
    fn compares_mapping_keys_and_values_recursively() {
        let previous = layout(json!({
            "storage": [{
                "astId": 1,
                "contract": "src/Vault.sol:Vault",
                "label": "balances",
                "offset": 0,
                "slot": "0",
                "type": "old_mapping"
            }],
            "types": {
                "old_mapping": {
                    "encoding": "mapping",
                    "key": "old_address",
                    "label": "mapping(address => uint256)",
                    "numberOfBytes": "32",
                    "value": "old_uint"
                },
                "old_address": {
                    "encoding": "inplace",
                    "label": "address",
                    "numberOfBytes": "20"
                },
                "old_uint": {
                    "encoding": "inplace",
                    "label": "uint256",
                    "numberOfBytes": "32"
                }
            }
        }));
        let mut current = layout(json!({
            "storage": [{
                "astId": 99,
                "contract": "src/Vault.sol:Vault",
                "label": "balances",
                "offset": 0,
                "slot": "0",
                "type": "new_mapping"
            }],
            "types": {
                "new_mapping": {
                    "encoding": "mapping",
                    "key": "new_address",
                    "label": "mapping(address => uint256)",
                    "numberOfBytes": "32",
                    "value": "new_uint"
                },
                "new_address": {
                    "encoding": "inplace",
                    "label": "address",
                    "numberOfBytes": "20"
                },
                "new_uint": {
                    "encoding": "inplace",
                    "label": "uint256",
                    "numberOfBytes": "32"
                }
            }
        }));

        assert!(compare(&previous, &current).compatible);
        current.types.get_mut("new_uint").unwrap().label = "bytes32".to_string();
        assert_change(&compare(&previous, &current), ChangeKind::TypeChanged);
    }

    #[test]
    fn rejects_location_type_inheritance_and_label_changes() {
        let previous = simple_layout("src/Base.sol:Base", "value", "0", "t_uint256");

        let mut slot = previous.clone();
        slot.storage[0].slot = "1".to_string();
        assert_change(&compare(&previous, &slot), ChangeKind::SlotChanged);

        let mut offset = previous.clone();
        offset.storage[0].offset = 1;
        assert_change(&compare(&previous, &offset), ChangeKind::OffsetChanged);

        let mut ty = previous.clone();
        ty.types.get_mut("t_uint256").unwrap().label = "bytes32".to_string();
        assert_change(&compare(&previous, &ty), ChangeKind::TypeChanged);

        let mut inheritance = previous.clone();
        inheritance.storage[0].contract = "src/Other.sol:Other".to_string();
        assert_change(&compare(&previous, &inheritance), ChangeKind::InheritanceChanged);

        let mut label = previous.clone();
        label.storage[0].label = "other".to_string();
        assert_change(&compare(&previous, &label), ChangeKind::LabelChanged);
    }

    #[test]
    fn allow_renames_is_explicit() {
        let previous = simple_layout("src/Vault.sol:Vault", "value", "0", "t_uint256");
        let mut current = previous.clone();
        current.storage[0].label = "renamed".to_string();
        let report = compare_storage_layouts(
            &previous,
            &current,
            "Vault".to_string(),
            "previous.json".to_string(),
            true,
        );
        assert!(report.compatible);
        assert_eq!(report.changes[0].kind, ChangeKind::LabelChanged);
        assert_eq!(report.changes[0].severity, Severity::Info);
    }

    #[test]
    fn rejects_removed_and_inserted_entries() {
        let previous = simple_layout("src/Vault.sol:Vault", "value", "1", "t_uint256");
        let removed = StorageLayout::default();
        assert_change(&compare(&previous, &removed), ChangeKind::Removed);

        let mut inserted = previous.clone();
        inserted.storage.insert(
            0,
            Storage {
                ast_id: 2,
                contract: "src/Vault.sol:Vault".to_string(),
                label: "inserted".to_string(),
                offset: 0,
                slot: "0".to_string(),
                storage_type: "t_uint256".to_string(),
            },
        );
        assert_change(&compare(&previous, &inserted), ChangeKind::AddedOrReordered);
    }

    fn assert_change(report: &CompatibilityReport, kind: ChangeKind) {
        assert!(!report.compatible);
        assert!(report.changes.iter().any(|change| change.kind == kind));
    }
}
