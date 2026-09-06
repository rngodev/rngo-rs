mod common;

use common::BuildErrorTestExt;
use rngo_sim::build::*;
use rngo_sim::{BuildError, Dialect, EffectKey, Simulation, SimulationEvent};
use serde_json::Value;

fn effect_offsets(sim: Simulation, take: usize) -> Vec<u64> {
    sim.filter_map(|e| match e {
        SimulationEvent::Input(input) => Some(input.offset),
        SimulationEvent::SkippedInput(_) => None,
    })
    .take(take)
    .collect()
}

/// The default simulation window is 30 days (start = -30d, end = now).
/// Offsets are cumulative seconds from the start, so all events should
/// have offset <= 30 * 86_400 = 2_592_000.
///
/// With the default clock rate of 1 event/day, taking 60 events drives
/// the simulation ~60 days into the future — well past the end.
#[test]
fn simulation_respects_end_time() {
    let mut builder = Simulation::builder();
    builder.with_effect("events", |e| e.schema(constant().value(Value::Null)));

    let offsets = effect_offsets(builder.build().unwrap(), 60);
    let window_secs: u64 = 30 * 86_400;

    let out_of_bounds: Vec<_> = offsets
        .iter()
        .copied()
        .filter(|&o| o > window_secs)
        .collect();
    assert!(
        out_of_bounds.is_empty(),
        "{} events past end time (>{window_secs}s), e.g. {}",
        out_of_bounds.len(),
        out_of_bounds[0],
    );
}

/// An effect with its own start offset should not emit events before that offset,
/// even though the simulation window begins earlier.
#[test]
fn effect_respects_start_time() {
    use chrono::TimeDelta;
    use rngo_sim::Moment;

    let mut builder = Simulation::builder();
    // Simulation: -30d to now. Effect starts at -15d (halfway through).
    builder.with_effect("events", |e| {
        e.start(Moment::Relative(TimeDelta::days(-15)))
            .schema(constant().value(Value::Null))
    });

    let offsets = effect_offsets(builder.build().unwrap(), 60);

    // Effect start is 15 days into the 30-day window = 15 * 86_400 seconds.
    let effect_start_offset: u64 = 15 * 86_400;

    let too_early: Vec<_> = offsets
        .iter()
        .copied()
        .filter(|&o| o < effect_start_offset)
        .collect();
    assert!(
        too_early.is_empty(),
        "{} events before effect start (<{effect_start_offset}s), e.g. {}",
        too_early.len(),
        too_early[0],
    );
}

/// An effect with its own end time (parsed from spec) should emit events before
/// that end time and nothing after. Goes through Dialect::parse_simulation_json
/// so a copy-paste bug that calls set_start instead of set_end in parse.rs would
/// cause events to appear in the wrong half of the window and fail this test.
#[test]
fn effect_respects_end_time_via_spec() {
    let spec = serde_json::json!({
        "seed": 1,
        "start": "2024-01-01",
        "end": "2024-12-31",
        "effects": {
            "events": {
                "trigger": "hz(1, day)",
                "end": "2024-06-01",
                "schema": { "type": "constant", "value": null }
            }
        }
    });

    let sim = Dialect::primitive()
        .parse_simulation_json(spec)
        .unwrap()
        .build()
        .unwrap();

    let offsets: Vec<u64> = sim
        .filter_map(|e| match e {
            SimulationEvent::Input(input) => Some(input.offset),
            SimulationEvent::SkippedInput(_) => None,
        })
        .collect();

    // 2024-01-01 to 2024-06-01 = 31+29+31+30+31 = 152 days (2024 is a leap year)
    let effect_end_offset: u64 = 152 * 86_400;

    assert!(
        !offsets.is_empty(),
        "effect should produce events before its end time"
    );

    let too_late: Vec<_> = offsets
        .iter()
        .copied()
        .filter(|&o| o > effect_end_offset)
        .collect();
    assert!(
        too_late.is_empty(),
        "{} events after effect end (>{effect_end_offset}s), e.g. {}",
        too_late.len(),
        too_late[0],
    );
}

/// An effect with both start and end inside the simulation window should only
/// emit events between its own start and end, not outside them.
#[test]
fn effect_respects_both_start_and_end() {
    let spec = serde_json::json!({
        "seed": 1,
        "start": "2024-01-01",
        "end": "2024-12-31",
        "effects": {
            "events": {
                "trigger": "hz(1, day)",
                "start": "2024-04-01",
                "end": "2024-09-30",
                "schema": { "type": "constant", "value": null }
            }
        }
    });

    let sim = Dialect::primitive()
        .parse_simulation_json(spec)
        .unwrap()
        .build()
        .unwrap();

    let offsets: Vec<u64> = sim
        .filter_map(|e| match e {
            SimulationEvent::Input(input) => Some(input.offset),
            SimulationEvent::SkippedInput(_) => None,
        })
        .collect();

    // 2024 is a leap year.
    // 2024-04-01 is day 92 (31+29+31+1), so offset = 91 days from Jan 1.
    // 2024-09-30 is day 274 (31+29+31+30+31+30+31+31+30), so offset = 273 days.
    let effect_start_offset: u64 = 91 * 86_400;
    let effect_end_offset: u64 = 273 * 86_400;

    assert!(
        !offsets.is_empty(),
        "effect should produce events within its window"
    );

    let too_early: Vec<_> = offsets
        .iter()
        .copied()
        .filter(|&o| o < effect_start_offset)
        .collect();
    assert!(
        too_early.is_empty(),
        "{} events before effect start (<{effect_start_offset}s), e.g. {}",
        too_early.len(),
        too_early[0],
    );

    let too_late: Vec<_> = offsets
        .iter()
        .copied()
        .filter(|&o| o > effect_end_offset)
        .collect();
    assert!(
        too_late.is_empty(),
        "{} events after effect end (>{effect_end_offset}s), e.g. {}",
        too_late.len(),
        too_late[0],
    );
}

#[test]
fn effect_start_before_simulation_start_is_error() {
    use chrono::TimeDelta;
    use rngo_sim::Moment;

    let mut builder = Simulation::builder();
    // Simulation: -30d to now. Effect tries to start before the simulation at -60d.
    builder.with_effect("events", |e| {
        e.start(Moment::Relative(TimeDelta::days(-60)))
            .schema(constant().value(Value::Null))
    });

    let errors = builder.build().unwrap_err();
    let error = errors
        .iter()
        .find(|e| matches!(e, BuildError::Effect { effect, .. } if effect == "events"))
        .unwrap();

    assert_eq!(error.message(), "start cannot be before simulation start");
    assert!(matches!(
        error,
        BuildError::Effect {
            key: EffectKey::Start,
            ..
        }
    ));
}

#[test]
fn effect_end_after_simulation_end_is_error() {
    use chrono::TimeDelta;
    use rngo_sim::Moment;

    let mut builder = Simulation::builder();
    // Simulation: -30d to now. Effect tries to end after the simulation at +1d.
    builder.with_effect("events", |e| {
        e.end(Moment::Relative(TimeDelta::days(1)))
            .schema(constant().value(Value::Null))
    });

    let errors = builder.build().unwrap_err();
    let error = errors
        .iter()
        .find(|e| matches!(e, BuildError::Effect { effect, .. } if effect == "events"))
        .unwrap();

    assert_eq!(error.message(), "end cannot be after simulation end");
    assert!(matches!(
        error,
        BuildError::Effect {
            key: EffectKey::End,
            ..
        }
    ));
}

#[test]
fn effect_bounds_outside_simulation_via_spec_are_errors() {
    let spec = serde_json::json!({
        "start": "2024-01-01",
        "end": "2024-12-31",
        "effects": {
            "too_early": {
                "start": "2023-06-01",
                "schema": { "type": "constant", "value": null }
            },
            "too_late": {
                "end": "2025-06-01",
                "schema": { "type": "constant", "value": null }
            }
        }
    });

    let errors = Dialect::primitive()
        .parse_simulation_json(spec)
        .unwrap()
        .build()
        .unwrap_err();

    let start_error = errors
        .iter()
        .find(|e| matches!(e, BuildError::Effect { effect, key: EffectKey::Start, .. } if effect == "too_early"))
        .unwrap();
    assert_eq!(
        start_error.message(),
        "start cannot be before simulation start"
    );

    let end_error = errors
        .iter()
        .find(|e| matches!(e, BuildError::Effect { effect, key: EffectKey::End, .. } if effect == "too_late"))
        .unwrap();
    assert_eq!(end_error.message(), "end cannot be after simulation end");
}
