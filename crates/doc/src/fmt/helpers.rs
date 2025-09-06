use ariadne::{Color, Fmt, Label, Report, ReportKind, Source};
use solang_parser::{diagnostics::Diagnostic, pt::*};
use std::path::Path;

/// Formats parser diagnostics
pub fn format_diagnostics_report(
    content: &str,
    path: Option<&Path>,
    diagnostics: &[Diagnostic],
) -> String {
    if diagnostics.is_empty() {
        return String::new();
    }

    let filename =
        path.map(|p| p.file_name().unwrap().to_string_lossy().to_string()).unwrap_or_default();
    let mut s = Vec::new();
    for diag in diagnostics {
        let span = (filename.as_str(), diag.loc.start()..diag.loc.end());
        let mut report = Report::build(ReportKind::Error, span.clone())
            .with_message(format!("{:?}", diag.ty))
            .with_label(
                Label::new(span)
                    .with_color(Color::Red)
                    .with_message(diag.message.as_str().fg(Color::Red)),
            );

        for note in &diag.notes {
            report = report.with_note(&note.message);
        }

        report.finish().write((filename.as_str(), Source::from(content)), &mut s).unwrap();
    }
    String::from_utf8(s).unwrap()
}

pub fn import_path_string(path: &ImportPath) -> String {
    match path {
        ImportPath::Filename(s) => s.string.clone(),
        ImportPath::Path(p) => p.to_string(),
    }
}
