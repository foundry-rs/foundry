use serde::{Deserialize, Serialize};
use std::fmt;

/// A JSON-RPC request object, a method call
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcMethodCall {
    /// The version of the protocol
    pub jsonrpc: Version,
    /// The name of the method to execute
    pub method: String,
    /// An array or object containing the parameters to be passed to the function.
    #[serde(default = "no_params")]
    pub params: RequestParams,
    /// The identifier for this request issued by the client,
    /// An [Id] must be a String, null or a number.
    /// If missing it's considered a notification in [Version::V2]
    pub id: Id,
}

impl RpcMethodCall {
    pub fn id(&self) -> Id {
        self.id.clone()
    }
}

/// Represents a JSON-RPC request which is considered a notification (missing [Id] optional
/// [Version])
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcNotification {
    pub jsonrpc: Option<Version>,
    pub method: String,
    #[serde(default = "no_params")]
    pub params: RequestParams,
}

/// Representation of a single JSON-RPC call
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcCall {
    /// the RPC method to invoke
    MethodCall(RpcMethodCall),
    /// A notification (no [Id] provided)
    Notification(RpcNotification),
    /// Invalid call
    Invalid {
        /// id or [Id::Null]
        #[serde(default = "null_id")]
        id: Id,
    },
}

/// Represents a JSON-RPC request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum Request {
    /// single json rpc request [RpcCall]
    Single(RpcCall),
    /// batch of several requests
    Batch(Vec<RpcCall>),
}

/// Request parameters
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum RequestParams {
    /// no parameters provided
    None,
    /// An array of JSON values
    Array(Vec<serde_json::Value>),
    /// a map of JSON values
    Object(serde_json::Map<String, serde_json::Value>),
}

impl From<RequestParams> for serde_json::Value {
    fn from(params: RequestParams) -> Self {
        match params {
            RequestParams::None => Self::Null,
            RequestParams::Array(arr) => arr.into(),
            RequestParams::Object(obj) => obj.into(),
        }
    }
}

fn no_params() -> RequestParams {
    RequestParams::None
}

/// Represents the version of the RPC protocol
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Version {
    #[serde(rename = "2.0")]
    V2,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    String(String),
    Number(i64),
    Null,
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(s) => s.fmt(f),
            Self::Number(n) => n.fmt(f),
            Self::Null => f.write_str("null"),
        }
    }
}

fn null_id() -> Id {
    Id::Null
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_serialize_batch() {
        let batch = Request::Batch(vec![
            RpcCall::MethodCall(RpcMethodCall {
                jsonrpc: Version::V2,
                method: "eth_method".to_owned(),
                params: RequestParams::Array(vec![
                    serde_json::Value::from(999),
                    serde_json::Value::from(1337),
                ]),
                id: Id::Number(1),
            }),
            RpcCall::Notification(RpcNotification {
                jsonrpc: Some(Version::V2),
                method: "eth_method".to_owned(),
                params: RequestParams::Array(vec![serde_json::Value::from(999)]),
            }),
        ]);

        let obj = serde_json::to_string(&batch).unwrap();
        assert_eq!(
            obj,
            r#"[{"jsonrpc":"2.0","method":"eth_method","params":[999,1337],"id":1},{"jsonrpc":"2.0","method":"eth_method","params":[999]}]"#
        );
    }

    #[test]
    fn can_deserialize_batch() {
        let s = r#"[{}, {"jsonrpc": "2.0", "method": "eth_call", "params": [1337,420], "id": 1},{"jsonrpc": "2.0", "method": "notify", "params": [999]}]"#;
        let obj: Request = serde_json::from_str(s).unwrap();
        assert_eq!(
            obj,
            Request::Batch(vec![
                RpcCall::Invalid { id: Id::Null },
                RpcCall::MethodCall(RpcMethodCall {
                    jsonrpc: Version::V2,
                    method: "eth_call".to_owned(),
                    params: RequestParams::Array(vec![
                        serde_json::Value::from(1337),
                        serde_json::Value::from(420)
                    ]),
                    id: Id::Number(1)
                }),
                RpcCall::Notification(RpcNotification {
                    jsonrpc: Some(Version::V2),
                    method: "notify".to_owned(),
                    params: RequestParams::Array(vec![serde_json::Value::from(999)])
                })
            ])
        )
    }

    #[test]
    fn can_serialize_method() {
        let m = RpcMethodCall {
            jsonrpc: Version::V2,
            method: "eth_method".to_owned(),
            params: RequestParams::Array(vec![
                serde_json::Value::from(999),
                serde_json::Value::from(1337),
            ]),
            id: Id::Number(1),
        };

        let obj = serde_json::to_string(&m).unwrap();
        assert_eq!(obj, r#"{"jsonrpc":"2.0","method":"eth_method","params":[999,1337],"id":1}"#);
    }

    #[test]
    fn can_serialize_call_notification() {
        let n = RpcCall::Notification(RpcNotification {
            jsonrpc: Some(Version::V2),
            method: "eth_method".to_owned(),
            params: RequestParams::Array(vec![serde_json::Value::from(999)]),
        });
        let obj = serde_json::to_string(&n).unwrap();
        assert_eq!(obj, r#"{"jsonrpc":"2.0","method":"eth_method","params":[999]}"#);
    }

    #[test]
    fn can_serialize_notification() {
        let n = RpcNotification {
            jsonrpc: Some(Version::V2),
            method: "eth_method".to_owned(),
            params: RequestParams::Array(vec![
                serde_json::Value::from(999),
                serde_json::Value::from(1337),
            ]),
        };
        let obj = serde_json::to_string(&n).unwrap();
        assert_eq!(obj, r#"{"jsonrpc":"2.0","method":"eth_method","params":[999,1337]}"#);
    }

    #[test]
    fn can_deserialize_notification() {
        let s = r#"{"jsonrpc": "2.0", "method": "eth_method", "params": [999,1337]}"#;
        let obj: RpcNotification = serde_json::from_str(s).unwrap();

        assert_eq!(
            obj,
            RpcNotification {
                jsonrpc: Some(Version::V2),
                method: "eth_method".to_owned(),
                params: RequestParams::Array(vec![
                    serde_json::Value::from(999),
                    serde_json::Value::from(1337)
                ])
            }
        );
        let s = r#"{"jsonrpc": "2.0", "method": "foobar"}"#;
        let obj: RpcNotification = serde_json::from_str(s).unwrap();
        assert_eq!(
            obj,
            RpcNotification {
                jsonrpc: Some(Version::V2),
                method: "foobar".to_owned(),
                params: RequestParams::None,
            }
        );
        let s = r#"{"jsonrpc": "2.0", "method": "eth_method", "params": [999,1337], "id": 1}"#;
        let obj: Result<RpcNotification, _> = serde_json::from_str(s);
        assert!(obj.is_err());
    }

    #[test]
    fn can_deserialize_call() {
        let s = r#"{"jsonrpc": "2.0", "method": "eth_method", "params": [999]}"#;
        let obj: RpcCall = serde_json::from_str(s).unwrap();
        assert_eq!(
            obj,
            RpcCall::Notification(RpcNotification {
                jsonrpc: Some(Version::V2),
                method: "eth_method".to_owned(),
                params: RequestParams::Array(vec![serde_json::Value::from(999)])
            })
        );

        let s = r#"{"jsonrpc": "2.0", "method": "eth_method", "params": [999], "id": 1}"#;
        let obj: RpcCall = serde_json::from_str(s).unwrap();
        assert_eq!(
            obj,
            RpcCall::MethodCall(RpcMethodCall {
                jsonrpc: Version::V2,
                method: "eth_method".to_owned(),
                params: RequestParams::Array(vec![serde_json::Value::from(999)]),
                id: Id::Number(1)
            })
        );

        let s = r#"{"jsonrpc": "2.0", "method": "eth_method", "params": [], "id": 1}"#;
        let obj: RpcCall = serde_json::from_str(s).unwrap();
        assert_eq!(
            obj,
            RpcCall::MethodCall(RpcMethodCall {
                jsonrpc: Version::V2,
                method: "eth_method".to_owned(),
                params: RequestParams::Array(vec![]),
                id: Id::Number(1)
            })
        );

        let s = r#"{"jsonrpc": "2.0", "method": "eth_method", "params": null, "id": 1}"#;
        let obj: RpcCall = serde_json::from_str(s).unwrap();
        assert_eq!(
            obj,
            RpcCall::MethodCall(RpcMethodCall {
                jsonrpc: Version::V2,
                method: "eth_method".to_owned(),
                params: RequestParams::None,
                id: Id::Number(1)
            })
        );

        let s = r#"{"jsonrpc": "2.0", "method": "eth_method", "id": 1}"#;
        let obj: RpcCall = serde_json::from_str(s).unwrap();
        assert_eq!(
            obj,
            RpcCall::MethodCall(RpcMethodCall {
                jsonrpc: Version::V2,
                method: "eth_method".to_owned(),
                params: RequestParams::None,
                id: Id::Number(1)
            })
        );
    }
}
