use crate::google::cloud::kms::v1::ProtectionLevel;
use bytes::{Buf, BufMut};
use secret_vault_value::SecretValue;

#[derive(Clone, PartialEq, Debug, Default)]
pub struct EncryptRequest {
    /// Required. The resource name of the
    /// \[CryptoKey][google.cloud.kms.v1.CryptoKey\] or
    /// \[CryptoKeyVersion][google.cloud.kms.v1.CryptoKeyVersion\] to use for
    /// encryption.
    ///
    /// If a \[CryptoKey][google.cloud.kms.v1.CryptoKey\] is specified, the server
    /// will use its [primary version]\[google.cloud.kms.v1.CryptoKey.primary\].
    //#[prost(string, tag="1")]
    pub name: ::prost::alloc::string::String,
    /// Required. The data to encrypt. Must be no larger than 64KiB.
    ///
    /// The maximum size depends on the key version's
    /// \[protection_level][google.cloud.kms.v1.CryptoKeyVersionTemplate.protection_level\].
    /// For \[SOFTWARE][google.cloud.kms.v1.ProtectionLevel.SOFTWARE\] keys, the
    /// plaintext must be no larger than 64KiB. For
    /// \[HSM][google.cloud.kms.v1.ProtectionLevel.HSM\] keys, the combined length of
    /// the plaintext and additional_authenticated_data fields must be no larger
    /// than 8KiB.
    //#[prost(bytes="vec", tag="2")]
    pub plaintext: SecretValue,
    /// Optional. Optional data that, if specified, must also be provided during
    /// decryption through
    /// \[DecryptRequest.additional_authenticated_data][google.cloud.kms.v1.DecryptRequest.additional_authenticated_data\].
    ///
    /// The maximum size depends on the key version's
    /// \[protection_level][google.cloud.kms.v1.CryptoKeyVersionTemplate.protection_level\].
    /// For \[SOFTWARE][google.cloud.kms.v1.ProtectionLevel.SOFTWARE\] keys, the AAD
    /// must be no larger than 64KiB. For
    /// \[HSM][google.cloud.kms.v1.ProtectionLevel.HSM\] keys, the combined length of
    /// the plaintext and additional_authenticated_data fields must be no larger
    /// than 8KiB.
    //#[prost(bytes="vec", tag="3")]
    pub additional_authenticated_data: ::prost::alloc::vec::Vec<u8>,
    /// Optional. An optional CRC32C checksum of the
    /// \[EncryptRequest.plaintext][google.cloud.kms.v1.EncryptRequest.plaintext\].
    /// If specified,
    /// \[KeyManagementService][google.cloud.kms.v1.KeyManagementService\] will
    /// verify the integrity of the received
    /// \[EncryptRequest.plaintext][google.cloud.kms.v1.EncryptRequest.plaintext\]
    /// using this checksum.
    /// \[KeyManagementService][google.cloud.kms.v1.KeyManagementService\] will
    /// report an error if the checksum verification fails. If you receive a
    /// checksum error, your client should verify that
    /// CRC32C(\[EncryptRequest.plaintext][google.cloud.kms.v1.EncryptRequest.plaintext\])
    /// is equal to
    /// \[EncryptRequest.plaintext_crc32c][google.cloud.kms.v1.EncryptRequest.plaintext_crc32c\],
    /// and if so, perform a limited number of retries. A persistent mismatch may
    /// indicate an issue in your computation of the CRC32C checksum. Note: This
    /// field is defined as int64 for reasons of compatibility across different
    /// languages. However, it is a non-negative integer, which will never exceed
    /// 2^32-1, and can be safely downconverted to uint32 in languages that support
    /// this type.
    //#[prost(message, optional, tag="7")]
    pub plaintext_crc32c: ::core::option::Option<i64>,
    /// Optional. An optional CRC32C checksum of the
    /// \[EncryptRequest.additional_authenticated_data][google.cloud.kms.v1.EncryptRequest.additional_authenticated_data\].
    /// If specified,
    /// \[KeyManagementService][google.cloud.kms.v1.KeyManagementService\] will
    /// verify the integrity of the received
    /// \[EncryptRequest.additional_authenticated_data][google.cloud.kms.v1.EncryptRequest.additional_authenticated_data\]
    /// using this checksum.
    /// \[KeyManagementService][google.cloud.kms.v1.KeyManagementService\] will
    /// report an error if the checksum verification fails. If you receive a
    /// checksum error, your client should verify that
    /// CRC32C(\[EncryptRequest.additional_authenticated_data][google.cloud.kms.v1.EncryptRequest.additional_authenticated_data\])
    /// is equal to
    /// \[EncryptRequest.additional_authenticated_data_crc32c][google.cloud.kms.v1.EncryptRequest.additional_authenticated_data_crc32c\],
    /// and if so, perform a limited number of retries. A persistent mismatch may
    /// indicate an issue in your computation of the CRC32C checksum. Note: This
    /// field is defined as int64 for reasons of compatibility across different
    /// languages. However, it is a non-negative integer, which will never exceed
    /// 2^32-1, and can be safely downconverted to uint32 in languages that support
    /// this type.
    //#[prost(message, optional, tag="8")]
    pub additional_authenticated_data_crc32c: ::core::option::Option<i64>,
}

impl prost::Message for EncryptRequest {
    #[allow(unused_variables)]
    fn encode_raw(&self, buf: &mut impl BufMut) {
        if self.name != "" {
            ::prost::encoding::string::encode(1u32, &self.name, buf);
        }
        if !self.plaintext.ref_sensitive_value().is_empty() {
            ::prost::encoding::bytes::encode(2u32, self.plaintext.ref_sensitive_value(), buf);
        }
        if self.additional_authenticated_data != b"" as &[u8] {
            ::prost::encoding::bytes::encode(3u32, &self.additional_authenticated_data, buf);
        }
        if let Some(ref msg) = self.plaintext_crc32c {
            ::prost::encoding::message::encode(7u32, msg, buf);
        }
        if let Some(ref msg) = self.additional_authenticated_data_crc32c {
            ::prost::encoding::message::encode(8u32, msg, buf);
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
        const STRUCT_NAME: &str = "EncryptRequest";
        match tag {
            1u32 => {
                let value = &mut self.name;
                ::prost::encoding::string::merge(wire_type, value, buf, ctx).map_err(|mut error| {
                    error.push(STRUCT_NAME, "name");
                    error
                })
            }
            2u32 => ::prost::encoding::bytes::merge(
                wire_type,
                self.plaintext.ref_sensitive_value_mut(),
                buf,
                ctx,
            )
            .map_err(|mut error| {
                error.push(STRUCT_NAME, "plaintext");
                error
            }),
            3u32 => {
                let value = &mut self.additional_authenticated_data;
                ::prost::encoding::bytes::merge(wire_type, value, buf, ctx).map_err(|mut error| {
                    error.push(STRUCT_NAME, "additional_authenticated_data");
                    error
                })
            }
            7u32 => {
                let value = &mut self.plaintext_crc32c;
                ::prost::encoding::message::merge(
                    wire_type,
                    value.get_or_insert_with(::core::default::Default::default),
                    buf,
                    ctx,
                )
                .map_err(|mut error| {
                    error.push(STRUCT_NAME, "plaintext_crc32c");
                    error
                })
            }
            8u32 => {
                let value = &mut self.additional_authenticated_data_crc32c;
                ::prost::encoding::message::merge(
                    wire_type,
                    value.get_or_insert_with(::core::default::Default::default),
                    buf,
                    ctx,
                )
                .map_err(|mut error| {
                    error.push(STRUCT_NAME, "additional_authenticated_data_crc32c");
                    error
                })
            }
            _ => ::prost::encoding::skip_field(wire_type, tag, buf, ctx),
        }
    }
    #[inline]
    fn encoded_len(&self) -> usize {
        0 + if self.name != "" {
            ::prost::encoding::string::encoded_len(1u32, &self.name)
        } else {
            0
        } + if !self.plaintext.ref_sensitive_value().is_empty() {
            ::prost::encoding::bytes::encoded_len(2u32, self.plaintext.ref_sensitive_value())
        } else {
            0
        } + if self.additional_authenticated_data != b"" as &[u8] {
            ::prost::encoding::bytes::encoded_len(3u32, &self.additional_authenticated_data)
        } else {
            0
        } + self
            .plaintext_crc32c
            .as_ref()
            .map_or(0, |msg| ::prost::encoding::message::encoded_len(7u32, msg))
            + self
                .additional_authenticated_data_crc32c
                .as_ref()
                .map_or(0, |msg| ::prost::encoding::message::encoded_len(8u32, msg))
    }
    fn clear(&mut self) {
        self.name.clear();
        self.plaintext.secure_clear();
        self.additional_authenticated_data.clear();
        self.plaintext_crc32c = ::core::option::Option::None;
        self.additional_authenticated_data_crc32c = ::core::option::Option::None;
    }
}

/// Response message for
/// \[KeyManagementService.Decrypt][google.cloud.kms.v1.KeyManagementService.Decrypt\].
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DecryptResponse {
    /// The decrypted data originally supplied in
    /// \[EncryptRequest.plaintext][google.cloud.kms.v1.EncryptRequest.plaintext\].
    //#[prost(bytes="vec", tag="1")]
    pub plaintext: SecretValue,
    /// Integrity verification field. A CRC32C checksum of the returned
    /// \[DecryptResponse.plaintext][google.cloud.kms.v1.DecryptResponse.plaintext\].
    /// An integrity check of
    /// \[DecryptResponse.plaintext][google.cloud.kms.v1.DecryptResponse.plaintext\]
    /// can be performed by computing the CRC32C checksum of
    /// \[DecryptResponse.plaintext][google.cloud.kms.v1.DecryptResponse.plaintext\]
    /// and comparing your results to this field. Discard the response in case of
    /// non-matching checksum values, and perform a limited number of retries. A
    /// persistent mismatch may indicate an issue in your computation of the CRC32C
    /// checksum. Note: receiving this response message indicates that
    /// \[KeyManagementService][google.cloud.kms.v1.KeyManagementService\] is able to
    /// successfully decrypt the
    /// \[ciphertext][google.cloud.kms.v1.DecryptRequest.ciphertext\]. Note: This
    /// field is defined as int64 for reasons of compatibility across different
    /// languages. However, it is a non-negative integer, which will never exceed
    /// 2^32-1, and can be safely downconverted to uint32 in languages that support
    /// this type.
    //#[prost(message, optional, tag="2")]
    pub plaintext_crc32c: Option<i64>,
    /// Whether the Decryption was performed using the primary key version.
    //#[prost(bool, tag="3")]
    pub used_primary: bool,
    /// The \[ProtectionLevel][google.cloud.kms.v1.ProtectionLevel\] of the
    /// \[CryptoKeyVersion][google.cloud.kms.v1.CryptoKeyVersion\] used in
    /// decryption.
    //#[prost(enumeration="ProtectionLevel", tag="4")]
    pub protection_level: i32,
}

impl prost::Message for DecryptResponse {
    #[allow(unused_variables)]
    fn encode_raw(&self, buf: &mut impl BufMut) {
        if !self.plaintext.ref_sensitive_value().is_empty() {
            ::prost::encoding::bytes::encode(1u32, self.plaintext.ref_sensitive_value(), buf);
        }
        if let Some(ref msg) = self.plaintext_crc32c {
            ::prost::encoding::message::encode(2u32, msg, buf);
        }
        if self.used_primary != false {
            ::prost::encoding::bool::encode(3u32, &self.used_primary, buf);
        }
        if self.protection_level != ProtectionLevel::default() as i32 {
            ::prost::encoding::int32::encode(4u32, &self.protection_level, buf);
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
        const STRUCT_NAME: &str = "DecryptResponse";
        match tag {
            1u32 => ::prost::encoding::bytes::merge(
                wire_type,
                self.plaintext.ref_sensitive_value_mut(),
                buf,
                ctx,
            )
            .map_err(|mut error| {
                error.push(STRUCT_NAME, "plaintext");
                error
            }),
            2u32 => {
                let value = &mut self.plaintext_crc32c;
                ::prost::encoding::message::merge(
                    wire_type,
                    value.get_or_insert_with(::core::default::Default::default),
                    buf,
                    ctx,
                )
                .map_err(|mut error| {
                    error.push(STRUCT_NAME, "plaintext_crc32c");
                    error
                })
            }
            3u32 => {
                let value = &mut self.used_primary;
                ::prost::encoding::bool::merge(wire_type, value, buf, ctx).map_err(|mut error| {
                    error.push(STRUCT_NAME, "used_primary");
                    error
                })
            }
            4u32 => {
                let value = &mut self.protection_level;
                ::prost::encoding::int32::merge(wire_type, value, buf, ctx).map_err(|mut error| {
                    error.push(STRUCT_NAME, "protection_level");
                    error
                })
            }
            _ => ::prost::encoding::skip_field(wire_type, tag, buf, ctx),
        }
    }
    #[inline]
    fn encoded_len(&self) -> usize {
        0 + if !self.plaintext.ref_sensitive_value().is_empty() {
            ::prost::encoding::bytes::encoded_len(1u32, self.plaintext.ref_sensitive_value())
        } else {
            0
        } + self
            .plaintext_crc32c
            .as_ref()
            .map_or(0, |msg| ::prost::encoding::message::encoded_len(2u32, msg))
            + if self.used_primary != false {
                ::prost::encoding::bool::encoded_len(3u32, &self.used_primary)
            } else {
                0
            }
            + if self.protection_level != ProtectionLevel::default() as i32 {
                ::prost::encoding::int32::encoded_len(4u32, &self.protection_level)
            } else {
                0
            }
    }
    fn clear(&mut self) {
        self.plaintext.secure_clear();
        self.plaintext_crc32c = ::core::option::Option::None;
        self.used_primary = false;
        self.protection_level = ProtectionLevel::default() as i32;
    }
}
