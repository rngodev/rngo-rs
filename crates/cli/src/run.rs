mod status;

use console::style;
use rngo_sim::{Dialect, RunLog, SimulationEvent, SqliteRunLog, signal, spec};
use status::StatusRunLog;
use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::{fmt, fs};
use uuid::Uuid;

pub fn run(
    base: &Path,
    stdout: bool,
    spec_path: Option<&Path>,
    dry_run: bool,
    limit: Option<std::num::NonZeroU64>,
) -> Result<bool, Box<dyn Error>> {
    let _ = dotenvy::from_path(base.join(".env"));

    let spec = match spec_path {
        Some(path) => load_spec_file(path)?,
        None => load_spec(base)?,
    };

    let dialect = Dialect::primitive();

    let mut simulation_builder = dialect.parse_spec(spec.clone()).map_err(join_errors)?;

    if let Some(limit) = limit {
        simulation_builder = simulation_builder.limit(limit.get());
    }

    if dry_run {
        simulation_builder.build().map_err(join_errors)?;
        return Ok(true);
    }

    let run_dir = new_run_dir(base)?;
    fs::create_dir_all(&run_dir)?;
    fs::write(
        run_dir.join("spec.json"),
        serde_json::to_string_pretty(&spec)?,
    )?;
    update_last_symlink(base, &run_dir)?;

    let effect_channels: HashMap<String, String> = spec
        .effects
        .iter()
        .filter_map(|(k, v)| v.channel.as_ref().map(|s| (k.clone(), s.clone())))
        .collect();

    let mut run_log = StatusRunLog::new(
        Box::new(SqliteRunLog::new(run_dir.clone(), simulation_builder.seed)),
        effect_channels,
    );

    let mut simulation = simulation_builder
        .run_log_reader(run_log.reader())
        .build()
        .map_err(join_errors)?;

    let system_builder = dialect.parse_system(spec.clone()).map_err(join_errors)?;

    let mut system = system_builder.build().map_err(join_errors)?;

    for event in &mut simulation {
        if stdout {
            println!("{}", serde_json::to_string(&event)?);
        } else {
            match event {
                SimulationEvent::Input(input) => {
                    let outputs = system.send(&input)?;
                    run_log.push_input(input);
                    for output in outputs {
                        run_log.push_output(output);
                    }
                }
                SimulationEvent::SkippedInput(_skipped_input) => todo!(),
            }
        }
    }

    // Closes stdin on every stream channel (triggering exit for those that react to EOF) and
    // kills any stragglers - including output-source channels with no natural end - after a
    // grace period. `system` remains iterable afterward for exactly this reason: outputs
    // produced on a channel's own schedule (e.g. a `stream` subprocess flushing once it gets
    // EOF, or an effect-less channel that only ever produces ambient output) never came back
    // from `send`, so they're drained here instead, before signals below evaluate the run log.
    system.finish();
    for output in &mut system {
        run_log.push_output(output);
    }

    let mut all_passed = true;

    if !spec.signals.is_empty() {
        let outcomes = signal::evaluate(&mut run_log, &spec.signals);

        println!();
        println!("{}", style("Audit").bold());

        let mut checked = 0;
        let mut passed = 0;

        for (key, outcome) in &outcomes {
            let spec::Signal::Sql { expect, .. } = &spec.signals[key];

            if let Some(error) = &outcome.error {
                all_passed = false;
                if expect.is_some() {
                    checked += 1;
                }
                println!("{key}: error - {error}");
                continue;
            }

            let value = outcome.value.as_ref().unwrap();
            match expect {
                Some(expect) => {
                    checked += 1;
                    if outcome.passed.unwrap() {
                        passed += 1;
                        println!("{key}: {value} (passed)");
                    } else {
                        all_passed = false;
                        println!("{key}: {value} (failed - expected '{expect}')");
                    }
                }
                None => println!("{key}: {value}"),
            }
        }

        if checked > 0 {
            println!("{passed} passed");
            println!("{} failed", checked - passed);
        }
    }

    Ok(all_passed)
}

fn load_spec_file(path: &Path) -> Result<spec::Spec, Box<dyn Error>> {
    let value: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(path)?)?;
    Ok(spec::from_value(value).map_err(join_errors)?)
}

fn load_spec(base: &Path) -> Result<spec::Spec, Box<dyn Error>> {
    let spec_path = base.join(".rngo/spec.yml");

    let mut spec: serde_json::Value = if spec_path.exists() {
        serde_yaml::from_str(&fs::read_to_string(&spec_path)?)?
    } else {
        serde_json::json!({ "seed": 1, "effects": {} })
    };

    if !spec["effects"].is_object() {
        spec["effects"] = serde_json::json!({});
    }

    let effects_dir = base.join(".rngo/effects");
    if effects_dir.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(&effects_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yml"))
            .collect();
        paths.sort();

        for path in paths {
            let key = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| format!("invalid filename: {}", path.display()))?
                .to_string();
            let effect: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(&path)?)?;
            spec["effects"][key] = effect;
        }
    }

    if !spec["channels"].is_object() {
        spec["channels"] = serde_json::json!({});
    }

    let channels_dir = base.join(".rngo/channels");
    if channels_dir.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(&channels_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yml"))
            .collect();
        paths.sort();

        for path in paths {
            let key = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| format!("invalid filename: {}", path.display()))?
                .to_string();
            let channel: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(&path)?)?;
            spec["channels"][key] = channel;
        }
    }

    if !spec["schemas"].is_object() {
        spec["schemas"] = serde_json::json!({});
    }

    let schemas_dir = base.join(".rngo/schemas");
    if schemas_dir.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(&schemas_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yml"))
            .collect();
        paths.sort();

        for path in paths {
            let key = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| format!("invalid filename: {}", path.display()))?
                .to_string();
            let schema: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(&path)?)?;
            spec["schemas"][key] = schema;
        }
    }

    if !spec["signals"].is_object() {
        spec["signals"] = serde_json::json!({});
    }

    let signals_dir = base.join(".rngo/signals");
    if signals_dir.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(&signals_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yml"))
            .collect();
        paths.sort();

        for path in paths {
            let key = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| format!("invalid filename: {}", path.display()))?
                .to_string();
            let signal: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(&path)?)?;
            spec["signals"][key] = signal;
        }
    }

    Ok(spec::from_value(spec).map_err(join_errors)?)
}

fn new_run_dir(base: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let runs_dir = base.join(".rngo/runs");
    fs::create_dir_all(&runs_dir)?;
    let id = Uuid::now_v7();
    Ok(runs_dir.join(id.to_string()))
}

fn update_last_symlink(base: &Path, run_dir: &Path) -> Result<(), Box<dyn Error>> {
    let link = base.join(".rngo/runs/last");
    if link.exists() || link.is_symlink() {
        fs::remove_file(&link)?;
    }
    let target = run_dir.strip_prefix(base.join(".rngo/runs"))?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, &link)?;
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(target, &link)?;
    Ok(())
}

fn join_errors<E: fmt::Display>(errors: Vec<E>) -> String {
    errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn write_yaml(path: impl AsRef<Path>, value: &serde_json::Value) {
        fs::write(path, serde_yaml::to_string(value).unwrap()).unwrap();
    }

    fn signal_outcome(base: &Path, key: &str) -> (serde_json::Value, bool) {
        let connection =
            rusqlite::Connection::open(base.join(".rngo/runs/last/log.sqlite")).unwrap();
        let (value, result): (Option<String>, Option<String>) = connection
            .query_row(
                "SELECT value, result FROM signals WHERE key = ?1",
                rusqlite::params![key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        (
            serde_json::from_str(&value.unwrap()).unwrap(),
            result.unwrap() == "passed",
        )
    }

    #[test]
    fn exec_target_runs_command_per_event() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("exec_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let command = "echo {{id}} >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": {},
                "target": { "type": "exec", "command": command }
            }),
        );

        run(base, false, None, false, None).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert!(
            content.lines().count() > 0,
            "exec command should have run once per event"
        );
    }

    #[test]
    fn exec_target_records_output_when_command_fails() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();
        fs::create_dir_all(base.join(".rngo/signals")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/signals/has-failure-output.yml"),
            &json!({
                "type": "sql",
                "query": "SELECT COUNT(*) FROM outputs WHERE data LIKE 'command exited with%'",
                "expect": "result >= 1"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": {},
                "target": { "type": "exec", "command": "exit 1" }
            }),
        );

        run(base, false, None, false, None).unwrap();

        let (_, passed) = signal_outcome(base, "has-failure-output");
        assert!(passed);
    }

    #[test]
    fn stream_target_does_not_drop_trailing_output() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();
        fs::create_dir_all(base.join(".rngo/signals")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        // Every event fed to `cat` is echoed straight back out over stdout, so the number of
        // outputs recorded should match the number of effects exactly - including the one
        // produced by the very last event, which arrives only after the subprocess is closed.
        write_yaml(
            base.join(".rngo/signals/matches-effect-count.yml"),
            &json!({
                "type": "sql",
                "query": "SELECT (SELECT COUNT(*) FROM inputs) - (SELECT COUNT(*) FROM outputs)",
                "expect": "result == 0"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": {},
                "target": { "type": "stream", "command": "cat" }
            }),
        );

        run(base, false, None, false, None).unwrap();

        let (value, passed) = signal_outcome(base, "matches-effect-count");
        assert!(
            passed,
            "expected output count to match effect count, got {value}"
        );
    }

    #[test]
    fn stream_target_pipes_events_to_subprocess() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("stream_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let command = "cat >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": {},
                "target": { "type": "stream", "command": command }
            }),
        );

        run(base, false, None, false, None).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert!(
            content.lines().count() > 0,
            "stream subprocess should have received events"
        );
    }

    #[test]
    fn sql_format_uses_table_from_effect_metadata() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("sql_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-02"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "db",
                "metadata": { "table": "accounts" },
                "trigger": "hz(1, hour)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let command = "cat >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/channels/db.yml"),
            &json!({
                "format": { "type": "sql" },
                "target": { "type": "stream", "command": command }
            }),
        );

        run(base, false, None, false, None).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert!(
            content.lines().count() > 0,
            "expected at least one formatted line"
        );
        for line in content.lines() {
            assert!(
                line.starts_with("INSERT INTO accounts ("),
                "expected sql format using metadata table, got: {line}"
            );
        }
    }

    #[test]
    fn spec_flag_uses_given_file_instead_of_rngo_dir() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("spec_flag_output.txt");

        // A broken `.rngo/spec.yml` proves it is never read when `--spec` is used.
        fs::create_dir_all(base.join(".rngo")).unwrap();
        fs::write(base.join(".rngo/spec.yml"), "not: [valid").unwrap();

        let command = "cat >> ".to_string() + output.to_str().unwrap();
        let spec_path = base.join("external_spec.yml");
        write_yaml(
            &spec_path,
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04",
                "channels": {
                    "logger": {
                        "format": {},
                        "target": { "type": "stream", "command": command }
                    }
                },
                "effects": {
                    "ping": {
                        "channel": "logger",
                        "trigger": "hz(1, day)",
                        "schema": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                            }
                        }
                    }
                }
            }),
        );

        run(base, false, Some(&spec_path), false, None).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert!(
            content.lines().count() > 0,
            "run should use the spec file passed via --spec"
        );
    }

    #[test]
    fn schemas_dir_yml_files_are_referenceable_by_type_name() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("schemas_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();
        fs::create_dir_all(base.join(".rngo/schemas")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-02"
            }),
        );

        write_yaml(
            base.join(".rngo/schemas/title.yml"),
            &json!({
                "schema": {
                    "type": "select",
                    "options": [
                        { "schema": { "type": "constant", "value": "Mr." } },
                        { "schema": { "type": "constant", "value": "Mrs." } },
                        { "schema": { "type": "constant", "value": "Dr." } }
                    ]
                }
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, hour)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "title" }
                    }
                }
            }),
        );

        let command = "cat >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": {},
                "target": { "type": "stream", "command": command }
            }),
        );

        run(base, false, None, false, None).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        let lines: Vec<_> = content.lines().collect();
        assert!(!lines.is_empty(), "expected at least one emitted event");
        for line in lines {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            let title = value["title"].as_str().unwrap();
            assert!(
                ["Mr.", "Mrs.", "Dr."].contains(&title),
                "unexpected title {title:?}"
            );
        }
    }

    #[test]
    fn signals_are_evaluated_and_written_to_run_dir() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04",
                "signals": {
                    "hasEvents": {
                        "type": "sql",
                        "query": "SELECT COUNT(*) FROM inputs",
                        "expect": "result >= 1"
                    },
                    "tooMany": {
                        "type": "sql",
                        "query": "SELECT COUNT(*) FROM inputs",
                        "expect": "result > 1000"
                    }
                }
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        run(base, false, None, false, None).unwrap();

        let (has_events_value, has_events_passed) = signal_outcome(base, "hasEvents");
        assert!(has_events_passed);
        assert!(has_events_value.as_i64().unwrap() >= 1);

        let (_, too_many_passed) = signal_outcome(base, "tooMany");
        assert!(!too_many_passed);
    }

    #[test]
    fn signals_dir_yml_files_are_merged_into_spec() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/signals")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/signals/has-events.yml"),
            &json!({
                "type": "sql",
                "query": "SELECT COUNT(*) FROM inputs",
                "expect": "result >= 1"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        run(base, false, None, false, None).unwrap();

        let (_, passed) = signal_outcome(base, "has-events");
        assert!(passed);
    }

    #[test]
    fn channel_with_no_effects_is_an_output_source() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();
        fs::create_dir_all(base.join(".rngo/signals")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        // No effect sets `channel: tail`, so this channel is a pure output source: its subprocess
        // still runs for the duration of the simulation and its output becomes outputs.
        write_yaml(
            base.join(".rngo/channels/tail.yml"),
            &json!({
                "target": { "type": "stream", "command": "printf 'one\\ntwo\\n'" }
            }),
        );

        write_yaml(
            base.join(".rngo/signals/tail-outputs.yml"),
            &json!({
                "type": "sql",
                "query": "SELECT COUNT(*) FROM outputs WHERE input_id IS NULL AND channel = 'tail'",
                "expect": "result == 2"
            }),
        );

        run(base, false, None, false, None).unwrap();

        let (value, passed) = signal_outcome(base, "tail-outputs");
        assert!(
            passed,
            "expected 2 outputs from the effect-less channel, got {value}"
        );
    }

    #[test]
    fn dry_run_does_not_persist_or_run_channels() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("dry_run_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let command = "echo {{id}} >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": {},
                "target": { "type": "exec", "command": command }
            }),
        );

        let passed = run(base, false, None, true, None).unwrap();

        assert!(passed, "dry run of a valid spec should succeed");
        assert!(
            !base.join(".rngo/runs").exists(),
            "dry run should not create a run directory"
        );
        assert!(
            !output.exists(),
            "dry run should not invoke channel side effects"
        );
    }

    #[test]
    fn dry_run_returns_error_when_simulation_cannot_be_built() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        // minimum > maximum is a build-time schema error.
        write_yaml(
            base.join(".rngo/effects/broken.yml"),
            &json!({
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "number",
                    "minimum": 100,
                    "maximum": 18
                }
            }),
        );

        let result = run(base, false, None, true, None);

        assert!(
            result.is_err(),
            "dry run should return an error when the simulation can't be built"
        );
        assert!(
            !base.join(".rngo/runs").exists(),
            "a failed dry run should not create a run directory either"
        );
    }

    #[test]
    fn limit_caps_total_effects_produced() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("limit_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-02-01"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, hour)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let command = "echo {{id}} >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": {},
                "target": { "type": "exec", "command": command }
            }),
        );

        run(
            base,
            false,
            None,
            false,
            Some(std::num::NonZeroU64::new(3).unwrap()),
        )
        .unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert_eq!(
            content.lines().count(),
            3,
            "limit should cap the run at exactly 3 effects"
        );
    }
}
