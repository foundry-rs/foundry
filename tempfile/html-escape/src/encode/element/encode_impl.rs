macro_rules! encode_impl {
    ($l:expr; $(#[$attr: meta])* $parse_macro:ident; $(#[$encode_attr: meta])* $encode_name: ident; $(#[$encode_to_string_attr: meta])* $encode_to_string_name: ident; $(#[$encode_to_vec_attr: meta])* $encode_to_vec_name: ident; $(#[$encode_to_writer_attr: meta])* $encode_to_writer_name: ident $(;)*) => {
        $(#[$encode_attr])*
        ///
        $(#[$attr])*
        #[inline]
        pub fn $encode_name<S: ?Sized + AsRef<str>>(text: &S) -> Cow<str> {
            let text = text.as_ref();
            let text_bytes = text.as_bytes();
            let text_length = text_bytes.len();

            let mut p = 0;
            let mut e;

            let mut step = 0;

            let (mut v, mut start) = loop {
                if p == text_length {
                    return Cow::from(text);
                }

                e = text_bytes[p];

                $parse_macro!(
                    e,
                    step,
                    {
                        let mut v = Vec::with_capacity(text_length + 1);

                        v.extend_from_slice(&text_bytes[..(p - $l)]);

                        break (v, p - $l);
                    },
                    {
                        let mut v = Vec::with_capacity(text_length + 1);

                        v.extend_from_slice(&text_bytes[..p]);

                        break (v, p);
                    },
                    {
                        let mut v = Vec::with_capacity(text_length + 1);

                        v.extend_from_slice(&text_bytes[..(p - 2)]);

                        break (v, p - 2);
                    }
                );

                p += 1;
            };

            v.push(b'\\');

            p += 1;

            for e in text_bytes[p..].iter().copied() {
                $parse_macro!(
                    e,
                    step,
                    {
                        v.extend_from_slice(&text_bytes[start..(p - $l)]);
                        start = p - $l;
                        v.push(b'\\');
                    },
                    {
                        v.extend_from_slice(&text_bytes[start..p]);
                        start = p;
                        v.push(b'\\');
                    },
                    {
                        v.extend_from_slice(&text_bytes[start..(p - 2)]);
                        start = p - 2;
                        v.push(b'\\');
                    }
                );

                p += 1;
            }

            v.extend_from_slice(&text_bytes[start..p]);

            Cow::from(unsafe { String::from_utf8_unchecked(v) })
        }

        $(#[$encode_to_string_attr])*
        ///
        $(#[$attr])*
        #[inline]
        pub fn $encode_to_string_name<S: AsRef<str>>(text: S, output: &mut String) -> &str {
            unsafe { from_utf8_unchecked($encode_to_vec_name(text, output.as_mut_vec())) }
        }

        $(#[$encode_to_vec_attr])*
        ///
        $(#[$attr])*
        #[inline]
        pub fn $encode_to_vec_name<S: AsRef<str>>(text: S, output: &mut Vec<u8>) -> &[u8] {
            let text = text.as_ref();
            let text_bytes = text.as_bytes();
            let text_length = text_bytes.len();

            output.reserve(text_length);

            let current_length = output.len();

            let mut start = 0;
            let mut end = 0;

            let mut step = 0;

            for e in text_bytes.iter().copied() {
                $parse_macro!(
                    e,
                    step,
                    {
                        output.extend_from_slice(&text_bytes[start..(end - $l)]);
                        start = end - $l;
                        output.push(b'\\');
                    },
                    {
                        output.extend_from_slice(&text_bytes[start..end]);
                        start = end;
                        output.push(b'\\');
                    },
                    {
                        output.extend_from_slice(&text_bytes[start..(end - 2)]);
                        start = end - 2;
                        output.push(b'\\');
                    }
                );

                end += 1;
            }

            output.extend_from_slice(&text_bytes[start..end]);

            &output[current_length..]
        }

        #[cfg(feature = "std")]
        $(#[$encode_to_writer_attr])*
        ///
        $(#[$attr])*
        #[inline]
        pub fn $encode_to_writer_name<S: AsRef<str>, W: Write>(text: S, output: &mut W) -> Result<(), io::Error> {
            let text = text.as_ref();
            let text_bytes = text.as_bytes();

            let mut start = 0;
            let mut end = 0;

            let mut step = 0;

            for e in text_bytes.iter().copied() {
                $parse_macro!(
                    e,
                    step,
                    {
                        output.write_all(&text_bytes[start..(end - $l)])?;
                        start = end - $l;
                        output.write_all(b"\\")?;
                    },
                    {
                        output.write_all(&text_bytes[start..end])?;
                        start = end;
                        output.write_all(b"\\")?;
                    },
                    {
                        output.write_all(&text_bytes[start..(end - 2)])?;
                        start = end - 2;
                        output.write_all(b"\\")?;
                    }
                );

                end += 1;
            }

            output.write_all(&text_bytes[start..end])
        }
    };
}
