use crate::channel::ChannelBuilder;
use crate::{BuildError, Channel, Input, Output};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};

pub struct System {
    pub channels: HashMap<String, Channel>,
    pub effect_channels: HashMap<String, String>,
    output_rx: Receiver<Output>,
}

impl System {
    pub fn builder() -> SystemBuilder {
        SystemBuilder::new()
    }

    /// Sends `input` to whichever channel its effect is assigned to, if any, returning any
    /// outputs the channel target produced synchronously in direct response (e.g. an `exec`
    /// target's captured stdout/stderr). Outputs a target produces on its own schedule (e.g. a
    /// `stream` target's subprocess writing to stdout at some later point) are not returned here
    /// - drain them from `System` itself, which implements `Iterator<Item = Output>`.
    pub fn send(&mut self, input: &Input) -> Result<Vec<Output>, Box<dyn std::error::Error>> {
        let channel_key = match self.effect_channels.get(&input.effect) {
            Some(k) => k.clone(),
            None => return Ok(vec![]),
        };

        match self.channels.get_mut(&channel_key) {
            Some(channel) => {
                let formatted_data = if let Some(format) = &channel.format {
                    format.format(input).ok()
                } else {
                    None
                };

                channel.target.send(input, formatted_data)
            }
            None => Ok(vec![]),
        }
    }

    /// Shuts down every channel's target (e.g. closing a `stream` subprocess's stdin and
    /// waiting for it to exit). This can itself produce trailing outputs, so `System` remains
    /// iterable afterward - drain it before dropping to pick those up.
    pub fn finish(&mut self) {
        self.channels.clear();
    }
}

/// Yields outputs a channel target has produced on its own schedule (e.g. a `stream` target's
/// subprocess writing to stdout), as opposed to the outputs `send` returns directly.
impl Iterator for System {
    type Item = Output;

    fn next(&mut self) -> Option<Self::Item> {
        self.output_rx.try_recv().ok()
    }
}

pub struct SystemBuilder {
    channel_builders: Vec<ChannelBuilder>,
}

impl SystemBuilder {
    pub fn new() -> Self {
        Self {
            channel_builders: vec![],
        }
    }

    pub fn set_channel(&mut self, channel: ChannelBuilder) {
        self.channel_builders.push(channel)
    }

    pub fn with_channel(
        &mut self,
        key: &str,
        f: impl FnOnce(ChannelBuilder) -> ChannelBuilder,
    ) -> &mut Self {
        let builder = Channel::builder(key.into());
        let builder = f(builder);
        self.channel_builders.push(builder);
        self
    }

    pub fn build(self) -> Result<System, Vec<BuildError>> {
        let mut errors = vec![];
        let mut channels = HashMap::new();
        let (output_tx, output_rx) = mpsc::channel::<Output>();

        for mut channel_builder in self.channel_builders {
            channel_builder.set_output_tx(output_tx.clone());

            match channel_builder.build() {
                Ok(channel) => {
                    channels.insert(channel.key.clone(), channel);
                }
                Err(mut e) => errors.append(&mut e),
            };
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        let effect_channels = channels
            .values()
            .flat_map(|channel| {
                channel
                    .effects
                    .iter()
                    .map(move |effect_key| (effect_key.clone(), channel.key.clone()))
            })
            .collect();

        Ok(System {
            channels,
            effect_channels,
            output_rx,
        })
    }
}

impl Default for SystemBuilder {
    fn default() -> Self {
        Self::new()
    }
}
