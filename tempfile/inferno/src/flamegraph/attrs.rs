use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use ahash::AHashMap;
use indexmap::map::Entry;
use log::warn;

type AttrMap<K, V> = indexmap::IndexMap<K, V, ahash::RandomState>;

macro_rules! unwrap_or_continue {
    ($e:expr) => {{
        if let Some(x) = $e {
            x
        } else {
            continue;
        }
    }};
}

/// Provides a way to customize the attributes on the SVG elements for a frame.
#[derive(PartialEq, Eq, Debug, Default)]
pub struct FuncFrameAttrsMap(AHashMap<String, FrameAttrs>);

impl FuncFrameAttrsMap {
    /// Parse frame attributes from a file.
    ///
    /// Each line should consist of a function name, a tab (`\t`), and then a sequence of
    /// tab-separated `name=value` pairs.
    pub fn from_file(path: &Path) -> io::Result<FuncFrameAttrsMap> {
        let file = BufReader::new(File::open(path)?);
        FuncFrameAttrsMap::from_reader(file)
    }

    /// Parse frame attributes from a `BufRead`.
    ///
    /// Each line should consist of a function name, a tab (`\t`), and then a sequence of
    /// tab-separated `name=value` pairs.
    pub fn from_reader<R: BufRead>(mut reader: R) -> io::Result<FuncFrameAttrsMap> {
        let mut funcattr_map = FuncFrameAttrsMap::default();
        let mut line = Vec::new();
        loop {
            line.clear();

            if reader.read_until(0x0A, &mut line)? == 0 {
                break;
            }

            let l = String::from_utf8_lossy(&line);
            let mut line = l.trim().splitn(2, '\t');
            let func = unwrap_or_continue!(line.next());
            if func.is_empty() {
                continue;
            }
            let funcattrs = funcattr_map.0.entry(func.to_string()).or_default();
            let namevals = unwrap_or_continue!(line.next());
            for nameval in namevals.split('\t') {
                let mut nameval = nameval.splitn(2, '=');
                let name = unwrap_or_continue!(nameval.next()).trim();
                if name.is_empty() {
                    continue;
                }
                let mut value = unwrap_or_continue!(nameval.next()).trim();
                // Remove optional quotes
                if value.starts_with('"') && value.ends_with('"') {
                    value = &value[1..value.len() - 1];
                }
                match name {
                    "title" => {
                        funcattrs.title = Some(value.to_string());
                    }
                    "href" => {
                        funcattrs.add_attr(func, "xlink:href".to_string(), value.to_string());
                    }
                    "id" | "class" | "target" => {
                        funcattrs.add_attr(func, name.to_string(), value.to_string());
                    }
                    "g_extra" | "a_extra" => funcattrs.parse_extra_attrs(func, value),
                    _ => warn!("invalid attribute {} found for {}", name, func),
                }
            }

            if funcattrs.attrs.contains_key("xlink:href") && !funcattrs.attrs.contains_key("target")
            {
                funcattrs
                    .attrs
                    .insert("target".to_string(), "_top".to_string());
            }
        }

        Ok(funcattr_map)
    }

    /// Return FrameAttrs for the given function name if it exists
    pub(super) fn frameattrs_for_func(&self, func: &str) -> Option<&FrameAttrs> {
        self.0.get(func)
    }
}

/// Attributes to set on the SVG elements of a frame
#[derive(PartialEq, Eq, Debug, Default)]
pub(super) struct FrameAttrs {
    /// The text to include in the `title` element.
    /// If set to None, the title is dynamically generated based on the function name.
    pub(super) title: Option<String>,

    pub(super) attrs: AttrMap<String, String>,
}

impl FrameAttrs {
    fn add_attr(&mut self, func: &str, name: String, value: String) {
        match self.attrs.entry(name) {
            Entry::Occupied(mut e) => {
                warn!(
                    "duplicate attribute `{}` in nameattr file for `{}`; replacing value \"{}\" with \"{}\"",
                    e.key(), func, e.get(), value
                );
                e.insert(value);
            }
            Entry::Vacant(e) => {
                e.insert(value);
            }
        };
    }

    fn parse_extra_attrs(&mut self, func: &str, s: &str) {
        AttrIter { s }.for_each(|(name, value)| {
            self.add_attr(func, name, value);
        });
    }
}

struct AttrIter<'a> {
    s: &'a str,
}

impl<'a> Iterator for AttrIter<'a> {
    type Item = (String, String);

    fn next(&mut self) -> Option<(String, String)> {
        let mut name_rest = self.s.splitn(2, '=');
        let name = name_rest.next()?.trim();
        if name.is_empty() {
            warn!("\"=\" found with no name in extra attributes");
            return None;
        }
        let mut split_name = name.split_whitespace();
        let name = split_name.next_back()?;
        for extra in split_name {
            warn!(
                "extra attribute {} has no value (did you mean to quote the value?)",
                extra
            );
        }

        let rest = name_rest.next()?.trim_start();
        if rest.is_empty() {
            warn!("no value after \"=\" for extra attribute {}", name);
        }

        let (value, rest) = if let Some(stripped_rest) = rest.strip_prefix('"') {
            if let Some(eq) = stripped_rest.find('"') {
                (&rest[1..=eq], &rest[eq + 1..])
            } else {
                warn!("no end quote found for extra attribute {}", name);
                return None;
            }
        } else if let Some(w) = rest.find(char::is_whitespace) {
            (&rest[..w], &rest[w + 1..])
        } else {
            (rest, "")
        };

        self.s = rest;

        Some((name.to_string(), value.to_string()))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use ahash::AHashMap;
    use maplit::{convert_args, hashmap};
    use pretty_assertions::assert_eq;

    #[test]
    fn func_frame_attrs_map_from_reader() {
        let foo = [
            "foo",
            // Without quotes
            "title=foo title",
            // With quotes
            r#"class="foo class""#,
            // gextra1 without quotes, gextra2 with quotes
            r#"g_extra=gextra1=gextra1 gextra2="foo gextra2""#,
            "href=foo href",
            "target=foo target",
            // Extra quotes around a_extra value
            r#"a_extra="aextra1="foo aextra1" aextra2="foo aextra2"""#,
        ]
        .join("\t");

        let bar = [
            "bar",
            "class=bar class",
            "href=bar href",
            // With an invalid attribute that has no value
            // This gets skipped and logged.
            r#"a_extra=aextra1=foo invalid aextra2=bar"#,
        ]
        .join("\t");

        let s = [foo, bar].join("\n");
        let r = s.as_bytes();

        let mut expected_inner = AHashMap::default();
        let foo_attrs: AttrMap<String, String> = convert_args!(hashmap!(
            "class" => "foo class",
            "xlink:href" => "foo href",
            "target" => "foo target",
            "gextra1" => "gextra1",
            "gextra2" => "foo gextra2",
            "aextra1" => "foo aextra1",
            "aextra2" => "foo aextra2",
        ))
        .into_iter()
        .collect();

        expected_inner.insert(
            "foo".to_owned(),
            FrameAttrs {
                title: Some("foo title".to_owned()),
                attrs: foo_attrs,
            },
        );

        let bar_attrs: AttrMap<String, String> = convert_args!(hashmap!(
            "class" => "bar class",
            "xlink:href" => "bar href",
            "aextra1" => "foo",
            "aextra2" => "bar",
            "target" => "_top",
        ))
        .into_iter()
        .collect();

        expected_inner.insert(
            "bar".to_owned(),
            FrameAttrs {
                title: None,
                attrs: bar_attrs,
            },
        );

        let result = FuncFrameAttrsMap::from_reader(r).unwrap();
        let expected = FuncFrameAttrsMap(expected_inner);

        assert_eq!(result, expected);
    }
}
