use super::EventFields;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::ast::{ElementaryType, Item, ItemEvent, ItemKind, Type, TypeKind, VariableDefinition};

declare_forge_lint!(
    EVENT_FIELDS,
    Severity::Info,
    "event-fields",
    "address and id event parameters should be indexed for efficient log filtering"
);

/// Maximum number of indexed parameters allowed by the EVM in a non-anonymous event.
const MAX_INDEXED_NON_ANON: usize = 3;
/// Maximum number of indexed parameters allowed by the EVM in an anonymous event.
const MAX_INDEXED_ANON: usize = 4;

impl<'ast> EarlyLintPass<'ast> for EventFields {
    fn check_item(&mut self, ctx: &LintContext, item: &'ast Item<'ast>) {
        let ItemKind::Event(event) = &item.kind else { return };
        check_event(ctx, event);
    }
}

fn check_event<'ast>(ctx: &LintContext, event: &'ast ItemEvent<'ast>) {
    if event.parameters.iter().any(|p| p.indexed) {
        return;
    }
    let slots_available = if event.anonymous { MAX_INDEXED_ANON } else { MAX_INDEXED_NON_ANON };

    // Collect offending unindexed params (with their positional index) in declaration order.
    let mut offenders: Vec<(usize, &VariableDefinition<'ast>)> = Vec::new();
    for (idx, param) in event.parameters.iter().enumerate() {
        if is_filterable_field(param) {
            offenders.push((idx, param));
            if offenders.len() == slots_available {
                break;
            }
        }
    }

    if offenders.is_empty() {
        return;
    }

    // Build a single message naming the offending fields.
    let names = offenders.iter().map(|(i, p)| describe_param(*i, p)).collect::<Vec<_>>().join(", ");
    let msg = format!("event has unindexed fields that may benefit from being indexed: {names}");
    ctx.emit_with_msg(&EVENT_FIELDS, event.name.span, msg);
}

/// Returns true when the parameter is an `address`, an `address payable`, or a uint256/bytes32
/// whose name looks like an identifier (id-like).
fn is_filterable_field(param: &VariableDefinition<'_>) -> bool {
    let TypeKind::Elementary(elem) = &param.ty.kind else { return false };
    match elem {
        ElementaryType::Address(_) => true,
        ElementaryType::UInt(size) if size.bits() == 256 => has_id_like_name(param),
        ElementaryType::FixedBytes(size) if size.bytes() == 32 => has_id_like_name(param),
        _ => false,
    }
}

/// Returns true when the parameter name matches `id`/`ID`, ends with `Id`, `_id`, `_ID`, or ends
/// with `ID` preceded by a lowercase ASCII letter.
fn has_id_like_name(param: &VariableDefinition<'_>) -> bool {
    let Some(ident) = &param.name else { return false };
    let name = ident.as_str();

    if name == "id" || name == "ID" {
        return true;
    }
    if name.ends_with("_id") || name.ends_with("_ID") || name.ends_with("Id") {
        return true;
    }
    if let Some(prefix) = name.strip_suffix("ID")
        && let Some(last) = prefix.chars().last()
    {
        return last.is_ascii_lowercase();
    }
    false
}

/// Render a parameter as `name (type)` (or `parameter #N (type)` if unnamed) for the diagnostic.
fn describe_param(index: usize, param: &VariableDefinition<'_>) -> String {
    let name = match &param.name {
        Some(ident) => ident.as_str().to_string(),
        None => format!("parameter #{}", index + 1),
    };
    let ty = type_str(&param.ty);
    format!("{name} ({ty})")
}

const fn type_str(ty: &Type<'_>) -> &'static str {
    match &ty.kind {
        TypeKind::Elementary(ElementaryType::Address(true)) => "address payable",
        TypeKind::Elementary(ElementaryType::Address(false)) => "address",
        TypeKind::Elementary(ElementaryType::UInt(_)) => "uint256",
        TypeKind::Elementary(ElementaryType::FixedBytes(_)) => "bytes32",
        _ => "?",
    }
}
