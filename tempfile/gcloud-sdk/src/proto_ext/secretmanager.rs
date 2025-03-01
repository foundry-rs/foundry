use bytes::{Buf, BufMut};
use secret_vault_value::SecretValue;

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SecretPayload {
    pub data: SecretValue,

    pub data_crc32c: Option<i64>,
}

impl prost::Message for SecretPayload {
    #[allow(unused_variables)]
    fn encode_raw(&self, buf: &mut impl BufMut) {
        if !self.data.ref_sensitive_value().is_empty() {
            ::prost::encoding::bytes::encode(1u32, self.data.ref_sensitive_value(), buf);
        }
        if let ::core::option::Option::Some(ref value) = self.data_crc32c {
            ::prost::encoding::int64::encode(2u32, value, buf);
        }
    }
    #[allow(unused_variables)]
    fn merge_field(
        &mut self,
        tag: u32,
        wire_type: ::prost::encoding::WireType,
        buf: &mut impl Buf,
        ctx: ::prost::encoding::DecodeContext,
    ) -> ::core::result::Result<(), ::prost::DecodeError> {
        const STRUCT_NAME: &str = "SecretPayload";
        match tag {
            1u32 => ::prost::encoding::bytes::merge(
                wire_type,
                self.data.ref_sensitive_value_mut(),
                buf,
                ctx,
            )
            .map_err(|mut error| {
                error.push(STRUCT_NAME, "data");
                error
            }),
            2u32 => {
                let value = &mut self.data_crc32c;
                ::prost::encoding::int64::merge(
                    wire_type,
                    value.get_or_insert_with(::core::default::Default::default),
                    buf,
                    ctx,
                )
                .map_err(|mut error| {
                    error.push(STRUCT_NAME, "data_crc32c");
                    error
                })
            }
            _ => ::prost::encoding::skip_field(wire_type, tag, buf, ctx),
        }
    }
    #[inline]
    fn encoded_len(&self) -> usize {
        0 + if !self.data.ref_sensitive_value().is_empty() {
            ::prost::encoding::bytes::encoded_len(1u32, self.data.ref_sensitive_value())
        } else {
            0
        } + self.data_crc32c.as_ref().map_or(0, |value| {
            ::prost::encoding::int64::encoded_len(2u32, value)
        })
    }
    fn clear(&mut self) {
        self.data.secure_clear();
        self.data_crc32c = ::core::option::Option::None;
    }
}
