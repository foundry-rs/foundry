use std::path::{Path, PathBuf};

use gix_object::bstr::{BStr, BString, ByteSlice, ByteVec};

use crate::{FullName, FullNameRef, Namespace, PartialNameRef};

impl Namespace {
    /// Dissolve ourselves into the interior representation
    pub fn into_bstring(self) -> BString {
        self.0
    }
    /// Return ourselves as
    pub fn as_bstr(&self) -> &BStr {
        self.0.as_ref()
    }
    /// Return ourselves as a path for use within the filesystem.
    pub fn to_path(&self) -> &Path {
        gix_path::from_byte_slice(&self.0)
    }
    /// Append the given `prefix` to this namespace so it becomes usable for prefixed iteration.
    pub fn into_namespaced_prefix(mut self, prefix: &Path) -> PathBuf {
        let prefix = gix_path::into_bstr(prefix);
        self.0.push_str(prefix.as_ref());
        gix_path::to_native_path_on_windows(self.0).into_owned()
    }
    pub(crate) fn into_namespaced_name(mut self, name: &FullNameRef) -> FullName {
        self.0.push_str(name.as_bstr());
        FullName(self.0)
    }
}

/// Given a `namespace` 'foo we output 'refs/namespaces/foo', and given 'foo/bar' we output 'refs/namespaces/foo/refs/namespaces/bar'.
///
/// For more information, consult the [git namespace documentation](https://git-scm.com/docs/gitnamespaces).
pub fn expand<'a, Name, E>(namespace: Name) -> Result<Namespace, gix_validate::reference::name::Error>
where
    Name: TryInto<&'a PartialNameRef, Error = E>,
    gix_validate::reference::name::Error: From<E>,
{
    let namespace = &namespace.try_into()?.0;
    let mut out = BString::default();
    for component in namespace.split_str(b"/") {
        out.push_str("refs/namespaces/");
        out.push_str(component);
        out.push_str(b"/");
    }
    Ok(Namespace(out))
}
