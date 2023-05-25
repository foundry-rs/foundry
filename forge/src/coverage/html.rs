use super::CoverageReporter;
use foundry_common::fs::create_file;
use foundry_evm::coverage::{CoverageReport, CoverageSummary};
use itertools::Itertools;
use maud::html;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
use time::{format_description, OffsetDateTime};

pub struct HtmlReporter {
    root: PathBuf,
}

impl HtmlReporter {
    pub fn new(root: PathBuf) -> HtmlReporter {
        Self { root }
    }
}

const COVERAGE_THRESHOLD: f32 = 0.9;
const START_HTML: &str = "<html>
   <head>
      <title>Coverage Report</title>
      <link rel='stylesheet' href='https://cdn.jsdelivr.net/npm/@picocss/pico@1/css/pico.min.css'>
      <link rel='preconnect' href='https://rsms.me/'>
      <link rel='stylesheet' href='https://rsms.me/inter/inter.css'>
      <style> * { font-family: 'Inter', sans-serif !important; } :root { font-family: 'Inter', sans-serif; } @supports (font-variation-settings: normal) { :root {{ font-family: 'Inter var', sans-serif; } }}
        .highlight-red { background:red; }
        .highlight-green { background:green; }
      </style>
   </head>
   <body>";
const START_TABLE_HTML: &str = "<article>
<table>
<tr>
   <td width='30%'><br></td>
   <td width='17.5%'></td>
   <td width='17.5%'></td>
   <td width='17.5%'></td>
   <td width='17.5%'></td>
</tr>
<tr>
   <th style='text-align: center'>Directory</th>
   <th style='text-align: center' colspan='2'>Lines</th>
   <th style='text-align: center' colspan='2'>Functions</th>
</tr>";

impl CoverageReporter for HtmlReporter {
    fn report(self, report: &CoverageReport) -> eyre::Result<()> {
        let mut top_html = String::from(START_HTML);

        let top_lvl_summary = report.items_by_source().flat_map(|x| x.1).fold(
            CoverageSummary::default(),
            |mut all_summary, item| {
                all_summary += &item;
                all_summary
            },
        );
        let top_level_path = self.root.join("index.html");
        let top_level_path = &top_level_path.to_string_lossy();
        let main_summary = gen_top_summary(&top_lvl_summary, "/", top_level_path);
        top_html.push_str(&main_summary);
        top_html.push_str(START_TABLE_HTML);

        for (dir_name, dir_files) in report
            .items_by_source()
            .group_by(|e| Path::new(&e.0).parent().unwrap().to_string_lossy().into_owned())
            .into_iter()
        {
            let mut src_preview_html = String::from(START_HTML);
            let dir_writer = &mut create_file(self.root.join(format!("{dir_name}/index.html")))?;
            writeln!(dir_writer, "{START_HTML}")?;

            let dir_files = dir_files.collect_vec();
            let top_lvl_summary = dir_files.iter().flat_map(|z| z.clone().1).fold(
                CoverageSummary::default(),
                |mut all_summary, item| {
                    all_summary += &item;
                    all_summary
                },
            );
            let top_lvl_summary_html = gen_top_summary(&top_lvl_summary, &dir_name, top_level_path);
            writeln!(dir_writer, "{top_lvl_summary_html}{START_TABLE_HTML}")?;
            src_preview_html.push_str(&top_lvl_summary_html);

            let file_summary = dir_files.iter().flat_map(|z| z.clone().1).fold(
                CoverageSummary::default(),
                |mut summary, item| {
                    summary += &item;
                    summary
                },
            );

            let row = gen_row(dir_name, &file_summary, true);
            top_html.push_str(row.as_str());

            for (file_name, covs) in dir_files {
                let mut open_source = std::fs::File::open(&file_name)?;
                let mut src_content = String::new();
                open_source.read_to_string(&mut src_content).unwrap();
                let filename = Path::new(&file_name).file_name().unwrap().to_string_lossy();
                let mut src_code = gen_source_block(src_content, &filename);
                let mut lines: Vec<String> = src_code.lines().map(String::from).collect();
                for (lnum, hits) in covs.iter().map(|c| (c.loc.line, c.hits)) {
                    if lnum > 0 && lnum <= lines.len() {
                        let class = if hits > 0 { "highlight-green" } else { "highlight-red" };
                        lines[lnum - 1] =
                            format!("<mark class=\"{}\">{}</mark>", class, &lines[lnum - 1]);
                    }
                }
                src_code = lines.join("\n");

                let mut src_preview_html = src_preview_html.clone();
                src_preview_html.push_str(&src_code);
                let summary = covs.iter().fold(CoverageSummary::default(), |mut summary, item| {
                    summary += item;
                    summary
                });
                let row = gen_row(filename.to_string(), &summary, false);
                writeln!(dir_writer, "{row}")?;
                src_preview_html.push_str("</table></article></body></html>");
                let source_file_writer =
                    &mut create_file(self.root.join(format!("{file_name}.html")))?;
                writeln!(source_file_writer, "{src_preview_html}")?;
            }
            writeln!(dir_writer, "</table></article></body></html>")?;
        }

        top_html.push_str("</table></article></body></html>");
        let top_writer = &mut create_file("index.html")?;
        writeln!(top_writer, "{top_html}")?;

        println!("Wrote HTML report.");

        Ok(())
    }
}

fn percentify(a: usize, b: usize) -> String {
    let num = if b == 0 { 0.0 } else { (a as f32 / b as f32 * 10000.0).round() / 100.0 };
    format!("{num:.2}")
}

fn gen_row(name: String, summary: &CoverageSummary, is_dir: bool) -> String {
    let (lc, lh) = (summary.line_count, summary.line_hits);
    let (fc, fh) = (summary.function_count, summary.function_hits);
    let (pl, pf) = (percentify(lh, lc), percentify(fh, fc));
    let lcolor = if lh as f32 / lc as f32 > 0.9 { "color: green" } else { "color: red" };
    let fcolor = if fh as f32 / fc as f32 > 0.9 { "color: green" } else { "color: red" };
    let to = if !is_dir { format!("{name}.html",) } else { format!("{name}/index.html",) };
    html! {
        tr {
            td { a href=(to) { strong { (name) } } }
            td style=(lcolor) { (lh)"/"(lc) }
            td style=(lcolor) { (pl)"%" }
            td style=(fcolor) { (fh)"/"(fc) }
            td style=(fcolor) { (pf)"%" }
        }
    }
    .into_string()
}

fn gen_source_block(code: String, file_name: &str) -> String {
    html! {
        article {
            header {
                h3 {
                    (file_name)
                }
            }
            pre {
                code {
                    (code)
                }
            }
        }
    }
    .into_string()
}

fn gen_top_summary(top_lvl_summary: &CoverageSummary, crumb: &str, root: &str) -> String {
    let (lc, lh) = (top_lvl_summary.line_count, top_lvl_summary.line_hits);
    let (fc, fh) = (top_lvl_summary.function_count, top_lvl_summary.function_hits);
    let (bc, bh) = (top_lvl_summary.branch_count, top_lvl_summary.branch_hits);
    let (pl, pf, pb) = (percentify(lh, lc), percentify(fh, fc), percentify(bh, bc));
    let now = OffsetDateTime::now_utc();
    let format =
        format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]").unwrap();
    let now = now.format(&format).unwrap();
    let crumb = if crumb != "/" { format!("/{crumb}") } else { String::from("/") };
    let lcolor =
        if lh as f32 / lc as f32 > COVERAGE_THRESHOLD { "color: green" } else { "color: red" };
    let fcolor =
        if fh as f32 / fc as f32 > COVERAGE_THRESHOLD { "color: green" } else { "color: red" };
    let bcolor =
        if bh as f32 / bc as f32 > COVERAGE_THRESHOLD { "color: green" } else { "color: red" };

    html! {
        div style="background-color: #130909" data-theme="dark" {
            header."container" {
                hgroup {
                    h1 { "Foundry Coverage Report" }
                    h4 { (now) }
                    h4 {
                        a href=(root) { "top" }
                        (crumb)
                    }
                }
                    article {
                        table {
                            tr { th { "Type "} th { "Hit" } th { "Total" } th { "Coverage" } }
                            tr { td { "Lines" } td style=(lcolor) { (lh) } td style=(lcolor) { (lc) } td style=(lcolor) { (pl)"%" } }
                            tr { td { "Functions" } td style=(fcolor) { (fh) } td style=(fcolor) { (fc) } td style=(fcolor) { (pf)"%" } }
                            tr { td { "Branches" } td style=(bcolor) { (bh) } td style=(bcolor) { (bc) } td style=(bcolor) { (pb)"%" }
                            }
                        }
                    }
            }
        }
    }
    .into_string()
}
