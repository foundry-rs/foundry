pub fn byte_offset_to_position(source: &str, byte_offset: usize) -> (u32, u32) {
    let mut line = 0;
    let mut col = 0;
    let mut i = 0;

    let bytes = source.as_bytes();
    while i < byte_offset && i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                line += 1;
                col = 0;
                i += 1;
            }
            b'\r' if i + 1 < bytes.len() && bytes[i + 1] == b'\n' => {
                line += 1;
                col = 0;
                i += 2;
            }
            _ => {
                col += 1;
                i += 1;
            }
        }
    }

    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_byte_offset_to_position_unix_newlines() {
        let source = "line1\nline2\nline3\n";
        assert_eq!(byte_offset_to_position(source, 0), (0, 0)); // 'l' in line1
        assert_eq!(byte_offset_to_position(source, 5), (0, 5)); // '\n'
        assert_eq!(byte_offset_to_position(source, 6), (1, 0)); // 'l' in line2
        assert_eq!(byte_offset_to_position(source, 11), (1, 5)); // '\n'
        assert_eq!(byte_offset_to_position(source, 12), (2, 0)); // 'l' in line3
    }

    #[test]
    fn test_byte_offset_to_position_windows_newlines() {
        let source = "line1\r\nline2\r\nline3\r\n";
        assert_eq!(byte_offset_to_position(source, 0), (0, 0));
        assert_eq!(byte_offset_to_position(source, 5), (0, 5));
        assert_eq!(byte_offset_to_position(source, 7), (1, 0)); // skips \r\n
        assert_eq!(byte_offset_to_position(source, 12), (1, 5));
        assert_eq!(byte_offset_to_position(source, 14), (2, 0));
    }

    #[test]
    fn test_byte_offset_to_position_no_newlines() {
        let source = "justoneline";
        assert_eq!(byte_offset_to_position(source, 0), (0, 0));
        assert_eq!(byte_offset_to_position(source, 5), (0, 5));
        assert_eq!(byte_offset_to_position(source, 11), (0, 11));
    }

    #[test]
    fn test_byte_offset_to_position_offset_out_of_bounds() {
        let source = "short\nfile";
        let offset = source.len() + 10;
        assert_eq!(byte_offset_to_position(source, offset), (1, 4));
    }

    #[test]
    fn test_byte_offset_to_position_empty_source() {
        let source = "";
        assert_eq!(byte_offset_to_position(source, 0), (0, 0));
        assert_eq!(byte_offset_to_position(source, 10), (0, 0));
    }
}
