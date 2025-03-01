macro_rules! decode_impl {
    ($l:expr; $(#[$attr: meta])* $parse_macro:ident; $(#[$decode_attr: meta])* $decode_name: ident; $(#[$decode_to_string_attr: meta])* $decode_to_string_name: ident; $(#[$decode_to_vec_attr: meta])* $decode_to_vec_name: ident; $(#[$decode_to_writer_attr: meta])* $decode_to_writer_name: ident $(;)*) => {
        $(#[$decode_attr])*
        ///
        $(#[$attr])*
        #[inline]
        pub fn $decode_name<S: ?Sized + AsRef<str>>(text: &S) -> Cow<str> {
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

                $parse_macro!(e, step,
                    {
                        let mut v = Vec::with_capacity(text_length);

                        v.extend_from_slice(&text_bytes[..(p - ($l + 1))]);

                        break (v, p - $l);
                    },
                    {
                        let mut v = Vec::with_capacity(text_length);

                        v.extend_from_slice(&text_bytes[..(p - 1)]);

                        break (v, p);
                    },
                    {
                        let mut v = Vec::with_capacity(text_length);

                        v.extend_from_slice(&text_bytes[..(p - 3)]);

                        break (v, p - 2);
                    }
                );

                p += 1;
            };

            p += 1;

            for e in text_bytes[p..].iter().copied() {
                $parse_macro!(e, step,
                    {
                        v.extend_from_slice(&text_bytes[start..(p - ($l + 1))]);
                        start = p - $l;
                    },
                    {
                        v.extend_from_slice(&text_bytes[start..(p - 1)]);
                        start = p;
                    },
                    {
                        v.extend_from_slice(&text_bytes[start..(p - 3)]);
                        start = p - 2;
                    }
                );

                p += 1;
            }

            v.extend_from_slice(&text_bytes[start..p]);

            Cow::from(unsafe { String::from_utf8_unchecked(v) })
        }

        $(#[$decode_to_string_attr])*
        ///
        $(#[$attr])*
        #[inline]
        pub fn $decode_to_string_name<S: AsRef<str>>(text: S, output: &mut String) -> &str {
            unsafe { from_utf8_unchecked($decode_to_vec_name(text, output.as_mut_vec())) }
        }

        $(#[$decode_to_vec_attr])*
        ///
        $(#[$attr])*
        #[inline]
        pub fn $decode_to_vec_name<S: AsRef<str>>(text: S, output: &mut Vec<u8>) -> &[u8] {
            let text = text.as_ref();
            let text_bytes = text.as_bytes();
            let text_length = text_bytes.len();

            output.reserve(text_length);

            let current_length = output.len();

            let mut start = 0;
            let mut end = 0;

            let mut step = 0;

            for e in text_bytes.iter().copied() {
                $parse_macro!(e, step,
                    {
                        output.extend_from_slice(&text_bytes[start..(end - ($l + 1))]);
                        start = end - $l;
                    },
                    {
                        output.extend_from_slice(&text_bytes[start..(end - 1)]);
                        start = end;
                    },
                    {
                        output.extend_from_slice(&text_bytes[start..(end - 3)]);
                        start = end - 2;
                    }
                );

                end += 1;
            }

            output.extend_from_slice(&text_bytes[start..end]);

            &output[current_length..]
        }

        #[cfg(feature = "std")]
        $(#[$decode_to_writer_attr])*
        ///
        $(#[$attr])*
        #[inline]
        pub fn $decode_to_writer_name<S: AsRef<str>, W: Write>(text: S, output: &mut W) -> Result<(), io::Error> {
            let text = text.as_ref();
            let text_bytes = text.as_bytes();

            let mut start = 0;
            let mut end = 0;

            let mut step = 0;

            for e in text_bytes.iter().copied() {
                $parse_macro!(e, step,
                    {
                        output.write_all(&text_bytes[start..(end - ($l + 1))])?;
                        start = end - $l;
                    },
                    {
                        output.write_all(&text_bytes[start..(end - 1)])?;
                        start = end;
                    },
                    {
                        output.write_all(&text_bytes[start..(end - 3)])?;
                        start = end - 2;
                    }
                );

                end += 1;
            }

            output.write_all(&text_bytes[start..end])
        }
    };
}
