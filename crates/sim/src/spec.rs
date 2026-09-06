use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub fn from_value(value: serde_json::Value) -> Result<Spec, Vec<ParseError>> {
    let mut track = serde_path_to_error::Track::new();
    let deserializer = serde_path_to_error::Deserializer::new(value, &mut track);
    serde_path_to_error::deserialize(deserializer).map_err(|e| {
        vec![ParseError::SchemaError {
            path: Some(e.path().to_string().split('.').map(String::from).collect()),
            message: e.inner().to_string(),
        }]
    })
}

#[derive(Error, Debug)]
#[error("failed to parse: `{message}`")]
pub enum ParseError {
    SchemaError {
        path: Option<Vec<String>>,
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Spec {
    pub seed: Option<u64>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub effects: IndexMap<String, Effect>,
    #[serde(default)]
    pub channels: IndexMap<String, Channel>,
    #[serde(default)]
    pub schemas: IndexMap<String, SchemaType>,
    #[serde(default)]
    pub signals: IndexMap<String, Signal>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum Trigger {
    Clock { rate: String },
    Effect { key: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TriggerUnion {
    Shorthand(String),
    Full(Trigger),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Effect {
    pub channel: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub trigger: Option<TriggerUnion>,
    pub metadata: Option<Value>,
    pub schema: Schema,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Format {
    #[serde(rename = "type")]
    pub ftype: Option<String>,
    #[serde(flatten)]
    pub fields: IndexMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Schema {
    #[serde(rename = "type")]
    pub stype: Option<String>,
    #[serde(flatten)]
    pub fields: IndexMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SchemaType {
    pub schema: Schema,
}

/// A `stream` channel with no effects writing to it (no `format`, no `channel: <key>` reference
/// from any effect) is a non-interactive output source: its subprocess runs for the duration of
/// the simulation and its stdout/stderr lines become outputs with no associated effect.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Channel {
    pub format: Option<Format>,
    pub target: ChannelTarget,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelTarget {
    #[serde(rename = "type")]
    pub ttype: Option<String>,
    #[serde(flatten)]
    pub fields: IndexMap<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum Signal {
    Sql {
        query: String,
        #[serde(default)]
        expect: Option<String>,
    },
}
