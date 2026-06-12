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
    "address event parameters should be indexed for efficient log filtering"
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

/// Returns true when the parameter is an `address`.
const fn is_filterable_field(param: &VariableDefinition<'_>) -> bool {
    matches!(&param.ty.kind, TypeKind::Elementary(ElementaryType::Address(_)))
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
