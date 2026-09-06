mod exec;
mod stream;

use crate::format::Format;
use crate::{BuildError, Input, Output};
use std::error::Error;
use std::sync::mpsc::Sender;

pub use exec::Exec;
pub use stream::Stream;

#[derive(Debug)]
pub struct Channel {
    pub key: String,
    pub format: Option<Box<dyn Format>>,
    pub target: Box<dyn ChannelTarget>,
    pub effects: Vec<String>,
}

impl Channel {
    pub fn builder(key: String) -> ChannelBuilder {
        ChannelBuilder::new(key)
    }
}

pub trait ChannelTarget: std::fmt::Debug {
    fn send(&mut self, input: &Input, data: Option<String>) -> Result<Vec<Output>, Box<dyn Error>>;
}

pub trait ChannelTargetBuilder {
    fn build(&self, output_tx: Sender<Output>) -> Result<Box<dyn ChannelTarget>, Vec<BuildError>>;
}

pub struct ChannelBuilder {
    key: String,
    format: Option<Box<dyn Format>>,
    channel_target_builder: Option<Box<dyn ChannelTargetBuilder>>,
    output_tx: Option<Sender<Output>>,
    effects: Vec<String>,
}

impl ChannelBuilder {
    fn new(key: String) -> Self {
        ChannelBuilder {
            key,
            format: None,
            channel_target_builder: None,
            output_tx: None,
            effects: vec![],
        }
    }

    pub fn format(mut self, format: impl Format + 'static) -> Self {
        self.set_format(Box::new(format));
        self
    }

    pub fn set_format(&mut self, format: Box<dyn Format>) -> &mut Self {
        self.format = Some(format);
        self
    }

    pub fn target(mut self, builder: impl ChannelTargetBuilder + 'static) -> Self {
        self.set_target(Box::new(builder));
        self
    }

    pub fn set_target(&mut self, builder: Box<dyn ChannelTargetBuilder>) -> &mut Self {
        self.channel_target_builder = Some(builder);
        self
    }

    pub fn output_tx(mut self, output_tx: Sender<Output>) -> Self {
        self.set_output_tx(output_tx);
        self
    }

    pub fn set_output_tx(&mut self, output_tx: Sender<Output>) -> &mut Self {
        self.output_tx = Some(output_tx);
        self
    }

    pub fn effects(mut self, effects: Vec<String>) -> Self {
        self.set_effects(effects);
        self
    }

    pub fn set_effects(&mut self, effects: Vec<String>) -> &mut Self {
        self.effects = effects;
        self
    }

    pub fn build(self) -> Result<Channel, Vec<BuildError>> {
        let target = if let Some(target_builder) = self.channel_target_builder {
            if let Some(output_tx) = self.output_tx {
                target_builder.build(output_tx)
            } else {
                Err(vec![BuildError::Channel {
                    channel: self.key.clone(),
                    message: "output_tx was not set".into(),
                }])
            }
        } else {
            Err(vec![BuildError::Channel {
                channel: self.key.clone(),
                message: "target was not set".into(),
            }])
        }?;

        Ok(Channel {
            key: self.key,
            format: self.format,
            target,
            effects: self.effects,
        })
    }
}
