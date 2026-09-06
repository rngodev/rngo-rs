use super::format::FormatParser;
use super::schema::{SchemaParseVisitor, SchemaParser};
use crate::channel::{self, Channel, ChannelTargetBuilder};
use crate::effect::Effect;
use crate::format::Format;
use crate::parse::ChannelTargetParser;
use crate::schema::custom::CustomParser;
use crate::simulation::{Simulation, SimulationBuilder};
use crate::spec::{self, ParseError, Spec};
use crate::system::{System, SystemBuilder};
use crate::util::time::Moment;
use crate::{format, schema};
use std::collections::HashMap;
use std::rc::Rc;

pub struct Dialect {
    schema_parsers: Rc<Vec<Box<dyn SchemaParser>>>,
    format_parsers: Rc<Vec<Box<dyn FormatParser>>>,
    channel_target_parsers: Rc<Vec<Box<dyn ChannelTargetParser>>>,
}

impl Dialect {
    pub fn new(
        schema_parsers: Vec<Box<dyn SchemaParser>>,
        format_parsers: Vec<Box<dyn FormatParser>>,
        channel_target_parsers: Vec<Box<dyn ChannelTargetParser>>,
    ) -> Self {
        Dialect {
            schema_parsers: Rc::new(schema_parsers),
            format_parsers: Rc::new(format_parsers),
            channel_target_parsers: Rc::new(channel_target_parsers),
        }
    }

    pub fn primitive() -> Self {
        Dialect::new(
            vec![
                Box::new(schema::Array::parser()),
                Box::new(schema::Constant::parser()),
                Box::new(schema::Context::parser()),
                Box::new(schema::Function::parser()),
                Box::new(schema::Number::parser()),
                Box::new(schema::Object::parser()),
                Box::new(schema::Reference::parser()),
                Box::new(schema::Select::parser()),
                Box::new(schema::Str::parser()),
            ],
            vec![Box::new(format::SqlFormat::parser())],
            vec![
                Box::new(channel::Exec::parser()),
                Box::new(channel::Stream::parser()),
            ],
        )
    }

    pub fn parse_simulation_json(
        &self,
        value: serde_json::Value,
    ) -> Result<SimulationBuilder, Vec<ParseError>> {
        let spec: Spec = spec::from_value(value)?;
        self.parse_spec(spec)
    }

    pub fn parse_spec(&self, spec: Spec) -> Result<SimulationBuilder, Vec<ParseError>> {
        let mut errors = vec![];
        let mut simulation_builder = Simulation::builder();
        let simulation_moment_parser = Moment::parser();

        if let Some(seed) = spec.seed {
            simulation_builder.set_seed(seed);
        }

        if let Some(start) = &spec.start {
            match simulation_moment_parser.parse("start", start) {
                Ok(timestamp) => {
                    simulation_builder.set_start(timestamp);
                }
                Err(mut e) => errors.append(&mut e),
            };
        };

        if let Some(end) = &spec.end {
            match simulation_moment_parser.parse("end", end) {
                Ok(timestamp) => {
                    simulation_builder.set_end(timestamp);
                }
                Err(mut e) => errors.append(&mut e),
            };
        };

        for name in spec.schemas.keys() {
            if self.schema_parsers.iter().any(|p| p.key() == name) {
                errors.push(ParseError::SchemaError {
                    path: Some(vec!["schemas".into(), name.clone()]),
                    message: format!(
                        "\"{name}\" is a primitive schema type and cannot be used as a custom schema name"
                    ),
                });
            }
        }

        let custom_schemas: Rc<Vec<CustomParser>> = Rc::new(
            spec.schemas
                .iter()
                .map(|(name, schema_type)| CustomParser::new(name.clone(), schema_type.clone()))
                .collect(),
        );

        for (key, effect) in &spec.effects {
            let mut effect_builder = Effect::builder(key.clone());
            let effect_moment_parser =
                Moment::parser().simulation(&simulation_builder.start, &simulation_builder.end);

            if let Some(start) = &effect.start {
                match effect_moment_parser.parse("start", start) {
                    Ok(timestamp) => {
                        effect_builder.set_start(timestamp);
                    }
                    Err(mut e) => errors.append(&mut e),
                };
            };

            if let Some(end) = &effect.end {
                match effect_moment_parser.parse("end", end) {
                    Ok(timestamp) => {
                        effect_builder.set_end(timestamp);
                    }
                    Err(mut e) => errors.append(&mut e),
                };
            };

            if let Some(trigger_union) = &effect.trigger {
                let trigger = match trigger_union {
                    spec::TriggerUnion::Shorthand(rate) => {
                        spec::Trigger::Clock { rate: rate.clone() }
                    }
                    spec::TriggerUnion::Full(trigger) => trigger.clone(),
                };

                match trigger {
                    spec::Trigger::Clock { rate } => effect_builder.set_trigger_expression(rate),
                    spec::Trigger::Effect { key } => effect_builder.set_trigger_effect(key),
                };
            }

            let visitor = SchemaParseVisitor::new(
                self.schema_parsers.clone(),
                Rc::clone(&custom_schemas),
                effect.schema.clone(),
                vec![],
                vec!["effects".into(), key.clone(), "schema".into()],
            );

            match visitor.parse() {
                Ok(schema_builder) => {
                    effect_builder.set_schema(schema_builder);
                    simulation_builder.set_effect(effect_builder);
                }
                Err(mut e) => errors.append(&mut e),
            }
        }

        if !errors.is_empty() {
            Err(errors)
        } else {
            Ok(simulation_builder)
        }
    }

    pub fn parse_system(&self, spec: Spec) -> Result<SystemBuilder, Vec<ParseError>> {
        let mut errors = vec![];
        let mut simulation_builder = System::builder();

        let effect_channels: HashMap<String, String> = spec
            .effects
            .iter()
            .filter_map(|(k, v)| v.channel.as_ref().map(|s| (k.clone(), s.clone())))
            .collect();

        for (effect_key, channel_key) in &effect_channels {
            if !spec.channels.keys().any(|key| key == channel_key) {
                errors.push(ParseError::SchemaError {
                    path: Some(vec!["effects".into(), effect_key.clone(), "channel".into()]),
                    message: format!("unknown channel: {channel_key}"),
                });
            }
        }

        for (key, channel) in &spec.channels {
            let mut channel_builder = Channel::builder(key.clone());

            match &channel.format {
                Some(format) if format.ftype.is_some() => match self.parse_format(format, &spec) {
                    Ok(format) => {
                        channel_builder.set_format(format);
                    }
                    Err(mut e) => errors.append(&mut e),
                },
                _ => (),
            };

            match self.parse_target(key, &channel.target) {
                Ok(target_builder) => {
                    channel_builder.set_target(target_builder);
                }
                Err(mut e) => errors.append(&mut e),
            }

            let effects = effect_channels
                .iter()
                .filter(|(_, channel_key)| *channel_key == key)
                .map(|(effect_key, _)| effect_key.clone())
                .collect();

            channel_builder.set_effects(effects);

            simulation_builder.set_channel(channel_builder);
        }

        if !errors.is_empty() {
            Err(errors)
        } else {
            Ok(simulation_builder)
        }
    }

    fn parse_format(
        &self,
        format: &spec::Format,
        simulation: &Spec,
    ) -> Result<Box<dyn Format>, Vec<ParseError>> {
        let matching: Vec<_> = self
            .format_parsers
            .iter()
            .filter(|p| format.ftype.as_deref() == Some(p.key()))
            .collect();

        match matching.as_slice() {
            [parser] => parser.parse(format, simulation),
            [] => Err(vec![ParseError::SchemaError {
                path: None,
                message: "unknown format type".to_string(),
            }]),
            _ => Err(vec![ParseError::SchemaError {
                path: None,
                message: format!("{} format parsers matched", matching.len()),
            }]),
        }
    }

    fn parse_target(
        &self,
        channel_key: &str,
        channel_target: &spec::ChannelTarget,
    ) -> Result<Box<dyn ChannelTargetBuilder>, Vec<ParseError>> {
        let matching: Vec<_> = self
            .channel_target_parsers
            .iter()
            .filter(|p| channel_target.ttype.as_deref() == Some(p.key()))
            .collect();

        match matching.as_slice() {
            [parser] => parser.parse(channel_key.into(), channel_target),
            [] => Err(vec![ParseError::SchemaError {
                path: None,
                message: "unknown target type".to_string(),
            }]),
            _ => Err(vec![ParseError::SchemaError {
                path: None,
                message: format!("{} target parsers matched", matching.len()),
            }]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_seed_is_applied_to_the_simulation_builder() {
        let value = serde_json::json!({ "seed": 42, "effects": {} });
        let builder = Dialect::primitive().parse_simulation_json(value).unwrap();
        assert_eq!(builder.seed, 42);
    }

    #[test]
    fn missing_spec_seed_defaults_to_one() {
        let value = serde_json::json!({ "effects": {} });
        let builder = Dialect::primitive().parse_simulation_json(value).unwrap();
        assert_eq!(builder.seed, 1);
    }
}
