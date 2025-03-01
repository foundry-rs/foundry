//! Macro for defining a new hash type.

/// Instantiate a new marked digest. Wraps the output of some type that implemented `digest::Digest`
macro_rules! marked_digest {
    (
        $(#[$outer:meta])*
        $marked_name:ident, $digest:ty
    ) => {
        $(#[$outer])*
        #[derive(Copy, Clone, Debug, Eq, PartialEq, Default, Hash, PartialOrd, Ord)]
        pub struct $marked_name($crate::hashes::DigestOutput<$digest>);

        impl $marked_name {
            /// Unwrap the marked digest, returning the underlying `GenericArray`
            pub fn to_internal(self) -> $crate::hashes::DigestOutput<$digest> {
                self.0
            }
        }

        impl $crate::hashes::MarkedDigestOutput for $marked_name {
            fn size(&self) -> usize {
                self.0.len()
            }
        }

        impl<T> From<T> for $marked_name
        where
            T: Into<$crate::hashes::DigestOutput<$digest>>
        {
            fn from(t: T) -> Self {
                $marked_name(t.into())
            }
        }

        impl $crate::hashes::MarkedDigest<$marked_name> for $digest {
            fn finalize_marked(self) -> $marked_name {
                $marked_name($crate::hashes::Digest::finalize(self))
            }

            fn digest_marked(data: &[u8]) -> $marked_name {
                $marked_name(<$digest as $crate::hashes::Digest>::digest(data))
            }
        }

        impl AsRef<$crate::hashes::DigestOutput<$digest>> for $marked_name {
            fn as_ref(&self) -> &$crate::hashes::DigestOutput<$digest> {
                &self.0
            }
        }

        impl AsMut<$crate::hashes::DigestOutput<$digest>> for $marked_name {
            fn as_mut(&mut self) -> &mut $crate::hashes::DigestOutput<$digest> {
                &mut self.0
            }
        }

        impl AsRef<[u8]> for $marked_name {
            fn as_ref(&self) -> &[u8] {
                self.0.as_ref()
            }
        }

        impl AsMut<[u8]> for $marked_name {
            fn as_mut(&mut self) -> &mut [u8] {
                self.0.as_mut()
            }
        }

        impl $crate::ser::ByteFormat for $marked_name {
            type Error = $crate::ser::SerError;

            fn serialized_length(&self) -> usize {
                $crate::hashes::MarkedDigestOutput::size(self)
            }

            fn read_from<R>(reader: &mut R) -> $crate::ser::SerResult<Self>
            where
                R: std::io::Read,
                Self: std::marker::Sized,
            {
                let mut buf = Self::default();
                reader.read_exact(buf.as_mut())?;
                Ok(buf)
            }

            fn write_to<W>(&self, writer: &mut W) -> $crate::ser::SerResult<usize>
            where
                W: std::io::Write,
            {
                Ok(writer.write(self.as_ref())?)
            }
        }
    };
}
