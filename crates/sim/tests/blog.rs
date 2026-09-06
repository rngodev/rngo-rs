mod common;

use rngo_sim::build::*;
use rngo_sim::{Dialect, RunLog, SimpleEventRunLog, Simulation, SimulationEvent};
use serde_json::Value;

/// `Simulation` only reads from a run log now (for `reference()` lookups); writing to it is the
/// caller's job, same as `cli/src/run.rs`. "post" references "user", so without pushing each
/// yielded input back into `run_log` as it's produced, `reference().effect("user")` would never
/// see any prior data and every "post" attempt would be skipped.
fn assert_simulation(mut run_log: SimpleEventRunLog, simulation: Simulation) {
    // A "post" fired before any "user" exists yet has nothing for its `reference` to resolve, so
    // it's skipped rather than emitted - filter down to real inputs first, then take 60 of those,
    // so an incidental early skip can't leave fewer than 60 to assert against.
    let events: Vec<_> = simulation
        .filter_map(|event| match event {
            SimulationEvent::Input(input) => {
                run_log.push_input(input.clone());
                Some(input)
            }
            SimulationEvent::SkippedInput(_) => None,
        })
        .take(60)
        .collect();

    let user_events: Vec<_> = events
        .iter()
        .filter(|e| e.effect == "user")
        .map(|e| &e.data)
        .collect();

    let post_events: Vec<_> = events
        .iter()
        .filter(|e| e.effect == "post")
        .map(|e| &e.data)
        .collect();

    assert!(!user_events.is_empty(), "expected user events");
    assert!(!post_events.is_empty(), "expected post events");

    let mut age_null_count = 0;

    for value in &user_events {
        let obj = value.as_object().unwrap();

        let id = obj["id"].as_i64().unwrap();
        assert!(id >= 1, "user id should be >= 1, got {id}");

        let name = obj["name"].as_str().unwrap();
        let name_len = name.chars().count();
        assert!(
            (10..=50).contains(&name_len),
            "user name length should be 10–50, got {name_len}"
        );

        match obj["age"] {
            Value::Null => age_null_count += 1,
            Value::Number(ref n) => {
                let age = n.as_f64().unwrap();
                assert!(
                    (18.0..=65.0).contains(&age),
                    "user age should be 18–65, got {age}"
                )
            }
            _ => panic!("expected null or number"),
        }

        let created_at = obj["created_at"].as_i64().unwrap();
        assert!(
            created_at >= 0,
            "user created_at should be >= 0, got {created_at}"
        );
    }

    assert!(age_null_count > 0);

    for value in post_events {
        let obj = value.as_object().unwrap();

        let id = obj["id"].as_i64().unwrap();
        assert!(id >= 1, "post id should be >= 1, got {id}");

        let user_id = obj["user_id"].as_i64().unwrap();
        let user = user_events
            .iter()
            .map(|u| u.as_object().unwrap())
            .find(|u| u["id"].as_i64().unwrap() == user_id)
            .expect("user_id to reference a user");

        let title = obj["title"].as_str().unwrap();
        assert!(
            title.starts_with("Post: "),
            "title should start with 'Post: ', got {title:?}"
        );

        let suffix_len = title.chars().count() - "Post: ".len();
        assert!(
            (10..=20).contains(&suffix_len),
            "title suffix length should be 10–20, got {suffix_len}"
        );

        let tags = obj["tags"].as_array().unwrap();
        assert!(tags.len() <= 10);
        tags.iter().for_each(|t| {
            let tag = t.as_str().unwrap();
            assert!(tag == "a" || tag == "b")
        });

        let created_at = obj["created_at"].as_i64().unwrap();
        assert!(
            created_at >= 0,
            "post created_at should be >= 0, got {created_at}"
        );
        assert!(
            user["created_at"].as_i64().unwrap() <= created_at,
            "post created_at should happen after the references user"
        )
    }
}

#[test]
fn builder() {
    let mut simulation_builder = Simulation::builder();
    let run_log = SimpleEventRunLog::new(simulation_builder.seed);

    simulation_builder
        .with_effect("user", |e| {
            e.schema(
                object()
                    .property("id", number().minimum(1).scale(0).step(1))
                    .property("name", string().pattern(".{10,50}"))
                    .property(
                        "age",
                        select()
                            .option(3, number().minimum(18).maximum(65))
                            .option(1, constant().value(Value::Null)),
                    )
                    .property("created_at", context().path(["sim", "offset"])),
            )
        })
        .with_effect("post", |e| {
            e.schema(
                object()
                    .property("id", number().minimum(1).scale(0).step(1))
                    .property(
                        "user_id",
                        function()
                            .expression("user.id")
                            .variable("user", reference().effect("user")),
                    )
                    .property("title", string().pattern("Post: .{10,20}"))
                    .property(
                        "tags",
                        array().min_items(0).max_items(10).items(
                            select()
                                .option(1, constant().value("a"))
                                .option(1, constant().value("b")),
                        ),
                    )
                    .property("created_at", context().path(["sim", "offset"])),
            )
        });

    let simulation = simulation_builder
        .run_log_reader(run_log.reader())
        .build()
        .unwrap();
    assert_simulation(run_log, simulation);
}

#[test]
fn spec() {
    let json = r#"{
        "effects": {
            "user": {
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 },
                        "name": { "type": "string", "pattern": ".{10,50}" },
                        "age": {
                            "type": "select",
                            "options": [
                                { "weight": 3, "schema": { "type": "number", "minimum": 18, "maximum": 65 } },
                                { "weight": 1, "schema": { "type": "constant", "value": null } }
                            ]
                        },
                        "created_at": { "type": "context", "path": ["sim", "offset"] }
                    }
                }
            },
            "post": {
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 },
                        "user_id": {
                            "type": "function",
                            "expression": "user.id",
                            "variables": {
                                "user": { "type": "reference", "effect": "user" }
                            }
                        },
                        "title": { "type": "string", "pattern": "Post: .{10,20}" },
                        "tags": {
                            "type": "array",
                            "minItems": 0,
                            "maxItems": 10,
                            "items": {
                                "type": "select",
                                "options": [
                                    { "weight": 1, "schema": { "type": "constant", "value": "a" } },
                                    { "weight": 1, "schema": { "type": "constant", "value": "b" } }
                                ]
                            }
                        },
                        "created_at": { "type": "context", "path": ["sim", "offset"] }
                    }
                }
            }
        }
    }"#;

    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    let simulation_builder = Dialect::primitive().parse_simulation_json(value).unwrap();
    let run_log = SimpleEventRunLog::new(simulation_builder.seed);
    let simulation = simulation_builder
        .run_log_reader(run_log.reader())
        .build()
        .unwrap();
    assert_simulation(run_log, simulation);
}
