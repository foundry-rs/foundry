use bstr::BStr;
use winnow::{error::ParserError, prelude::*};

use crate::{tree, tree::EntryRef, TreeRef, TreeRefIter};

impl<'a> TreeRefIter<'a> {
    /// Instantiate an iterator from the given tree data.
    pub fn from_bytes(data: &'a [u8]) -> TreeRefIter<'a> {
        TreeRefIter { data }
    }

    /// Follow a sequence of `path` components starting from this instance, and look them up in `odb` one by one using `buffer`
    /// until the last component is looked up and its tree entry is returned.
    ///
    /// # Performance Notes
    ///
    /// Searching tree entries is currently done in sequence, which allows the search to be allocation free. It would be possible
    /// to reuse a vector and use a binary search instead, which might be able to improve performance over all.
    /// However, a benchmark should be created first to have some data and see which trade-off to choose here.
    pub fn lookup_entry<I, P>(
        &self,
        odb: impl crate::Find,
        buffer: &'a mut Vec<u8>,
        path: I,
    ) -> Result<Option<tree::Entry>, crate::find::Error>
    where
        I: IntoIterator<Item = P>,
        P: PartialEq<BStr>,
    {
        buffer.clear();

        let mut path = path.into_iter().peekable();
        buffer.extend_from_slice(self.data);
        while let Some(component) = path.next() {
            match TreeRefIter::from_bytes(buffer)
                .filter_map(Result::ok)
                .find(|entry| component.eq(entry.filename))
            {
                Some(entry) => {
                    if path.peek().is_none() {
                        return Ok(Some(entry.into()));
                    } else {
                        let next_id = entry.oid.to_owned();
                        let obj = odb.try_find(&next_id, buffer)?;
                        let Some(obj) = obj else { return Ok(None) };
                        if !obj.kind.is_tree() {
                            return Ok(None);
                        }
                    }
                }
                None => return Ok(None),
            }
        }
        Ok(None)
    }

    /// Like [`Self::lookup_entry()`], but takes any [`AsRef<Path>`](`std::path::Path`) directly via `relative_path`,
    /// a path relative to this tree.
    /// `odb` and `buffer` are used to lookup intermediate trees.
    ///
    /// # Note
    ///
    /// If any path component contains illformed UTF-8 and thus can't be converted to bytes on platforms which can't do so natively,
    /// the returned component will be empty which makes the lookup fail.
    pub fn lookup_entry_by_path(
        &self,
        odb: impl crate::Find,
        buffer: &'a mut Vec<u8>,
        relative_path: impl AsRef<std::path::Path>,
    ) -> Result<Option<tree::Entry>, crate::find::Error> {
        use crate::bstr::ByteSlice;
        self.lookup_entry(
            odb,
            buffer,
            relative_path.as_ref().components().map(|c: std::path::Component<'_>| {
                gix_path::os_str_into_bstr(c.as_os_str())
                    .unwrap_or_else(|_| "".into())
                    .as_bytes()
            }),
        )
    }
}

impl<'a> TreeRef<'a> {
    /// Deserialize a Tree from `data`.
    pub fn from_bytes(mut data: &'a [u8]) -> Result<TreeRef<'a>, crate::decode::Error> {
        let input = &mut data;
        match decode::tree.parse_next(input) {
            Ok(tag) => Ok(tag),
            Err(err) => Err(crate::decode::Error::with_err(err, input)),
        }
    }

    /// Find an entry named `name` knowing if the entry is a directory or not, using a binary search.
    ///
    /// Note that it's impossible to binary search by name alone as the sort order is special.
    pub fn bisect_entry(&self, name: &BStr, is_dir: bool) -> Option<EntryRef<'a>> {
        static NULL_HASH: gix_hash::ObjectId = gix_hash::Kind::shortest().null();

        let search = EntryRef {
            mode: if is_dir {
                tree::EntryKind::Tree
            } else {
                tree::EntryKind::Blob
            }
            .into(),
            filename: name,
            oid: &NULL_HASH,
        };
        self.entries
            .binary_search_by(|e| e.cmp(&search))
            .ok()
            .map(|idx| self.entries[idx])
    }

    /// Create an instance of the empty tree.
    ///
    /// It's particularly useful as static part of a program.
    pub const fn empty() -> TreeRef<'static> {
        TreeRef { entries: Vec::new() }
    }
}

impl<'a> TreeRefIter<'a> {
    /// Consume self and return all parsed entries.
    pub fn entries(self) -> Result<Vec<EntryRef<'a>>, crate::decode::Error> {
        self.collect()
    }
}

impl<'a> Iterator for TreeRefIter<'a> {
    type Item = Result<EntryRef<'a>, crate::decode::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.data.is_empty() {
            return None;
        }
        match decode::fast_entry(self.data) {
            Some((data_left, entry)) => {
                self.data = data_left;
                Some(Ok(entry))
            }
            None => {
                let failing = self.data;
                self.data = &[];
                #[allow(clippy::unit_arg)]
                Some(Err(crate::decode::Error::with_err(
                    winnow::error::ErrMode::from_error_kind(&failing, winnow::error::ErrorKind::Verify),
                    failing,
                )))
            }
        }
    }
}

impl<'a> TryFrom<&'a [u8]> for tree::EntryMode {
    type Error = &'a [u8];

    fn try_from(mode: &'a [u8]) -> Result<Self, Self::Error> {
        mode_from_decimal(mode)
            .map(|(mode, _rest)| tree::EntryMode(mode as u16))
            .ok_or(mode)
    }
}

fn mode_from_decimal(i: &[u8]) -> Option<(u32, &[u8])> {
    let mut mode = 0u32;
    let mut spacer_pos = 1;
    for b in i.iter().take_while(|b| **b != b' ') {
        if *b < b'0' || *b > b'7' {
            return None;
        }
        mode = (mode << 3) + u32::from(b - b'0');
        spacer_pos += 1;
    }
    if i.len() < spacer_pos {
        return None;
    }
    let (_, i) = i.split_at(spacer_pos);
    Some((mode, i))
}

impl TryFrom<u32> for tree::EntryMode {
    type Error = u32;

    fn try_from(mode: u32) -> Result<Self, Self::Error> {
        Ok(match mode {
            0o40000 | 0o120000 | 0o160000 => tree::EntryMode(mode as u16),
            blob_mode if blob_mode & 0o100000 == 0o100000 => tree::EntryMode(mode as u16),
            _ => return Err(mode),
        })
    }
}

mod decode {
    use bstr::ByteSlice;
    use winnow::{error::ParserError, prelude::*};

    use crate::{
        tree,
        tree::{ref_iter::mode_from_decimal, EntryRef},
        TreeRef,
    };

    pub fn fast_entry(i: &[u8]) -> Option<(&[u8], EntryRef<'_>)> {
        let (mode, i) = mode_from_decimal(i)?;
        let mode = tree::EntryMode::try_from(mode).ok()?;
        let (filename, i) = i.split_at(i.find_byte(0)?);
        let i = &i[1..];
        const HASH_LEN_FIXME: usize = 20; // TODO(SHA256): know actual/desired length or we may overshoot
        let (oid, i) = match i.len() {
            len if len < HASH_LEN_FIXME => return None,
            _ => i.split_at(20),
        };
        Some((
            i,
            EntryRef {
                mode,
                filename: filename.as_bstr(),
                oid: gix_hash::oid::try_from_bytes(oid).expect("we counted exactly 20 bytes"),
            },
        ))
    }

    pub fn tree<'a, E: ParserError<&'a [u8]>>(i: &mut &'a [u8]) -> PResult<TreeRef<'a>, E> {
        let mut out = Vec::new();
        let mut i = &**i;
        while !i.is_empty() {
            let Some((rest, entry)) = fast_entry(i) else {
                #[allow(clippy::unit_arg)]
                return Err(winnow::error::ErrMode::from_error_kind(
                    &i,
                    winnow::error::ErrorKind::Verify,
                ));
            };
            i = rest;
            out.push(entry);
        }
        Ok(TreeRef { entries: out })
    }
}
