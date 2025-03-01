use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

use clap::builder::TypedValueParser;
use clap::{ArgAction, Parser};
use env_logger::Env;
use inferno::flamegraph::color::{
    parse_hex_color, BackgroundColor, Color, PaletteMap, SearchColor, StrokeColor,
};
use inferno::flamegraph::{self, defaults, Direction, Options, Palette, TextTruncateDirection};

#[cfg(feature = "nameattr")]
use inferno::flamegraph::FuncFrameAttrsMap;

#[derive(Debug, Parser)]
#[clap(name = "inferno-flamegraph", about)]
struct Opt {
    // ************* //
    // *** FLAGS *** //
    // ************* //
    /// Use consistent palette (palette.map)
    #[clap(long = "cp")]
    cp: bool,

    /// Colors are selected by hashing the function name, weighting earlier characters more
    /// heavily
    #[clap(long = "hash", conflicts_with = "deterministic")]
    hash: bool,

    /// Colors are selected such that the color of a function does not change between runs
    #[clap(long = "deterministic", conflicts_with = "hash")]
    deterministic: bool,

    /// Plot the flame graph up-side-down
    #[clap(short = 'i', long = "inverted")]
    inverted: bool,

    /// If text doesn't fit in frame, truncate right side.
    #[clap(long = "truncate-text-right")]
    truncate_text_right: bool,

    /// Switch differential hues (green<->red)
    #[clap(long = "negate")]
    negate: bool,

    /// Don't include static JavaScript in flame graph.
    /// This flag is hidden since it's only meant to be used in
    /// tests so we don't have to include the same static
    /// JavaScript in all of the test files
    #[clap(hide = true, long = "no-javascript")]
    no_javascript: bool,

    /// Don't sort the input lines.
    /// If you set this flag you need to be sure your
    /// input stack lines are already sorted
    #[clap(name = "no-sort", long = "no-sort")]
    no_sort: bool,

    /// Pretty print XML with newlines and indentation.
    #[clap(long = "pretty-xml")]
    pretty_xml: bool,

    /// Silence all log output
    #[clap(short = 'q', long = "quiet")]
    quiet: bool,

    /// Generate stack-reversed flame graph
    #[clap(long = "reverse", conflicts_with = "no-sort")]
    reverse: bool,

    /// Verbose logging mode (-v, -vv, -vvv)
    #[clap(short = 'v', long = "verbose", action = ArgAction::Count)]
    verbose: u8,

    // *************** //
    // *** OPTIONS *** //
    // *************** //
    /// Set background colors. Gradient choices are yellow (default), blue, green, grey; flat colors use "#rrggbb"
    #[clap(long = "bgcolors", value_name = "STRING")]
    bgcolors: Option<BackgroundColor>,

    /// Set color palette
    #[clap(
        short = 'c',
        long = "colors",
        default_value = defaults::COLORS,
        value_parser = clap::builder::PossibleValuesParser::new(Palette::VARIANTS).map(|s| s.parse::<Palette>().unwrap()),
        value_name = "STRING"
    )]
    colors: Palette,

    /// Color frames based on their width, highlighting expensive codepaths
    #[clap(long = "colordiffusion", conflicts_with = "colors")]
    color_diffusion: bool,

    /// Count type label
    #[clap(
        long = "countname",
        default_value = defaults::COUNT_NAME,
        value_name = "STRING"
    )]
    countname: String,

    /// Factor to scale sample counts by
    #[clap(
        long = "factor",
        default_value = &**defaults::str::FACTOR,
        value_name = "FLOAT"
    )]
    factor: f64,

    /// Font size
    #[clap(
        long = "fontsize",
        default_value = &**defaults::str::FONT_SIZE,
        value_name = "UINT"
    )]
    fontsize: usize,

    /// Font type
    #[clap(
        long = "fonttype",
        default_value = defaults::FONT_TYPE,
        value_name = "STRING"
    )]
    fonttype: String,

    /// Font width
    #[clap(
        long = "fontwidth",
        default_value = &**defaults::str::FONT_WIDTH,
        value_name = "FLOAT"
    )]
    fontwidth: f64,

    /// Color of UI text such as the search and reset zoom buttons
    #[clap(
        long = "uicolor",
        default_value = defaults::UI_COLOR,
        value_parser = |s: &str| {
            parse_hex_color(s)
                .ok_or_else(|| format!("Expected a color in hexadecimal format, got: {}", s))
        },
        value_name = "#RRGGBB"
    )]
    uicolor: Color,

    /// Height of each frame
    #[clap(
        long = "height",
        default_value = &**defaults::str::FRAME_HEIGHT,
        value_name = "UINT"
    )]
    height: usize,

    /// Omit functions smaller than <FLOAT> percent
    #[clap(
        long = "minwidth",
        default_value = &**defaults::str::MIN_WIDTH,
        value_name = "FLOAT"
    )]
    minwidth: f64,

    /// File containing attributes to use for the SVG frames of particular functions.
    /// Each line in the file should be a function name followed by a tab,
    /// then a sequence of tab separated name=value pairs
    #[cfg(feature = "nameattr")]
    #[clap(long = "nameattr", value_name = "PATH")]
    nameattr: Option<PathBuf>,

    /// Name type label
    #[clap(
        long = "nametype",
        default_value = defaults::NAME_TYPE,
        value_name = "STRING"
    )]
    nametype: String,

    /// Set embedded notes in SVG
    #[clap(long = "notes", value_name = "STRING")]
    notes: Option<String>,

    /// Search color
    #[clap(
        long = "search-color",
        default_value = defaults::SEARCH_COLOR,
        value_name = "STRING"
    )]
    search_color: SearchColor,

    /// Adds an outline to every frame
    #[clap(
        long = "stroke-color",
        default_value = defaults::STROKE_COLOR,
        value_name = "STRING"
    )]
    stroke_color: StrokeColor,

    /// Second level title (optional)
    #[clap(long = "subtitle", value_name = "STRING")]
    subtitle: Option<String>,

    /// Change title text
    #[clap(
        long = "title",
        default_value = defaults::TITLE,
        value_name = "STRING"
    )]
    title: String,

    /// Width of image
    #[clap(long = "width", value_name = "UINT")]
    width: Option<usize>,

    /// Omit samples whose stacks do not contain this symbol. When this symbol is in a sample's
    /// stack, truncate the call stack so that this is the bottom-most symbol.
    /// This is particularly useful when you want to profile a specific function in a codebase that
    /// uses some kind of heavy runtime like rayon, tokio, or the rustc query system.
    #[clap(long = "base", value_name = "STRING")]
    base: Vec<String>,

    // ************ //
    // *** ARGS *** //
    // ************ //
    /// Collapsed perf output files. With no PATH, or PATH is -, read STDIN.
    #[clap(name = "PATH", value_parser)]
    infiles: Vec<PathBuf>,

    /// Produce a flame chart (sort by time, do not merge stacks)
    #[clap(
        long = "flamechart",
        conflicts_with = "no-sort",
        conflicts_with = "reverse"
    )]
    flame_chart: bool,
}

impl<'a> Opt {
    fn into_parts(self) -> (Vec<PathBuf>, Options<'a>) {
        let mut options = Options::default();
        options.title = self.title.clone();
        options.colors = self.colors;
        options.bgcolors = self.bgcolors;
        options.hash = self.hash;
        options.deterministic = self.deterministic;

        self.set_func_frameattrs(&mut options);

        if self.inverted {
            options.direction = Direction::Inverted;
            if self.title == defaults::TITLE {
                options.title = "Icicle Graph".to_string();
            }
        }
        if self.truncate_text_right {
            options.text_truncate_direction = TextTruncateDirection::Right;
        }
        options.negate_differentials = self.negate;
        options.factor = self.factor;
        options.pretty_xml = self.pretty_xml;
        options.no_sort = self.no_sort;
        options.no_javascript = self.no_javascript;
        options.color_diffusion = self.color_diffusion;
        options.reverse_stack_order = self.reverse;
        options.flame_chart = self.flame_chart;
        options.base = self.base;

        if self.flame_chart && self.title == defaults::TITLE {
            options.title = defaults::CHART_TITLE.to_owned();
        }

        // set style options
        options.subtitle = self.subtitle;
        options.image_width = self.width;
        options.frame_height = self.height;
        options.min_width = self.minwidth;
        options.font_type = self.fonttype;
        options.font_size = self.fontsize;
        options.font_width = self.fontwidth;
        options.count_name = self.countname;
        options.name_type = self.nametype;
        if let Some(notes) = self.notes {
            options.notes = notes;
        }
        options.negate_differentials = self.negate;
        options.factor = self.factor;
        options.search_color = self.search_color;
        options.stroke_color = self.stroke_color;
        options.uicolor = self.uicolor;
        (self.infiles, options)
    }

    #[cfg(feature = "nameattr")]
    fn set_func_frameattrs(&self, options: &mut Options) {
        if let Some(file) = &self.nameattr {
            match FuncFrameAttrsMap::from_file(file) {
                Ok(m) => {
                    options.func_frameattrs = m;
                }
                Err(e) => panic!("Error reading {}: {:?}", file.display(), e),
            }
        };
    }

    #[cfg(not(feature = "nameattr"))]
    fn set_func_frameattrs(&self, _: &mut Options) {}
}

const PALETTE_MAP_FILE: &str = "palette.map"; // default name for the palette map file

fn main() -> io::Result<()> {
    let opt = Opt::parse();

    // Initialize logger
    if !opt.quiet {
        env_logger::Builder::from_env(Env::default().default_filter_or(match opt.verbose {
            0 => "warn",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }))
        .format_timestamp(None)
        .init();
    }

    let mut palette_map = match fetch_consistent_palette_if_needed(opt.cp, PALETTE_MAP_FILE) {
        Ok(palette_map) => palette_map,
        Err(e) => panic!("Error reading {}: {:?}", PALETTE_MAP_FILE, e),
    };

    let (infiles, mut options) = opt.into_parts();

    options.palette_map = palette_map.as_mut();

    if std::io::stdout().is_terminal() {
        flamegraph::from_files(&mut options, &infiles, io::stdout().lock())?;
    } else {
        flamegraph::from_files(
            &mut options,
            &infiles,
            io::BufWriter::new(io::stdout().lock()),
        )?;
    }

    save_consistent_palette_if_needed(&palette_map, PALETTE_MAP_FILE)
}

fn fetch_consistent_palette_if_needed(
    use_consistent_palette: bool,
    palette_file: &str,
) -> io::Result<Option<PaletteMap>> {
    let palette_map = if use_consistent_palette {
        let path = Path::new(palette_file);
        Some(PaletteMap::load_from_file_or_empty(&path)?)
    } else {
        None
    };

    Ok(palette_map)
}

fn save_consistent_palette_if_needed(
    palette_map: &Option<PaletteMap>,
    palette_file: &str,
) -> io::Result<()> {
    if let Some(palette_map) = palette_map {
        let path = Path::new(palette_file);
        palette_map.save_to_file(&path)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Opt;
    use clap::Parser;
    use inferno::flamegraph::{color, Direction, Options, Palette, TextTruncateDirection};
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;
    use std::str::FromStr;

    #[test]
    fn default_options() {
        let args = vec!["inferno-flamegraph", "test_infile"];
        let opt = Opt::try_parse_from(args).unwrap();
        let (_infiles, options) = opt.into_parts();
        assert_eq!(options, Options::default());
    }

    #[test]
    fn options() {
        let args = vec![
            "inferno-flamegraph",
            "--inverted",
            "--truncate-text-right",
            "--colors",
            "purple",
            "--bgcolors",
            "blue",
            "--hash",
            "--cp",
            "--search-color",
            "#203040",
            "--title",
            "Test Title",
            "--subtitle",
            "Test Subtitle",
            "--width",
            "100",
            "--height",
            "500",
            "--minwidth",
            "90.1",
            "--fonttype",
            "Helvetica",
            "--fontsize",
            "13",
            "--fontwidth",
            "10.5",
            "--countname",
            "test count name",
            "--nametype",
            "test name type",
            "--notes",
            "Test notes",
            "--negate",
            "--factor",
            "0.1",
            "--pretty-xml",
            "--reverse",
            "--no-javascript",
            "test_infile1",
            "test_infile2",
        ];
        let opt = Opt::try_parse_from(args).unwrap();
        let (infiles, options) = opt.into_parts();
        let mut expected_options = Options::default();
        expected_options.colors = Palette::from_str("purple").unwrap();
        expected_options.search_color = color::SearchColor::from_str("#203040").unwrap();
        expected_options.title = "Test Title".to_string();
        expected_options.image_width = Some(100);
        expected_options.frame_height = 500;
        expected_options.min_width = 90.1;
        expected_options.font_type = "Helvetica".to_string();
        expected_options.font_size = 13;
        expected_options.font_width = 10.5;
        expected_options.text_truncate_direction = TextTruncateDirection::Right;
        expected_options.count_name = "test count name".to_string();
        expected_options.name_type = "test name type".to_string();
        expected_options.factor = 0.1;
        expected_options.notes = "Test notes".to_string();
        expected_options.subtitle = Some("Test Subtitle".to_string());
        expected_options.bgcolors = Some(color::BackgroundColor::Blue);
        expected_options.hash = true;
        expected_options.direction = Direction::Inverted;
        expected_options.negate_differentials = true;
        expected_options.pretty_xml = true;
        expected_options.no_sort = false;
        expected_options.reverse_stack_order = true;
        expected_options.no_javascript = true;
        expected_options.color_diffusion = false;

        assert_eq!(options, expected_options);
        assert_eq!(infiles.len(), 2, "expected 2 input files");
        assert_eq!(infiles[0], PathBuf::from_str("test_infile1").unwrap());
        assert_eq!(infiles[1], PathBuf::from_str("test_infile2").unwrap());
    }
}
