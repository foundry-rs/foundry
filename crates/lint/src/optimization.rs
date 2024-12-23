use solar_ast::{
    ast::{Item, ItemFunction, ItemKind},
    visit::Visit,
};

use crate::Keccak256;

impl<'ast> Visit<'ast> for Keccak256 {
    fn visit_item(&mut self, item: &'ast Item<'ast>) {
        if let ItemKind::Function(ItemFunction { kind, header, body }) = &item.kind {
            if let Some(name) = header.name {
                // Use assembly to hash <https://github.com/0xKitsune/EVM-Gas-Optimizations?tab=readme-ov-file#gas-report-10>
                if name.as_str() == "keccak256" {
                    self.items.push(item.span);
                }
            }
        }

        self.walk_item(item);
    }
}
