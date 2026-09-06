mod init;
mod run;
mod skills;
mod ui;

use clap::{Parser, Subcommand};

/// Simulate code usage, record everything and analyze the results
#[derive(Parser)]
#[command(name = "rngo", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a directory for rngo
    ///
    /// Creates a `.rngo` directory, a starter `.rngo/spec.yml`, and
    /// updates `.gitignore`.
    Init {
        /// Directory to initialize
        #[arg(long, default_value = ".")]
        dir: std::path::PathBuf,
        /// Don't prompt for input and don't install agent skills
        ///
        /// Uses the current directory name as the project key and 1 as the
        /// seed.
        #[arg(long)]
        default: bool,
    },
    /// Run a simulation
    ///
    /// Loads a spec, runs the simulation, routes events to channels,
    /// and records everything.
    Run {
        /// Write  events to stdout (instead of routing to channels)
        #[arg(long)]
        stdout: bool,
        /// Check that the simulation can be built without generating or persisting anything
        #[arg(long)]
        dry_run: bool,
        /// Cap the number of effect and error events a run produces
        #[arg(long)]
        limit: Option<std::num::NonZeroU64>,
        /// Path to a spec file (instead of building from the `.rngo` directory)
        #[arg(long)]
        spec: Option<std::path::PathBuf>,
        /// Path to the `.rngo` directory
        #[arg(long, default_value = ".")]
        dir: std::path::PathBuf,
    },
    /// Manage rngo agent skills
    Skills {
        #[command(subcommand)]
        command: SkillsCommands,
    },
}

#[derive(Subcommand)]
enum SkillsCommands {
    /// Download the latest rngo agent skills and install them
    ///
    /// Idempotent: any previously installed `rngo-` skills in the target
    /// directory are replaced with the latest release.
    Install {
        /// Where to install skills. Skips the interactive location prompt.
        #[arg(long)]
        path: Option<std::path::PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { dir, default } => {
            if let Err(e) = init::init(&dir, default) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Run {
            stdout,
            dry_run,
            limit,
            dir,
            spec,
        } => match run::run(&dir, stdout, spec.as_deref(), dry_run, limit) {
            Ok(true) => {}
            Ok(false) => std::process::exit(1),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
        Commands::Skills { command } => match command {
            SkillsCommands::Install { path } => {
                if let Err(e) = skills::install(std::path::Path::new("."), path) {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        },
    }
}
