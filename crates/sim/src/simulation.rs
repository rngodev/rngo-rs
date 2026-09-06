use crate::RunLogReader;
use crate::build::{BuildError, SimulationKey};
use crate::effect::{Effect, EffectBuilder, Input, SkippedInput};
use crate::util::time::Moment;
use chrono::{TimeDelta, Utc};
use serde::Serialize;
use std::rc::Rc;

#[derive(Debug)]
pub struct Simulation {
    effects: Vec<Effect>,
    limit: Option<u64>,
    emitted: u64,
}

impl Simulation {
    pub fn builder() -> SimulationBuilder {
        SimulationBuilder::new()
    }
}

#[derive(Debug, Serialize)]
pub enum SimulationEvent {
    Input(Input),
    SkippedInput(SkippedInput),
}

impl Iterator for Simulation {
    type Item = SimulationEvent;

    fn next(&mut self) -> Option<Self::Item> {
        if self.limit.is_some_and(|limit| self.emitted >= limit) {
            return None;
        }

        self.effects
            .sort_unstable_by_key(|e| e.next_offset().unwrap_or(u64::MAX));

        match self.effects.first_mut()?.next()? {
            Ok(input) => {
                self.emitted += 1;
                Some(SimulationEvent::Input(input))
            }
            Err(skipped_input) => {
                self.emitted += 1;
                if self.limit.is_some_and(|limit| self.emitted >= limit) {
                    return None;
                }
                Some(SimulationEvent::SkippedInput(skipped_input))
            }
        }
    }
}

#[derive(Debug)]
pub struct SimulationBuilder {
    pub seed: u64,
    pub start: Moment,
    pub end: Moment,
    run_log_reader: Option<Rc<dyn RunLogReader>>,
    effect_builders: Vec<EffectBuilder>,
    limit: Option<u64>,
}

impl SimulationBuilder {
    fn new() -> Self {
        SimulationBuilder {
            seed: 1,
            start: Moment::Relative(TimeDelta::days(-30)),
            end: Moment::Relative(TimeDelta::zero()),
            run_log_reader: None,
            effect_builders: vec![],
            limit: None,
        }
    }

    pub fn run_log_reader(mut self, run_log_reader: Rc<dyn RunLogReader>) -> Self {
        self.run_log_reader = Some(run_log_reader);
        self
    }

    /// Caps the total number of events (effects and errors combined) the built [`Simulation`]
    /// will emit before its iterator ends.
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.set_seed(seed);
        self
    }

    pub fn set_seed(&mut self, seed: u64) -> &mut Self {
        self.seed = seed;
        self
    }

    pub fn start(mut self, start: Moment) -> Self {
        self.set_start(start);
        self
    }

    pub fn set_start(&mut self, start: Moment) -> &mut Self {
        self.start = start;
        self
    }

    pub fn end(mut self, end: Moment) -> Self {
        self.set_end(end);
        self
    }

    pub fn set_end(&mut self, end: Moment) -> &mut Self {
        self.end = end;
        self
    }

    pub fn set_effect(&mut self, effect: EffectBuilder) {
        self.effect_builders.push(effect)
    }

    pub fn with_effect(
        &mut self,
        key: &str,
        f: impl FnOnce(EffectBuilder) -> EffectBuilder,
    ) -> &mut Self {
        let builder = Effect::builder(key.into());
        let builder = f(builder);
        self.effect_builders.push(builder);
        self
    }

    pub fn build(self) -> Result<Simulation, Vec<BuildError>> {
        let mut errors = vec![];
        let now = Utc::now().fixed_offset();
        let start = self.start.resolve(now);
        let end = self.end.resolve(now);

        if start >= end {
            errors.push(BuildError::Simulation {
                key: SimulationKey::Start,
                message: "start must be before end".into(),
            });
        }

        let run_log_reader = match self.run_log_reader {
            Some(run_log_reader) => run_log_reader,
            None => {
                errors.push(BuildError::Simulation {
                    key: SimulationKey::RunLog,
                    message: "start must be before end".into(),
                });

                return Err(errors);
            }
        };

        let mut effects = vec![];

        for mut effect_builder in self.effect_builders {
            effect_builder
                .set_now(now)
                .set_sim_start(start)
                .set_sim_end(end)
                .set_event_run_log(run_log_reader.clone())
                .set_seed(self.seed);

            match effect_builder.build() {
                Ok(effect) => effects.push(effect),
                Err(mut e) => errors.append(&mut e),
            }
        }

        if errors.is_empty() {
            Ok(Simulation {
                effects,
                limit: self.limit,
                emitted: 0,
            })
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::build::BuildError;
    use crate::schema::{
        Metadata, Schema, SchemaBuildVisitor, SchemaBuilder, SchemaContext, SchemaResult,
    };

    /// A schema that deterministically alternates between succeeding and failing on
    /// every other call, so a test can know exactly how many `Ok`s and `Err`s a fixed
    /// number of calls produces without depending on any effect's trigger timing.
    #[derive(Debug, Default)]
    struct AlternatingSchema {
        calls: u32,
    }

    impl Schema for AlternatingSchema {
        fn next(&mut self, _context: &SchemaContext) -> SchemaResult {
            self.calls += 1;
            if self.calls % 2 == 1 {
                SchemaResult {
                    value: Some(serde_json::Value::Null),
                    metadata: vec![],
                }
            } else {
                SchemaResult {
                    value: None,
                    metadata: vec![Metadata {
                        mtype: "error".into(),
                        attribute: None,
                        data: Some(serde_json::json!({ "message": "boom" })),
                    }],
                }
            }
        }
    }

    #[derive(Debug)]
    struct AlternatingSchemaBuilder;

    impl SchemaBuilder for AlternatingSchemaBuilder {
        fn build(&self, _visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>> {
            Ok(Box::new(AlternatingSchema::default()))
        }
    }

    #[test]
    fn limit_counts_effects_and_errors_together() {
        let mut simulation_builder = super::Simulation::builder();

        simulation_builder.with_effect("alternating", |e| {
            e.trigger_hertz(1000.0).schema(AlternatingSchemaBuilder)
        });

        let events: Vec<_> = simulation_builder.limit(5).build().unwrap().collect();

        // 5 emitted total (limit), alternating Ok, Err, Ok, Err, Ok - so 3 are yielded.
        assert_eq!(
            events.len(),
            3,
            "limit should count both effect and error events toward the cap"
        );
    }
}
