/// Embedded web assets for the browser wallet interface
pub mod web {
    /// HTML index page
    pub const INDEX_HTML: &str = include_str!("assets/web/index.html");
    
    /// JavaScript files
    pub mod js {
        pub const MAIN_JS: &str = include_str!("assets/web/js/main.js");
        pub const WALLET_JS: &str = include_str!("assets/web/js/wallet.js");
        pub const POLLING_JS: &str = include_str!("assets/web/js/polling.js");
        pub const UTILS_JS: &str = include_str!("assets/web/js/utils.js");
    }
    
    /// CSS files
    pub mod css {
        pub const STYLES_CSS: &str = include_str!("assets/web/css/styles.css");
    }
}