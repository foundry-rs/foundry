use super::EventFields;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::ast::{ElementaryType, Item, ItemKind, TypeKind};

declare_forge_lint!(
    EVENT_FIELDS,
    Severity::Info,
    "event-fields",
    "`address` event parameter is not `indexed`",
    help = "consider indexing address fields that consumers need to filter by"
);

impl<'ast> EarlyLintPass<'ast> for EventFields {
    fn check_item(&mut self, ctx: &LintContext, item: &'ast Item<'ast>) {
        let ItemKind::Event(event) = &item.kind else { return };
        if event.parameters.iter().any(|p| p.indexed) {
            return;
        }
        // The EVM allows 3 indexed parameters in a non-anonymous event and 4 in an anonymous one.
        let slots_available = if event.anonymous { 4 } else { 3 };
        // The offending `address` parameters, rendered as `name (type)` in declaration order.
        let names = event
            .parameters
            .iter()
            .enumerate()
            .filter_map(|(idx, param)| {
                let TypeKind::Elementary(ElementaryType::Address(payable)) = &param.ty.kind else {
                    return None;
                };
                let name = param
                    .name
                    .map_or_else(|| format!("parameter #{}", idx + 1), |n| format!("`{n}`"));
                let ty = if *payable { "address payable" } else { "address" };
                Some(format!("{name} (`{ty}`)"))
            })
            .take(slots_available)
            .collect::<Vec<String>>();
        if !names.is_empty() {
            let msg = format!("event has unindexed address fields: {}", names.join(", "));
            ctx.emit_with_msg(&EVENT_FIELDS, event.name.span, msg);
        }
    }
}
