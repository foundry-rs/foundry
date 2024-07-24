use reqwest::Url;

/// Returns `true` if the URL only consists of host.
///
/// This is used to check user input url for missing /api path
#[inline]
pub fn is_host_only(url: &Url) -> bool {
    matches!(url.path(), "/" | "")
}
