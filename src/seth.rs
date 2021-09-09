use rustc_hex::ToHex;

#[derive(Default)]
pub struct Seth {}

impl Seth {
    pub fn new() -> Self {
        Self {}
    }

    /// Converts ASCII text input to hex
    ///
    /// ```
    /// use dapptools::seth::Seth;
    ///
    /// let bin = Seth::from_ascii("yo");
    /// assert_eq!(bin, "0x796f")
    ///
    /// ```
    pub fn from_ascii(s: &str) -> String {
        let s: String = s.as_bytes().to_hex();
        format!("0x{}", s)
    }
}
