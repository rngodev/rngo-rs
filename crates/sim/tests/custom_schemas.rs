mod common;

use common::ParseErrorTestExt;
use rngo_sim::{Dialect, ParseError, Simulation, SimulationEvent};
use std::fmt;

fn build(json: &str) -> Result<Simulation, String> {
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    let simulation_builder = Dialect::primitive()
        .parse_simulation_json(value)
        .map_err(join_errors)?;
    simulation_builder.build().map_err(join_errors)
}

fn join_errors<E: fmt::Display>(errors: Vec<E>) -> String {
    errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_errors(json: &str) -> Vec<ParseError> {
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    Dialect::primitive()
        .parse_simulation_json(value)
        .unwrap_err()
}

#[test]
fn resolves_custom_schema_independently_per_effect() {
    let json = r#"{
        "seed": 1,
        "start": "2024-01-01",
        "end": "2024-01-02",
        "schemas": {
            "title": {
                "schema": {
                    "type": "select",
                    "options": [
                        { "schema": { "type": "constant", "value": "Mr." } },
                        { "schema": { "type": "constant", "value": "Mrs." } },
                        { "schema": { "type": "constant", "value": "Dr." } }
                    ]
                }
            }
        },
        "effects": {
            "a": {
                "trigger": "hz(1, hour)",
                "schema": {
                    "type": "object",
                    "properties": { "title": { "type": "title" } }
                }
            },
            "b": {
                "trigger": "hz(1, hour)",
                "schema": {
                    "type": "object",
                    "properties": { "title": { "type": "title" } }
                }
            }
        }
    }"#;

    let simulation = build(json).unwrap();
    let events: Vec<_> = simulation
        .filter_map(|event| match event {
            SimulationEvent::Input(input) => Some(input),
            SimulationEvent::SkippedInput(_) => None,
        })
        .take(20)
        .collect();

    assert!(events.iter().any(|e| e.effect == "a"));
    assert!(events.iter().any(|e| e.effect == "b"));

    for event in &events {
        let title = event.data["title"].as_str().unwrap();
        assert!(
            ["Mr.", "Mrs.", "Dr."].contains(&title),
            "unexpected title {title:?}"
        );
    }
}

#[test]
fn custom_schema_can_reference_another_custom_schema() {
    let json = r#"{
        "seed": 1,
        "start": "2024-01-01",
        "end": "2024-01-02",
        "schemas": {
            "inner": { "schema": { "type": "constant", "value": "x" } },
            "outer": { "schema": { "type": "inner" } }
        },
        "effects": {
            "a": {
                "trigger": "hz(1, hour)",
                "schema": { "type": "outer" }
            }
        }
    }"#;

    let simulation = build(json).unwrap();
    let events: Vec<_> = simulation
        .filter_map(|event| match event {
            SimulationEvent::Input(input) => Some(input),
            SimulationEvent::SkippedInput(_) => None,
        })
        .take(1)
        .collect();
    assert_eq!(events[0].data, serde_json::json!("x"));
}

#[test]
fn custom_schema_name_cannot_shadow_primitive_type() {
    let json = r#"{
        "schemas": {
            "object": { "schema": { "type": "constant", "value": 1 } }
        },
        "effects": {
            "a": { "schema": { "type": "constant", "value": 1 } }
        }
    }"#;

    let errors = parse_errors(json);

    let error = errors
        .iter()
        .find(|e| e.message().contains("primitive schema type"))
        .unwrap();
    assert_eq!(error.path().unwrap().as_slice(), ["schemas", "object"]);
}

#[test]
fn cyclical_custom_schema_reference_errors() {
    let json = r#"{
        "schemas": {
            "a": { "schema": { "type": "b" } },
            "b": { "schema": { "type": "a" } }
        },
        "effects": {
            "e": { "schema": { "type": "a" } }
        }
    }"#;

    let errors = parse_errors(json);

    let error = errors
        .iter()
        .find(|e| e.message().contains("cyclical"))
        .unwrap();
    assert!(
        error.message().contains("a -> b -> a"),
        "unexpected message: {}",
        error.message()
    );
}

#[test]
fn malformed_custom_schema_errors() {
    let json = r#"{
        "schemas": {
            "age": { "schema": { "type": "number", "minimum": "not-a-number" } }
        },
        "effects": {
            "a": { "schema": { "type": "age" } }
        }
    }"#;

    let errors = parse_errors(json);

    let error = errors
        .iter()
        .find(|e| e.message().contains("minimum must be a number"))
        .unwrap();
    assert_eq!(
        error.path().unwrap().as_slice(),
        ["schemas", "age", "schema", "minimum"]
    );
}

#[test]
fn unknown_custom_schema_type_still_errors_like_before() {
    let json = r#"{
        "effects": {
            "a": { "schema": { "type": "nonexistent" } }
        }
    }"#;

    let errors = parse_errors(json);
    assert!(
        errors
            .iter()
            .any(|e| e.message() == "no schema parser matched")
    );
}
