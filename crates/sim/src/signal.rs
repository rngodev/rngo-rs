use crate::spec;
use crate::util::cel::json_to_cel;
use cel::{Context, Program};
use indexmap::IndexMap;
use rusqlite::Connection;
use rusqlite::types::Value as SqlValue;
use serde::Serialize;
use thiserror::Error;

/// `value`/`passed` are `None` together only when `error` is `Some` - a signal with no `expect`
/// still has a `value`, just no `passed` verdict.
#[derive(Clone, Debug, Serialize)]
pub struct SignalOutcome {
    pub value: Option<serde_json::Value>,
    pub passed: Option<bool>,
    pub error: Option<String>,
}

impl SignalOutcome {
    pub(crate) fn error(error: SignalError) -> Self {
        SignalOutcome {
            value: None,
            passed: None,
            error: Some(error.to_string()),
        }
    }
}

#[derive(Debug, Error)]
pub enum SignalError {
    #[error("signal `{key}`: this run log does not support evaluating signals")]
    Unsupported { key: String },
    #[error("signal `{key}`: query failed: {message}")]
    Query { key: String, message: String },
    #[error("signal `{key}`: query returned a blob, which is not a supported result type")]
    UnsupportedValue { key: String },
    #[error("signal `{key}`: expect expression failed to compile: {message}")]
    ExpectCompile { key: String, message: String },
    #[error("signal `{key}`: expect expression failed to evaluate: {message}")]
    ExpectEval { key: String, message: String },
    #[error("signal `{key}`: expect expression must evaluate to a bool, got {value:?}")]
    ExpectNotBool { key: String, value: cel::Value },
}

pub(crate) fn write_outcomes(connection: &Connection, outcomes: &IndexMap<String, SignalOutcome>) {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS signals (
                key TEXT NOT NULL,
                value TEXT,
                result TEXT,
                error TEXT
            )",
        )
        .unwrap();

    for (key, outcome) in outcomes {
        let result = match (&outcome.error, outcome.passed) {
            (Some(_), _) => Some("error"),
            (None, Some(true)) => Some("passed"),
            (None, Some(false)) => Some("failed"),
            (None, None) => None,
        };

        connection
            .prepare_cached(
                "INSERT INTO signals (key, value, result, error) VALUES (?1, ?2, ?3, ?4)",
            )
            .unwrap()
            .execute(rusqlite::params![
                key,
                outcome.value.as_ref().map(|v| v.to_string()),
                result,
                outcome.error,
            ])
            .unwrap();
    }
}

pub(crate) fn evaluate(
    connection: &Connection,
    signals: &IndexMap<String, spec::Signal>,
) -> IndexMap<String, SignalOutcome> {
    signals
        .iter()
        .map(|(key, signal)| (key.clone(), evaluate_one(connection, key, signal)))
        .collect()
}

fn evaluate_one(connection: &Connection, key: &str, signal: &spec::Signal) -> SignalOutcome {
    let spec::Signal::Sql { query, expect } = signal;

    let sql_value = match connection.query_row(query, [], |row| row.get::<_, SqlValue>(0)) {
        Ok(v) => v,
        Err(e) => {
            return SignalOutcome::error(SignalError::Query {
                key: key.to_string(),
                message: e.to_string(),
            });
        }
    };

    let Some(value) = sql_value_to_json(sql_value) else {
        return SignalOutcome::error(SignalError::UnsupportedValue {
            key: key.to_string(),
        });
    };

    let Some(expect) = expect else {
        return SignalOutcome {
            value: Some(value),
            passed: None,
            error: None,
        };
    };

    let program = match Program::compile(expect) {
        Ok(p) => p,
        Err(e) => {
            return SignalOutcome::error(SignalError::ExpectCompile {
                key: key.to_string(),
                message: e.to_string(),
            });
        }
    };

    let mut ctx = Context::default();
    ctx.add_variable_from_value("result", json_to_cel(value.clone()));

    let result = match program.execute(&ctx) {
        Ok(r) => r,
        Err(e) => {
            return SignalOutcome::error(SignalError::ExpectEval {
                key: key.to_string(),
                message: e.to_string(),
            });
        }
    };

    let passed = match result {
        cel::Value::Bool(b) => b,
        other => {
            return SignalOutcome::error(SignalError::ExpectNotBool {
                key: key.to_string(),
                value: other,
            });
        }
    };

    SignalOutcome {
        value: Some(value),
        passed: Some(passed),
        error: None,
    }
}

pub(crate) fn sql_value_to_json(value: SqlValue) -> Option<serde_json::Value> {
    Some(match value {
        SqlValue::Null => serde_json::Value::Null,
        SqlValue::Integer(i) => serde_json::Value::from(i),
        SqlValue::Real(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        SqlValue::Text(s) => serde_json::Value::String(s),
        SqlValue::Blob(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection_with_outputs() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE outputs (status INTEGER NOT NULL);
                 INSERT INTO outputs (status) VALUES (200), (200), (500);",
            )
            .unwrap();
        connection
    }

    fn signal(query: &str, expect: &str) -> IndexMap<String, spec::Signal> {
        let mut map = IndexMap::new();
        map.insert(
            "check".to_string(),
            spec::Signal::Sql {
                query: query.to_string(),
                expect: Some(expect.to_string()),
            },
        );
        map
    }

    fn signal_without_expect(query: &str) -> IndexMap<String, spec::Signal> {
        let mut map = IndexMap::new();
        map.insert(
            "check".to_string(),
            spec::Signal::Sql {
                query: query.to_string(),
                expect: None,
            },
        );
        map
    }

    #[test]
    fn passing_signal() {
        let connection = connection_with_outputs();
        let signals = signal(
            "SELECT COUNT(*) FROM outputs WHERE status = 200",
            "result == 2",
        );

        let outcomes = evaluate(&connection, &signals);
        let outcome = &outcomes["check"];
        assert_eq!(outcome.value, Some(serde_json::json!(2)));
        assert_eq!(outcome.passed, Some(true));
        assert!(outcome.error.is_none());
    }

    #[test]
    fn failing_signal() {
        let connection = connection_with_outputs();
        let signals = signal(
            "SELECT COUNT(*) FROM outputs WHERE status = 500",
            "result == 0",
        );

        let outcomes = evaluate(&connection, &signals);
        let outcome = &outcomes["check"];
        assert_eq!(outcome.value, Some(serde_json::json!(1)));
        assert_eq!(outcome.passed, Some(false));
        assert!(outcome.error.is_none());
    }

    #[test]
    fn missing_expect_has_a_value_but_no_result() {
        let connection = connection_with_outputs();
        let signals = signal_without_expect("SELECT COUNT(*) FROM outputs WHERE status = 500");

        let outcomes = evaluate(&connection, &signals);
        let outcome = &outcomes["check"];
        assert_eq!(outcome.value, Some(serde_json::json!(1)));
        assert_eq!(outcome.passed, None);
        assert!(outcome.error.is_none());
    }

    #[test]
    fn range_expression() {
        let connection = connection_with_outputs();
        let signals = signal("SELECT COUNT(*) FROM outputs", "result >= 2 && result <= 5");

        let outcomes = evaluate(&connection, &signals);
        assert_eq!(outcomes["check"].passed, Some(true));
    }

    #[test]
    fn invalid_query_error_is_captured_in_outcome() {
        let connection = connection_with_outputs();
        let signals = signal("SELECT COUNT(*) FROM missing_table", "result == 0");

        let outcomes = evaluate(&connection, &signals);
        let outcome = &outcomes["check"];
        assert!(outcome.value.is_none());
        assert!(outcome.passed.is_none());
        assert!(outcome.error.as_ref().unwrap().contains("query failed"));
    }

    #[test]
    fn non_bool_expect_error_is_captured_in_outcome() {
        let connection = connection_with_outputs();
        let signals = signal("SELECT COUNT(*) FROM outputs", "result");

        let outcomes = evaluate(&connection, &signals);
        let outcome = &outcomes["check"];
        assert!(outcome.value.is_none());
        assert!(outcome.passed.is_none());
        assert!(
            outcome
                .error
                .as_ref()
                .unwrap()
                .contains("must evaluate to a bool")
        );
    }
}
