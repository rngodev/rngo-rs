use crate::util::cel::json_to_cel;
use crate::{RunLog, spec};
use cel::{Context, Program};
use indexmap::IndexMap;
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
    #[error("signal `{key}`: expect expression failed to compile: {message}")]
    ExpectCompile { key: String, message: String },
    #[error("signal `{key}`: expect expression failed to evaluate: {message}")]
    ExpectEval { key: String, message: String },
    #[error("signal `{key}`: expect expression must evaluate to a bool, got {value:?}")]
    ExpectNotBool { key: String, value: cel::Value },
}

/// Evaluates every signal against `run_log`: fetches each signal's raw value via
/// [`RunLog::get_signal`] (backend-specific - e.g. a real SQL query for `SqliteRunLog`, always
/// `None` for `SimpleEventRunLog`), then checks `expect` against it via [`evaluate_one`], which is
/// backend-agnostic. Records each outcome back onto `run_log` via `RunLog::push_signal_outcome`.
pub fn evaluate(
    run_log: &mut dyn RunLog,
    signals: &IndexMap<String, spec::Signal>,
) -> IndexMap<String, SignalOutcome> {
    signals
        .iter()
        .map(|(key, signal)| {
            let value = run_log.get_signal(signal.clone());
            let outcome = evaluate_one(key, signal, value);
            run_log.push_signal_outcome(key, &outcome);
            (key.clone(), outcome)
        })
        .collect()
}

/// Checks one signal's `expect` expression against `value` - the raw result of
/// [`RunLog::get_signal`], or `None` if this run log couldn't produce one (either it doesn't
/// support evaluating signals at all, or the query itself failed).
pub fn evaluate_one(
    key: &str,
    signal: &spec::Signal,
    value: Option<serde_json::Value>,
) -> SignalOutcome {
    let spec::Signal::Sql { expect, .. } = signal;

    let Some(value) = value else {
        return SignalOutcome::error(SignalError::Unsupported {
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

    fn sql_signal(query: &str, expect: &str) -> spec::Signal {
        spec::Signal::Sql {
            query: query.to_string(),
            expect: Some(expect.to_string()),
        }
    }

    fn sql_signal_without_expect(query: &str) -> spec::Signal {
        spec::Signal::Sql {
            query: query.to_string(),
            expect: None,
        }
    }

    #[test]
    fn passing_signal() {
        let signal = sql_signal("unused", "result == 2");
        let outcome = evaluate_one("check", &signal, Some(serde_json::json!(2)));
        assert_eq!(outcome.value, Some(serde_json::json!(2)));
        assert_eq!(outcome.passed, Some(true));
        assert!(outcome.error.is_none());
    }

    #[test]
    fn failing_signal() {
        let signal = sql_signal("unused", "result == 0");
        let outcome = evaluate_one("check", &signal, Some(serde_json::json!(1)));
        assert_eq!(outcome.value, Some(serde_json::json!(1)));
        assert_eq!(outcome.passed, Some(false));
        assert!(outcome.error.is_none());
    }

    #[test]
    fn missing_expect_has_a_value_but_no_result() {
        let signal = sql_signal_without_expect("unused");
        let outcome = evaluate_one("check", &signal, Some(serde_json::json!(1)));
        assert_eq!(outcome.value, Some(serde_json::json!(1)));
        assert_eq!(outcome.passed, None);
        assert!(outcome.error.is_none());
    }

    #[test]
    fn range_expression() {
        let signal = sql_signal("unused", "result >= 2 && result <= 5");
        let outcome = evaluate_one("check", &signal, Some(serde_json::json!(3)));
        assert_eq!(outcome.passed, Some(true));
    }

    #[test]
    fn missing_value_is_reported_as_unsupported() {
        let signal = sql_signal("unused", "result == 0");
        let outcome = evaluate_one("check", &signal, None);
        assert!(outcome.value.is_none());
        assert!(outcome.passed.is_none());
        assert!(
            outcome
                .error
                .as_ref()
                .unwrap()
                .contains("does not support evaluating signals")
        );
    }

    #[test]
    fn non_bool_expect_error_is_captured_in_outcome() {
        let signal = sql_signal("unused", "result");
        let outcome = evaluate_one("check", &signal, Some(serde_json::json!(3)));
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
