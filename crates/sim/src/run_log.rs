mod simple;
mod sqlite;

use crate::effect::Input;
use crate::schema::Metadata;
use crate::{Output, spec};
use serde_json::Value;
use std::rc::Rc;

pub use simple::SimpleEventRunLog;
pub use sqlite::SqliteRunLog;

pub trait RunLog: std::fmt::Debug {
    fn push_input(&mut self, input: Input);
    fn push_output(&mut self, output: Output);
    fn push_metadata(&mut self, metadata: EffectMetadata);
    fn get_signal(&self, signal: spec::Signal) -> Option<Value>;
    fn reader(&self) -> Rc<dyn RunLogReader>;
}

pub trait RunLogReader: std::fmt::Debug {
    fn last(&self) -> Option<Rc<Input>>;
    fn index(&self, config: RunLogIndexConfig) -> Box<dyn RunLogIndex>;
}

pub trait RunLogIndex: std::fmt::Debug {
    fn sample(&self) -> Option<Rc<Input>>;
}

#[derive(Clone, Debug)]
pub enum RunLogIndexConfig {
    ByEffect { key: String, cursor: Cursor },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Last,
    Random,
    Unique,
}

#[derive(Clone, Debug)]
pub struct EffectMetadata {
    input_id: Option<i64>,
    effect: String,
    offset: u64,
    metadata: Vec<Metadata>,
}
