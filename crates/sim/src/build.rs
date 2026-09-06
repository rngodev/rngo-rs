pub use crate::schema::array::ArrayBuilder;
pub use crate::schema::constant::ConstantBuilder;
pub use crate::schema::context::ContextBuilder;
pub use crate::schema::function::FunctionBuilder;
pub use crate::schema::number::NumberBuilder;
pub use crate::schema::object::ObjectBuilder;
pub use crate::schema::reference::ReferenceBuilder;
pub use crate::schema::select::SelectBuilder;
pub use crate::schema::string::StrBuilder;
use crate::schema::{Array, Constant, Context, Function, Number, Object, Reference, Select, Str};
use thiserror::Error;

pub fn array() -> ArrayBuilder {
    Array::builder()
}

pub fn constant() -> ConstantBuilder {
    Constant::builder()
}

pub fn context() -> ContextBuilder {
    Context::builder()
}

pub fn function() -> FunctionBuilder {
    Function::builder()
}

pub fn number() -> NumberBuilder {
    Number::builder()
}

pub fn object() -> ObjectBuilder {
    Object::builder()
}

pub fn reference() -> ReferenceBuilder {
    Reference::builder()
}

pub fn select() -> SelectBuilder {
    Select::builder()
}

pub fn string() -> StrBuilder {
    Str::builder()
}

#[derive(Error, Debug)]
#[error("failed to build: `{message}`")]
pub enum BuildError {
    Simulation {
        key: SimulationKey,
        message: String,
    },
    Effect {
        effect: String,
        key: EffectKey,
        message: String,
    },
    Schema {
        effect: String,
        path: Vec<SchemaEdge>,
        message: String,
    },
    System {
        message: String,
    },
    Channel {
        channel: String,
        message: String,
    },
    ChannelTarget {
        channel: String,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct SchemaEdge {
    pub kind: &'static str,
    pub key: String,
}

#[derive(Debug)]
pub enum SimulationKey {
    Start,
    End,
}

#[derive(Debug)]
pub enum EffectKey {
    Schema,
    Trigger,
    Config,
    Start,
    End,
}
