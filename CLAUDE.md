# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## After editing code

Always run `just fmt` and `just clippy` after making code changes. If clippy reports warnings or errors, fix them directly in the code.

## Commands

```bash
cargo test --workspace          # run all tests
cargo test -p rngo-sim          # run sim crate tests only
cargo test <test_name>          # run a single test by name
just fmt                        # format all Rust code (preferred)
cargo fmt                       # format code (rustfmt.toml: imports_granularity = "Module")
cargo fmt --check               # check formatting (used in CI)
just clippy                     # lint, matching CI (warnings are errors)
just clippy-fix                 # lint and auto-fix clippy suggestions
cargo clippy --workspace --all-targets -- -D warnings  # lint (warnings are errors in CI)
cargo build                     # build
cargo run -p rngo-cli -- run            # run simulation (writes to .rngo/runs/local/<N>/)
cargo run -p rngo-cli -- run --stdout   # run simulation, print all events to stdout as JSON
```

## Architecture

The workspace has two crates:
- `crates/sim` (`rngo-sim`) — core simulation library
- `crates/cli` (`rngo-cli`) — CLI binary that wires the library to the filesystem and subprocesses

### Data flow

1. **Spec** (`spec.rs`): A YAML/JSON document loaded from `.rngo/spec.yml` + per-file `effects/*.yml` and `channels/*.yml`. Defines `seed`, `start`, `end`, named `effects`, and named `channels`.

2. **Dialect::parse_simulation** (`spec/parse.rs`): Converts a `spec::Simulation` into a `SimulationBuilder` by dispatching each effect's schema to a matching `SchemaParser` and each effect's format to a matching `FormatParser`. `Dialect::core()` registers all built-in parsers.

3. **Simulation** (`simulation.rs`): An `Iterator<Item = Input>`. Each call to `next()` sorts all `Effect`s by their next timestamp offset and advances the earliest one.

4. **Effect** (`effect.rs`): Also an iterator, yielding `Result<Input, SkippedInput>`. Driven by a `Trigger` (either a `Clock` for time-based firing or another `Effect` for dependency-based firing) and a `Schema` for generating values. `Input` (`{ id, effect, offset, timestamp, data, metadata }`) is the event an effect produces each time it fires.

### CLI run loop (`cli/src/run/exec.rs`)

- Loads spec, creates a run directory at `.rngo/runs/<UUID>/`, writes `spec.json` snapshot and initializes a `log.sqlite` SQLite database.
- Without `--stdout`: writes each `Input` to the `inputs` table in the SQLite database and dispatches to any assigned channel via `ChannelDispatch`.
- With `--stdout`: serializes all input events to stdout.

### Channel targets (`cli/src/run/channel.rs`)

`ChannelDispatch` implements two integration modes for `channels`:
- `stream`: spawns one long-lived subprocess per channel, writes formatted event lines to its stdin.
- `exec`: runs a fresh `sh -c <command>` per event; the command string is a Handlebars template rendered with the event's JSON value.

An effect opts into a channel by setting `channel: <channel-key>`. The format used is resolved by merging the effect-level `format` over the channel-level `format`. A `stream` channel with no effects writing to it is still spawned for the run's duration, but only as an output source (e.g. tailing a log file) - its stdout/stderr lines still become `Output` events, just with no associated effect.

### Schema types (all in `sim/src/schema/`)

`Array`, `Constant`, `Context`, `Function`, `Number`, `Object`, `Reference`, `Select`, `Str`. Each implements `SchemaBuilder` (parse-time) and `Schema` (run-time). Builder factory functions are re-exported from `sim/src/build.rs`.

### Log (`sim/src/log.rs`)

A shared `Rc<dyn LogReader>` is threaded through all effects so that `Reference` and trigger-by-effect can look up previously emitted input events (`LogEvent::Input`). `SimpleEventLog` is the in-memory implementation used for this; `FsProxyLog` and `SqliteProxyLog` wrap a child `Log` to additionally persist events to disk.
