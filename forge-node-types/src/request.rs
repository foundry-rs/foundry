use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JsonRpcRequest {
    jsonrpc: Version,
    id: Id,
    method: String,
    params: Option<Box<RawValue>>,
}

impl JsonRpcRequest {
    pub fn id(&self) -> Id {
        self.id.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum Version {
    #[serde(rename = "2.0")]
    V2,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Id {
    String(String),
    Number(i64),
    Null,
}
