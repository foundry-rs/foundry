use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::str::FromStr;

use log::warn;

use crate::flamegraph::color::Color;

/// Mapping of the association between a function name and the color used when drawing information
/// from this function.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PaletteMap(HashMap<String, Color>);

impl PaletteMap {
    /// Returns the color value corresponding to the given function name.
    pub fn get(&self, func: &str) -> Option<Color> {
        self.0.get(func).cloned()
    }

    /// Inserts a function name/color pair in the map.
    pub fn insert<S: ToString>(&mut self, func: S, color: Color) -> Option<Color> {
        self.0.insert(func.to_string(), color)
    }

    /// Provides an iterator over the elements of the map.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Color)> {
        self.0.iter().map(|(func, color)| (func.as_str(), *color))
    }

    /// Builds a mapping based on the inputs given by the reader.
    ///
    /// The reader should provide name/color pairs as text input, each pair separated by a line
    /// separator.
    ///
    /// Each line should follow the format: NAME->rgb(RED, GREEN, BLUE)
    /// where NAME is the function name, and RED, GREEN, BLUE integer values between 0 and 255
    /// included.
    /// Any line which does not follow the previous format will be ignored.
    ///
    /// This function will propagate any [`std::io::Error`] returned by the given reader.
    pub fn from_reader(reader: &mut dyn io::BufRead) -> io::Result<Self> {
        let mut map = HashMap::default();
        let mut ignored = 0;

        for line in reader.lines() {
            let line = line?;
            if let Ok((name, color)) = parse_line(&line) {
                map.insert(name.to_string(), color);
            } else {
                ignored += 1;
            }
        }

        if ignored != 0 {
            warn!("Ignored {} lines with invalid format", ignored);
        }

        Ok(PaletteMap(map))
    }

    /// Writes the palette map using the given writer.
    ///
    /// The output content will follow the same format described in
    /// [`from_reader`](Self::from_reader).
    ///
    /// The name/color pairs will be sorted by name in lexicographic order.
    pub fn to_writer(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        let mut entries = self.0.iter().collect::<Vec<_>>();
        // We sort the palette because the Perl implementation does.
        entries.sort_unstable();

        for (name, color) in entries {
            writer.write_all(
                format!("{}->rgb({},{},{})\n", name, color.r, color.g, color.b).as_bytes(),
            )?
        }

        Ok(())
    }

    /// Utility function to load a palette map from a file.
    ///
    /// The file content should follow the format described in [`from_reader`](Self::from_reader).
    ///
    /// If the file does not exist, an empty palette map is returned.
    pub fn load_from_file_or_empty(path: &dyn AsRef<Path>) -> io::Result<Self> {
        // If the file does not exist, it is probably the first call to flamegraph with a consistent
        // palette: there is nothing to load.
        if path.as_ref().exists() {
            let file = File::open(path)?;
            let mut reader = BufReader::new(file);
            PaletteMap::from_reader(&mut reader)
        } else {
            Ok(PaletteMap::default())
        }
    }

    /// Utility function to save a palette map to a file.
    ///
    /// The file content will follow the format described in [`from_reader`](Self::from_reader).
    pub fn save_to_file(&self, path: &dyn AsRef<Path>) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        self.to_writer(&mut file)
    }

    /// Returns the color value corresponding to the given function name if it is present.
    /// Otherwise compute the color, and insert the new function name/color in the map.
    pub(crate) fn find_color_for<F: FnMut(&str) -> Color>(
        &mut self,
        name: &str,
        mut compute_color: F,
    ) -> Color {
        match self.get(name) {
            Some(color) => color,
            None => {
                let color = compute_color(name);
                self.insert(name, color);
                color
            }
        }
    }
}

fn parse_line(line: &str) -> io::Result<(&str, Color)> {
    // A line is formatted like this: NAME -> rbg(RED, GREEN, BLUE)
    let mut words = line.split("->");

    let name = match words.next() {
        Some(name) => name,
        None => return Err(io::Error::from(io::ErrorKind::InvalidInput)),
    };

    let color = match words.next() {
        Some(name) => name,
        None => return Err(io::Error::from(io::ErrorKind::InvalidInput)),
    };

    if words.next().is_some() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }

    let rgb_color =
        parse_rgb_string(color).ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;

    Ok((name, rgb_color))
}

fn parse_rgb_string(s: &str) -> Option<Color> {
    let s = s.trim();

    if !s.starts_with("rgb(") || !s.ends_with(')') {
        return None;
    }

    let s = &s["rgb(".len()..s.len() - 1];
    let r_end_index = s.find(',')?;
    let r_str = s[..r_end_index].trim();
    let r = u8::from_str(r_str).ok()?;

    let s = &s[r_end_index + 1..];
    let g_end_index = s.find(',')?;
    let g_str = s[..g_end_index].trim();
    let g = u8::from_str(g_str).ok()?;

    let b_str = &s[g_end_index + 1..].trim();
    let b = u8::from_str(b_str).ok()?;

    Some(Color { r, g, b })
}

#[cfg(test)]
mod tests {
    use crate::flamegraph::color::palette_map::{parse_line, PaletteMap};
    use crate::flamegraph::color::Color;
    use pretty_assertions::assert_eq;
    use std::io::Cursor;

    macro_rules! color {
        ($r:expr, $g:expr, $b:expr) => {
            Color {
                r: $r,
                g: $g,
                b: $b,
            }
        };
    }

    #[test]
    fn palette_map_test() {
        let mut palette = PaletteMap::default();

        assert_eq!(palette.insert("foo", color!(0, 50, 255)), None);
        assert_eq!(palette.insert("bar", color!(50, 0, 60)), None);
        assert_eq!(
            palette.insert("foo", color!(80, 20, 63)),
            Some(color!(0, 50, 255))
        );
        assert_eq!(
            palette.insert("foo", color!(128, 128, 128)),
            Some(color!(80, 20, 63))
        );
        assert_eq!(palette.insert("baz", color!(255, 0, 255)), None);

        assert_eq!(palette.get("func"), None);
        assert_eq!(palette.get("bar"), Some(color!(50, 0, 60)));
        assert_eq!(palette.get("foo"), Some(color!(128, 128, 128)));
        assert_eq!(palette.get("baz"), Some(color!(255, 0, 255)));

        let mut vec = palette.iter().collect::<Vec<_>>();
        vec.sort_unstable();
        let mut iter = vec.iter();

        assert_eq!(iter.next(), Some(&("bar", color!(50, 0, 60))));
        assert_eq!(iter.next(), Some(&("baz", color!(255, 0, 255))));
        assert_eq!(iter.next(), Some(&("foo", color!(128, 128, 128))));
        assert_eq!(iter.next(), None);

        let mut buf = Cursor::new(Vec::new());

        palette.to_writer(&mut buf).unwrap();
        buf.set_position(0);
        let palette = PaletteMap::from_reader(&mut buf).unwrap();

        let mut vec = palette.iter().collect::<Vec<_>>();
        vec.sort_unstable();
        let mut iter = vec.iter();

        assert_eq!(iter.next(), Some(&("bar", color!(50, 0, 60))));
        assert_eq!(iter.next(), Some(&("baz", color!(255, 0, 255))));
        assert_eq!(iter.next(), Some(&("foo", color!(128, 128, 128))));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn parse_line_test() {
        assert_eq!(
            parse_line("func->rgb(0, 0, 0)").unwrap(),
            ("func", color!(0, 0, 0))
        );
        assert_eq!(
            parse_line("->rgb(255, 255, 255)").unwrap(),
            ("", color!(255, 255, 255))
        );

        assert!(parse_line("").is_err());
        assert!(parse_line("func->(0, 0, 0)").is_err());
        assert!(parse_line("func->").is_err());
        assert!(parse_line("func->foo->rgb(0, 0, 0)").is_err());
        assert!(parse_line("func->rgb(0, 0, 0)->foo").is_err());
        assert!(parse_line("func->rgb(255, 255, 256)").is_err());
        assert!(parse_line("func->rgb(-1, 255, 255)").is_err());
    }

    #[test]
    fn load_from_non_existing_file() {
        let palette_map = PaletteMap::load_from_file_or_empty(&"non-existing-palette.map").unwrap();
        assert_eq!(palette_map, PaletteMap::default());
    }
}
