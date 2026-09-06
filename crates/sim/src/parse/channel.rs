use crate::channel::ChannelTargetBuilder;
use crate::{ParseError, spec};

pub trait ChannelTargetParser {
    fn key(&self) -> &str;
    fn parse(
        &self,
        channel_key: String,
        channel_target: &spec::ChannelTarget,
    ) -> Result<Box<dyn ChannelTargetBuilder>, Vec<ParseError>>;
}
