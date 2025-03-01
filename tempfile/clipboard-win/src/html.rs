use core::{cmp, fmt};

pub const SEP: char = ':';
pub const NEWLINE: &str = "\r\n";
pub const LEN_SIZE: usize = 10;
pub const VERSION: &str = "Version";
pub const START_FRAGMENT: &str = "StartFragment";
pub const END_FRAGMENT: &str = "EndFragment";
pub const START_HTML: &str = "StartHTML";
pub const END_HTML: &str = "EndHTML";
pub const BODY_HEADER: &str = "<html>\r\n<body>\r\n<!--StartFragment-->";
pub const BODY_FOOTER: &str = "<!--EndFragment-->\r\n</body>\r\n</html>";

pub struct LengthBuffer([u8; LEN_SIZE]);

impl LengthBuffer {
    #[inline(always)]
    pub const fn new() -> Self {
        Self([b'0'; LEN_SIZE])
    }

    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[inline(always)]
    pub const fn as_ptr(&self) -> *const u8 {
        self.0.as_ptr()
    }
}

impl AsRef<[u8]> for LengthBuffer {
    #[inline(always)]
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl fmt::Write for LengthBuffer {
    fn write_str(&mut self, input: &str) -> fmt::Result {
        debug_assert!(input.len() <= self.0.len());
        let size = cmp::min(input.len(), self.0.len());

        self.0[10-size..].copy_from_slice(&input.as_bytes()[..size]);

        Ok(())
    }
}

//Samples
//DATA=Version:0.9
//StartHTML:0000000187
//EndHTML:0000001902
//StartFragment:0000000223
//EndFragment:0000001866
//<html>
//<body>
//<!--StartFragment-->
//<table style="color: rgb(255, 255, 255); font-style: normal; font-variant-ligatures: normal; font-variant-caps: normal; font-weight: 400; letter-spacing: normal; orphans: 2; text-align: start; text-transform: none; widows: 2; word-spacing: 0px; -webkit-text-stroke-width: 0px; text-decoration-thickness: initial; text-decoration-style: initial; text-decoration-color: initial;"><tbody></tbody></table>
//<!--EndFragment-->
//</body>
//</html>

//Version:0.9\r\nStartHTML:0000000105\r\nEndHTML:0000000187\r\nStartFragment:0000000141\r\nEndFragment:0000000151\r\n<html>\r\n<body>\r\n<!--StartFragment--><tr>1</tr><!--EndFragment-->\r\n</body>\r\n</html>
