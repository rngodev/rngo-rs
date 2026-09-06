use crate::channel::{ChannelTarget, ChannelTargetBuilder};
use crate::parse::ChannelTargetParser;
use crate::{BuildError, Input, Level, Output, ParseError, spec};
use chrono::Utc;
use handlebars::Handlebars;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

#[derive(Debug)]
pub struct Exec {
    channel_key: String,
    hbs: Handlebars<'static>,
}

impl Exec {
    pub fn parser() -> ExecParser {
        ExecParser {}
    }
}

impl ChannelTarget for Exec {
    fn send(
        &mut self,
        input: &Input,
        _data: Option<String>,
    ) -> Result<Vec<Output>, Box<dyn std::error::Error>> {
        let command = self.hbs.render("command", &input.data)?;

        let output = Command::new("sh")
            .arg("-c")
            .arg(&command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;

        let timestamp = Utc::now();
        let mut outputs = vec![];

        for (bytes, level) in [
            (&output.stdout, Level::Info),
            (&output.stderr, Level::Error),
        ] {
            for line in BufReader::new(bytes.as_slice())
                .lines()
                .map_while(Result::ok)
            {
                if !line.is_empty() {
                    outputs.push(Output {
                        input_id: Some(input.id),
                        channel: self.channel_key.clone(),
                        level,
                        data: line,
                        timestamp,
                    });
                }
            }
        }

        if !output.status.success() {
            outputs.push(Output {
                input_id: Some(input.id),
                channel: self.channel_key.clone(),
                level: Level::Error,
                data: format!("command exited with {}", output.status),
                timestamp,
            });
        }

        Ok(outputs)
    }
}

pub struct ExecBuilder {
    channel_key: String,
    command: Option<String>,
}

impl ExecBuilder {
    pub fn new(channel_key: String) -> Self {
        ExecBuilder {
            channel_key,
            command: None,
        }
    }

    pub fn command(mut self, value: String) -> Self {
        self.set_command(value);
        self
    }

    pub fn set_command(&mut self, value: String) -> &mut Self {
        self.command = Some(value);
        self
    }
}

impl ChannelTargetBuilder for ExecBuilder {
    fn build(&self, _output_tx: Sender<Output>) -> Result<Box<dyn ChannelTarget>, Vec<BuildError>> {
        let Some(command) = self.command.clone() else {
            return Err(vec![BuildError::ChannelTarget {
                channel: self.channel_key.clone(),
                message: "command not specified".into(),
            }]);
        };

        let mut hbs = Handlebars::new();
        hbs.register_template_string("command", &command)
            .map_err(|_e| {
                vec![BuildError::ChannelTarget {
                    channel: self.channel_key.clone(),
                    message: "FIX ME must be a string".into(),
                }]
            })?;

        Ok(Box::new(Exec {
            channel_key: self.channel_key.clone(),
            hbs,
        }))
    }
}

pub struct ExecParser {}

impl ChannelTargetParser for ExecParser {
    fn key(&self) -> &str {
        "exec"
    }

    fn parse(
        &self,
        channel_key: String,
        channel_target: &spec::ChannelTarget,
    ) -> Result<Box<dyn ChannelTargetBuilder>, Vec<ParseError>> {
        let command = match channel_target.fields.get("command") {
            Some(value) => match value.as_str() {
                Some(command) => command,
                None => {
                    return Err(vec![ParseError::SchemaError {
                        path: None,
                        message: "FIX ME must be a string".into(),
                    }]);
                }
            },
            None => {
                return Err(vec![ParseError::SchemaError {
                    path: None,
                    message: "FIX ME must be a string".into(),
                }]);
            }
        };

        Ok(Box::new(
            ExecBuilder::new(channel_key).command(command.into()),
        ))
    }
}
