use super::CoverageReporter;
use foundry_evm::coverage::CoverageReport;

/// A super verbose reporter for debugging coverage while it is still unstable.
pub struct DebugReporter;

impl CoverageReporter for DebugReporter {
    fn report(self, report: &CoverageReport) -> eyre::Result<()> {
        for (path, items) in report.items_by_source() {
            println!("Uncovered for {path}:");
            items.iter().for_each(|item| {
                if item.hits == 0 {
                    println!("- {item}");
                }
            });
            println!();
        }

        for (contract_id, anchors) in &report.anchors {
            println!("Anchors for {contract_id}:");
            anchors.iter().for_each(|anchor| {
                println!("- {anchor}");
                println!(
                    "  - Refers to item: {}",
                    report
                        .items
                        .get(&contract_id.version)
                        .and_then(|items| items.get(anchor.item_id))
                        .map_or("None".to_owned(), |item| item.to_string())
                );
            });
            println!();
        }

        Ok(())
    }
}
