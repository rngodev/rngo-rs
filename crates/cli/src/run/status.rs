use chrono::{DateTime, FixedOffset};
use console::{Term, style};
use rngo_sim::{EffectMetadata, Input, Output, RunLog, RunLogReader, spec};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Minimum real time between redraws, so a fast-running simulation doesn't spend its time
/// repainting the terminal instead of processing events.
const RENDER_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct ChannelStats {
    effects: u64,
    outputs: u64,
}

/// A [`RunLog`] proxy that renders a live-updating status block to stderr - the current simulated
/// time and, per channel, how many effects and outputs it has produced - leaving stdout free for
/// `--stdout` event output. Forwards every event to `child` unchanged.
pub struct StatusRunLog {
    child: Box<dyn RunLog>,
    effect_channels: HashMap<String, String>,
    term: Term,
    stats: BTreeMap<String, ChannelStats>,
    last_timestamp: Option<DateTime<FixedOffset>>,
    rendered_lines: usize,
    last_render: Option<Instant>,
}

impl StatusRunLog {
    pub fn new(child: Box<dyn RunLog>, effect_channels: HashMap<String, String>) -> Self {
        StatusRunLog {
            child,
            effect_channels,
            term: Term::stderr(),
            stats: BTreeMap::new(),
            last_timestamp: None,
            rendered_lines: 0,
            last_render: None,
        }
    }

    fn render(&mut self, force: bool) {
        if !self.term.is_term() {
            return;
        }

        let now = Instant::now();
        if !force
            && let Some(last) = self.last_render
            && now.duration_since(last) < RENDER_INTERVAL
        {
            return;
        }
        self.last_render = Some(now);

        let time = match self.last_timestamp {
            Some(timestamp) => timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
            None => "-".to_string(),
        };

        let mut lines = Vec::with_capacity(self.stats.len() + 2);
        lines.push(style("Simulation").bold().for_stderr().to_string());
        lines.push(format!("time: {time}"));
        for (channel, stats) in &self.stats {
            lines.push(format!(
                "{channel}: {} effects, {} outputs",
                stats.effects, stats.outputs
            ));
        }

        let _ = self.term.clear_last_lines(self.rendered_lines);
        for line in &lines {
            let _ = self.term.write_line(line);
        }
        self.rendered_lines = lines.len();
    }
}

impl std::fmt::Debug for StatusRunLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatusRunLog").finish_non_exhaustive()
    }
}

impl RunLog for StatusRunLog {
    fn push_input(&mut self, input: Input) {
        self.last_timestamp = Some(input.timestamp);
        if let Some(channel) = self.effect_channels.get(&input.effect) {
            self.stats.entry(channel.clone()).or_default().effects += 1;
        }

        self.render(false);
        self.child.push_input(input);
    }

    fn push_output(&mut self, output: Output) {
        self.stats
            .entry(output.channel.clone())
            .or_default()
            .outputs += 1;

        self.render(false);
        self.child.push_output(output);
    }

    fn push_metadata(&mut self, metadata: EffectMetadata) {
        self.render(false);
        self.child.push_metadata(metadata);
    }

    fn get_signal(&self, signal: spec::Signal) -> Option<serde_json::Value> {
        self.child.get_signal(signal)
    }

    fn reader(&self) -> Rc<dyn RunLogReader> {
        self.child.reader()
    }
}

impl Drop for StatusRunLog {
    fn drop(&mut self) {
        // Guarantees the block reflects final counts even if the last update landed inside the
        // render throttle window.
        self.render(true);
    }
}
