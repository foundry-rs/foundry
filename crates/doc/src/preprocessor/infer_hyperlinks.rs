use super::{Preprocessor, PreprocessorId};
use crate::{Comments, Document, ParseItem, ParseSource, solang_ext::SafeUnwrap};
use regex::{Captures, Match, Regex};
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::LazyLock,
};

/// A regex that matches `{identifier-part}` placeholders
///
/// Overloaded functions are referenced by including the exact function arguments in the `part`
/// section of the placeholder.
static RE_INLINE_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)(\{(?P<xref>xref-)?(?P<identifier>[a-zA-Z_][0-9a-zA-Z_]*)(-(?P<part>[a-zA-Z_][0-9a-zA-Z_-]*))?}(\[(?P<link>(.*?))\])?)").unwrap()
});

/// [InferInlineHyperlinks] preprocessor id.
pub const INFER_INLINE_HYPERLINKS_ID: PreprocessorId = PreprocessorId("infer inline hyperlinks");

/// The infer hyperlinks preprocessor tries to map @dev tags to referenced items
/// Traverses the documents and attempts to find referenced items
/// comments for dev comment tags.
///
/// This preprocessor replaces inline links in comments with the links to the referenced items.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct InferInlineHyperlinks;

impl Preprocessor for InferInlineHyperlinks {
    fn id(&self) -> PreprocessorId {
        INFER_INLINE_HYPERLINKS_ID
    }

    fn preprocess(&self, mut documents: Vec<Document>) -> Result<Vec<Document>, eyre::Error> {
        // traverse all comments and try to match inline links and replace with inline links for
        // markdown
        let mut docs = Vec::with_capacity(documents.len());
        while !documents.is_empty() {
            let mut document = documents.remove(0);
            let target_path = document.relative_output_path().to_path_buf();
            for idx in 0..document.content.len() {
                let (mut comments, item_children_len) = {
                    let item = document.content.get_mut(idx).unwrap();
                    let comments = std::mem::take(&mut item.comments);
                    let children = item.children.len();
                    (comments, children)
                };
                Self::inline_doc_links(&documents, &target_path, &mut comments, &document);
                document.content.get_mut(idx).unwrap().comments = comments;

                // we also need to iterate over all child items
                // This is a bit horrible but we need to traverse all items in all documents
                for child_idx in 0..item_children_len {
                    let mut comments = {
                        let item = document.content.get_mut(idx).unwrap();

                        std::mem::take(&mut item.children[child_idx].comments)
                    };
                    Self::inline_doc_links(&documents, &target_path, &mut comments, &document);
                    document.content.get_mut(idx).unwrap().children[child_idx].comments = comments;
                }
            }

            docs.push(document);
        }

        Ok(docs)
    }
}

impl InferInlineHyperlinks {
    /// Finds the first match for the given link.
    ///
    /// All items get their own section in the markdown file.
    /// This section uses the identifier of the item: `#functionname`
    ///
    /// Note: the target path is the relative path to the markdown file being searched.
    /// The current_path is the path of the document where the link appears.
    fn find_match<'a>(
        link: &InlineLink<'a>,
        target_path: &Path,
        current_path: &Path,
        items: impl Iterator<Item = &'a ParseItem>,
    ) -> Option<InlineLinkTarget<'a>> {
        for item in items {
            match &item.source {
                ParseSource::Contract(contract) => {
                    let name = &contract.name.safe_unwrap().name;
                    if name == link.identifier {
                        if link.part.is_none() {
                            return Some(InlineLinkTarget::borrowed(
                                name,
                                target_path.to_path_buf(),
                                current_path.to_path_buf(),
                            ));
                        }
                        // try to find the referenced item in the contract's children
                        return Self::find_match(link, target_path, current_path, item.children.iter());
                    }
                }
                ParseSource::Function(fun) => {
                    // TODO: handle overloaded functions
                    // functions can be overloaded so we need to keep track of how many matches we
                    // have so we can match the correct one
                    if let Some(id) = &fun.name {
                        // Note: constructors don't have a name
                        if id.name == link.ref_name() {
                            return Some(InlineLinkTarget::borrowed(
                                &id.name,
                                target_path.to_path_buf(),
                                current_path.to_path_buf(),
                            ));
                        }
                    } else if link.ref_name() == "constructor" {
                        return Some(InlineLinkTarget::borrowed(
                            "constructor",
                            target_path.to_path_buf(),
                            current_path.to_path_buf(),
                        ));
                    }
                }
                ParseSource::Variable(_) => {}
                ParseSource::Event(ev) => {
                    let ev_name = &ev.name.safe_unwrap().name;
                    if ev_name == link.ref_name() {
                        return Some(InlineLinkTarget::borrowed(
                            ev_name,
                            target_path.to_path_buf(),
                            current_path.to_path_buf(),
                        ));
                    }
                }
                ParseSource::Error(err) => {
                    let err_name = &err.name.safe_unwrap().name;
                    if err_name == link.ref_name() {
                        return Some(InlineLinkTarget::borrowed(
                            err_name,
                            target_path.to_path_buf(),
                            current_path.to_path_buf(),
                        ));
                    }
                }
                ParseSource::Struct(structdef) => {
                    let struct_name = &structdef.name.safe_unwrap().name;
                    if struct_name == link.ref_name() {
                        return Some(InlineLinkTarget::borrowed(
                            struct_name,
                            target_path.to_path_buf(),
                            current_path.to_path_buf(),
                        ));
                    }
                }
                ParseSource::Enum(_) => {}
                ParseSource::Type(_) => {}
            }
        }

        None
    }

    /// Attempts to convert inline links to markdown links.
    fn inline_doc_links(
        documents: &[Document],
        current_path: &Path,
        comments: &mut Comments,
        parent: &Document,
    ) {
        // loop over all comments in the item
        for comment in comments.iter_mut() {
            let val = comment.value.clone();
            // replace all links with inline markdown links
            for link in InlineLink::captures(val.as_str()) {
                // First, try to find a match in the current document.
                // This handles both simple `{functionName}` and `{Contract-functionName}`
                // when the contract and function are in the same file.
                let target = Self::find_match(
                    &link,
                    current_path,
                    current_path,
                    parent
                        .content
                        .iter_items()
                        .flat_map(|item| Some(item).into_iter().chain(item.children.iter())),
                )
                .or_else(|| {
                    // If not found locally, search in all other documents
                    documents.iter().find_map(|doc| {
                        Self::find_match(
                            &link,
                            doc.relative_output_path(),
                            current_path,
                            doc.content.iter_items().flat_map(|item| {
                                Some(item).into_iter().chain(item.children.iter())
                            }),
                        )
                    })
                });

                if let Some(target) = target {
                    let display_value = link.markdown_link_display_value();
                    let markdown_link = format!("[{display_value}]({target})");
                    // replace the link with the markdown link
                    comment.value =
                        comment.value.as_str().replacen(link.as_str(), markdown_link.as_str(), 1);
                }
            }
        }
    }
}

struct InlineLinkTarget<'a> {
    section: Cow<'a, str>,
    target_path: PathBuf,
    current_path: PathBuf,
}

impl<'a> InlineLinkTarget<'a> {
    fn borrowed(section: &'a str, target_path: PathBuf, current_path: PathBuf) -> Self {
        Self { section: Cow::Borrowed(section), target_path, current_path }
    }
}

impl std::fmt::Display for InlineLinkTarget<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let anchor = self.section.to_lowercase();

        if self.target_path == self.current_path {
            write!(f, "#{anchor}")
        } else {
            let link = make_relative_link(&self.current_path, &self.target_path);
            write!(f, "{link}#{anchor}")
        }
    }
}

/// Computes a relative link from the current document path to the target document path.
/// Both paths should be relative paths from the same root (e.g., `src/Foo.sol/contract.Foo.md`).
pub fn make_relative_link(current_path: &Path, target_path: &Path) -> String {
    let current_dir = current_path.parent().unwrap_or(Path::new("."));

    let mut current_components: Vec<_> = current_dir.components().collect();
    let mut target_components: Vec<_> = target_path.components().collect();

    while !current_components.is_empty()
        && !target_components.is_empty()
        && current_components[0] == target_components[0]
    {
        current_components.remove(0);
        target_components.remove(0);
    }

    let mut result = PathBuf::new();
    for _ in &current_components {
        result.push("..");
    }
    for component in &target_components {
        result.push(component);
    }

    if result.as_os_str().is_empty() {
        ".".to_string()
    } else {
        result.display().to_string().replace('\\', "/")
    }
}

/// A parsed link to an item.
#[derive(Debug)]
struct InlineLink<'a> {
    outer: Match<'a>,
    identifier: &'a str,
    part: Option<&'a str>,
    link: Option<&'a str>,
}

impl<'a> InlineLink<'a> {
    fn from_capture(cap: Captures<'a>) -> Option<Self> {
        Some(Self {
            outer: cap.get(1)?,
            identifier: cap.name("identifier")?.as_str(),
            part: cap.name("part").map(|m| m.as_str()),
            link: cap.name("link").map(|m| m.as_str()),
        })
    }

    fn captures(s: &'a str) -> impl Iterator<Item = Self> + 'a {
        RE_INLINE_LINK.captures_iter(s).filter_map(Self::from_capture)
    }

    /// Parses the first inline link.
    #[allow(unused)]
    fn capture(s: &'a str) -> Option<Self> {
        let cap = RE_INLINE_LINK.captures(s)?;
        Self::from_capture(cap)
    }

    /// Returns the name of the link
    fn markdown_link_display_value(&self) -> Cow<'_, str> {
        if let Some(link) = self.link {
            Cow::Borrowed(link)
        } else if let Some(part) = self.part {
            Cow::Owned(format!("{}-{}", self.identifier, part))
        } else {
            Cow::Borrowed(self.identifier)
        }
    }

    /// Returns the name of the referenced item.
    fn ref_name(&self) -> &str {
        self.exact_identifier().split('-').next().unwrap()
    }

    fn exact_identifier(&self) -> &str {
        let mut name = self.identifier;
        if let Some(part) = self.part {
            name = part;
        }
        name
    }

    /// Returns the content of the matched link.
    fn as_str(&self) -> &str {
        self.outer.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_inline_links() {
        let s = "    {IERC165-supportsInterface}   ";
        let cap = RE_INLINE_LINK.captures(s).unwrap();

        let identifier = cap.name("identifier").unwrap().as_str();
        assert_eq!(identifier, "IERC165");
        let part = cap.name("part").unwrap().as_str();
        assert_eq!(part, "supportsInterface");

        let s = "    {supportsInterface}   ";
        let cap = RE_INLINE_LINK.captures(s).unwrap();

        let identifier = cap.name("identifier").unwrap().as_str();
        assert_eq!(identifier, "supportsInterface");

        let s = "{xref-ERC721-_safeMint-address-uint256-}";
        let cap = RE_INLINE_LINK.captures(s).unwrap();

        let identifier = cap.name("identifier").unwrap().as_str();
        assert_eq!(identifier, "ERC721");
        let identifier = cap.name("xref").unwrap().as_str();
        assert_eq!(identifier, "xref-");
        let identifier = cap.name("part").unwrap().as_str();
        assert_eq!(identifier, "_safeMint-address-uint256-");

        let link = InlineLink::capture(s).unwrap();
        assert_eq!(link.ref_name(), "_safeMint");
        assert_eq!(link.as_str(), "{xref-ERC721-_safeMint-address-uint256-}");

        let s = "{xref-ERC721-_safeMint-address-uint256-}[`Named link`]";
        let link = InlineLink::capture(s).unwrap();
        assert_eq!(link.link, Some("`Named link`"));
        assert_eq!(link.markdown_link_display_value(), "`Named link`");
    }

    #[test]
    fn test_make_relative_link() {
        // Same directory
        let current = Path::new("src/Foo.sol/contract.Foo.md");
        let target = Path::new("src/Foo.sol/interface.IFoo.md");
        assert_eq!(make_relative_link(current, target), "interface.IFoo.md");

        // Different directory (go up one level)
        let current = Path::new("src/Borrower.sol/contract.Borrower.md");
        let target = Path::new("src/Policy.sol/abstract.Policy.md");
        assert_eq!(make_relative_link(current, target), "../Policy.sol/abstract.Policy.md");

        // Same file should return just the filename (edge case for anchors)
        let current = Path::new("src/Foo.sol/library.ECDSA.md");
        let target = Path::new("src/Foo.sol/library.ECDSA.md");
        assert_eq!(make_relative_link(current, target), "library.ECDSA.md");

        // Deep nesting
        let current = Path::new("src/a/b/c/contract.C.md");
        let target = Path::new("src/x/y/contract.Y.md");
        assert_eq!(make_relative_link(current, target), "../../../x/y/contract.Y.md");
    }
}
