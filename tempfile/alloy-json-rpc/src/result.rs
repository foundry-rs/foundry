use crate::{Response, ResponsePayload, RpcError, RpcRecv};
use serde_json::value::RawValue;
use std::borrow::Borrow;

/// The result of a JSON-RPC request.
///
/// Either a success response, an error response, or a non-response error. The
/// non-response error is intended to be used for errors returned by a
/// transport, or serde errors.
///
/// The common cases are:
/// - `Ok(T)` - The server returned a successful response.
/// - `Err(RpcError::ErrorResponse(ErrResp))` - The server returned an error response.
/// - `Err(RpcError::SerError(E))` - A serialization error occurred.
/// - `Err(RpcError::DeserError { err: E, text: String })` - A deserialization error occurred.
/// - `Err(RpcError::TransportError(E))` - Some client-side or communication error occurred.
pub type RpcResult<T, E, ErrResp = Box<RawValue>> = Result<T, RpcError<E, ErrResp>>;

/// A partially deserialized [`RpcResult`], borrowing from the deserializer.
pub type BorrowedRpcResult<'a, E> = RpcResult<&'a RawValue, E, &'a RawValue>;

/// Transform a transport response into an [`RpcResult`], discarding the [`Id`].
///
/// [`Id`]: crate::Id
pub fn transform_response<T, E, ErrResp>(response: Response<T, ErrResp>) -> RpcResult<T, E, ErrResp>
where
    ErrResp: RpcRecv,
{
    match response {
        Response { payload: ResponsePayload::Failure(err_resp), .. } => {
            Err(RpcError::err_resp(err_resp))
        }
        Response { payload: ResponsePayload::Success(result), .. } => Ok(result),
    }
}

/// Transform a transport outcome into an [`RpcResult`], discarding the [`Id`].
///
/// [`Id`]: crate::Id
pub fn transform_result<T, E, ErrResp>(
    response: Result<Response<T, ErrResp>, E>,
) -> Result<T, RpcError<E, ErrResp>>
where
    ErrResp: RpcRecv,
{
    match response {
        Ok(resp) => transform_response(resp),
        Err(e) => Err(RpcError::Transport(e)),
    }
}

/// Attempt to deserialize the `Ok(_)` variant of an [`RpcResult`].
pub fn try_deserialize_ok<J, T, E, ErrResp>(
    result: RpcResult<J, E, ErrResp>,
) -> RpcResult<T, E, ErrResp>
where
    J: Borrow<RawValue>,
    T: RpcRecv,
    ErrResp: RpcRecv,
{
    let json = result?;
    let json = json.borrow().get();
    trace!(ty=%std::any::type_name::<T>(), %json, "deserializing response");
    serde_json::from_str(json)
        .inspect(|response| trace!(?response, "deserialized response"))
        .inspect_err(|err| trace!(?err, "failed to deserialize response"))
        .map_err(|err| RpcError::deser_err(err, json))
}
