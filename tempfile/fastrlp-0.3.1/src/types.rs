#[derive(Clone, Default, PartialEq, Eq)]
pub struct Header {
    pub list: bool,
    pub payload_length: usize,
}

pub const EMPTY_STRING_CODE: u8 = 0x80;
pub const EMPTY_LIST_CODE: u8 = 0xC0;
