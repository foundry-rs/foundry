pub(crate) mod contents {
    pub const INDEX_HTML: &str = include_str!("assets/index.html");
    pub const STYLES_CSS: &str = include_str!("assets/styles.css");
    pub const MAIN_JS: &str = include_str!("assets/main.js");
    pub const BANNER_PNG: &[u8] = include_bytes!("assets/banner.png");
    pub const LOGO_PNG: &[u8] = include_bytes!("assets/logo.png");
}
