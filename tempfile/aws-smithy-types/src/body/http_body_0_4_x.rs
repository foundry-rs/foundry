/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;

use crate::body::{Error, SdkBody};

impl SdkBody {
    /// Construct an `SdkBody` from a type that implements [`http_body_0_4::Body<Data = Bytes>`](http_body_0_4::Body).
    ///
    /// _Note: This is only available with `http-body-0-4-x` enabled._
    pub fn from_body_0_4<T, E>(body: T) -> Self
    where
        T: http_body_0_4::Body<Data = Bytes, Error = E> + Send + Sync + 'static,
        E: Into<Error> + 'static,
    {
        SdkBody::from_body_0_4_internal(body)
    }
}

#[cfg(feature = "hyper-0-14-x")]
impl From<hyper_0_14::Body> for SdkBody {
    fn from(body: hyper_0_14::Body) -> Self {
        SdkBody::from_body_0_4(body)
    }
}

impl http_body_0_4::Body for SdkBody {
    type Data = Bytes;
    type Error = Error;

    fn poll_data(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        self.poll_next(cx)
    }

    fn poll_trailers(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<http::HeaderMap<http::HeaderValue>>, Self::Error>> {
        self.poll_next_trailers(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.is_end_stream()
    }

    fn size_hint(&self) -> http_body_0_4::SizeHint {
        let mut result = http_body_0_4::SizeHint::default();
        let (lower, upper) = self.bounds_on_remaining_length();
        result.set_lower(lower);
        if let Some(u) = upper {
            result.set_upper(u)
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use crate::body::SdkBody;

    #[test]
    fn map_preserve_preserves_bytes_hint() {
        let initial = SdkBody::from("hello!");
        assert_eq!(initial.bytes(), Some(b"hello!".as_slice()));

        let new_body = initial.map_preserve_contents(SdkBody::from_body_0_4);
        assert_eq!(new_body.bytes(), Some(b"hello!".as_slice()));
    }

    #[test]
    fn sdkbody_debug_dyn() {
        let hyper_body = hyper_0_14::Body::channel().1;
        let body = SdkBody::from_body_0_4(hyper_body);
        assert!(format!("{:?}", body).contains("BoxBody"));
    }
}
