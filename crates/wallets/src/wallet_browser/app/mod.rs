pub(crate) mod contents {
    pub(crate) const INDEX_HTML: &str = include_str!("assets/index.html");
    pub(crate) const STYLES_CSS: &str = include_str!("assets/styles.css");
    pub(crate) const MAIN_JS: &str = include_str!("assets/main.js");
    pub(crate) const BANNER_PNG: &[u8] = include_bytes!("assets/banner.png");
    pub(crate) const LOGO_PNG: &[u8] = include_bytes!("assets/logo.png");
}
