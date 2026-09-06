use serde_json::Value;
use std::collections::HashMap;

use crate::effect::Input;
use crate::format::Format;
use crate::parse::FormatParser;
use crate::{ParseError, spec};

#[derive(Debug)]
pub struct SqlFormat {
    effect_tables: HashMap<String, String>,
}

impl SqlFormat {
    pub fn parser() -> SqlFormatParser {
        SqlFormatParser {}
    }
}

impl Format for SqlFormat {
    fn format(&self, event: &Input) -> Result<String, String> {
        let table = self
            .effect_tables
            .get(&event.effect)
            .map(String::as_str)
            .unwrap_or(&event.effect);

        Ok(match &event.data {
            Value::Null => {
                format!("INSERT INTO {table} VALUES (null);")
            }
            Value::Bool(b) => {
                format!("INSERT INTO {table} VALUES ({b:?});")
            }
            Value::Number(n) => {
                format!("INSERT INTO {table} VALUES ({n});")
            }
            Value::String(s) => {
                format!("INSERT INTO {table} VALUES ({s});")
            }
            Value::Array(a) => {
                let json_array = serde_json::to_string(a).unwrap();
                format!("INSERT INTO {table} VALUES ('{json_array}');")
            }
            Value::Object(map) => {
                let columns = map
                    .keys()
                    .map(|k| format!("\"{k}\""))
                    .collect::<Vec<_>>()
                    .join(", ");

                let values = map
                    .values()
                    .map(|v| match v {
                        Value::Null => "null".to_string(),
                        Value::String(s) => format!("'{}'", s.replace('\'', "''")),
                        Value::Array(a) => {
                            let json_array = serde_json::to_string(a).unwrap();
                            format!("'{}'", json_array.replace('\'', "''"))
                        }
                        Value::Object(o) => {
                            let json_object = serde_json::to_string(o).unwrap();
                            format!("'{}'", json_object.replace('\'', "''"))
                        }
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                format!("INSERT INTO {table} ({columns}) VALUES ({values});")
            }
        })
    }
}

pub struct SqlFormatParser;

impl FormatParser for SqlFormatParser {
    fn key(&self) -> &str {
        "sql"
    }

    fn parse(
        &self,
        _format: &spec::Format,
        simulation: &spec::Spec,
    ) -> Result<Box<dyn Format>, Vec<ParseError>> {
        let effect_tables = simulation
            .effects
            .iter()
            .filter_map(|(key, effect)| {
                let table = effect.metadata.as_ref()?.get("table")?.as_str()?;
                Some((key.clone(), table.to_string()))
            })
            .collect();

        Ok(Box::new(SqlFormat { effect_tables }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn event(data: Value) -> Input {
        Input {
            id: 1,
            effect: "user".to_string(),
            offset: 0,
            timestamp: Utc::now().fixed_offset(),
            data,
            metadata: vec![],
        }
    }

    #[test]
    fn defaults_table_to_effect_key() {
        let format = SqlFormat {
            effect_tables: HashMap::new(),
        };
        let event = event(json!({ "id": 1, "name": "alice" }));
        let sql = format.format(&event).unwrap();
        assert!(
            sql.starts_with("INSERT INTO user ("),
            "expected INSERT INTO user, got: {sql}"
        );
        assert!(sql.contains("\"id\""));
        assert!(sql.contains("\"name\""));
    }

    #[test]
    fn table_can_be_overridden_by_metadata() {
        let format = SqlFormat {
            effect_tables: HashMap::from([("user".to_string(), "accounts".to_string())]),
        };
        let event = event(json!({ "id": 1 }));
        let sql = format.format(&event).unwrap();
        assert!(
            sql.starts_with("INSERT INTO accounts ("),
            "expected INSERT INTO accounts, got: {sql}"
        );
    }
}
