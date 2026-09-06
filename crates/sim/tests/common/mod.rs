#![allow(dead_code)]

use rngo_sim::{BuildError, ParseError, SchemaEdge};

pub trait BuildErrorTestExt {
    fn message(&self) -> &str;
    fn schema_path(&self) -> Option<&Vec<SchemaEdge>>;
}

impl BuildErrorTestExt for BuildError {
    fn message(&self) -> &str {
        match self {
            BuildError::Simulation { message, .. } => message,
            BuildError::Effect { message, .. } => message,
            BuildError::Schema { message, .. } => message,
            BuildError::System { message } => message,
            BuildError::Channel { message, .. } => message,
            BuildError::ChannelTarget { message, .. } => message,
        }
    }

    fn schema_path(&self) -> Option<&Vec<SchemaEdge>> {
        match self {
            BuildError::Schema { path, .. } => Some(path),
            _ => None,
        }
    }
}

pub trait ParseErrorTestExt {
    fn message(&self) -> &str;
    fn path(&self) -> Option<&Vec<String>>;
}

impl ParseErrorTestExt for ParseError {
    fn message(&self) -> &str {
        let ParseError::SchemaError { message, .. } = self;
        message
    }

    fn path(&self) -> Option<&Vec<String>> {
        let ParseError::SchemaError { path, .. } = self;
        path.as_ref()
    }
}
