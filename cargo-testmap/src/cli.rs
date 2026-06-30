use clap::{Args, Parser, Subcommand};

/// cargo-testmap — which tests cover this line of code?
#[derive(Parser)]
#[command(
    name = "cargo-testmap",
    bin_name = "cargo-testmap",
    version,
    propagate_version = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Collect per-test coverage and build a testmap.json database.
    Collect(CollectArgs),
    /// Generate an HTML report from a testmap.json database.
    Report(ReportArgs),
}

#[derive(Args)]
pub struct CollectArgs {
    /// Collect across all workspace members (default).
    #[arg(long)]
    pub workspace: bool,

    /// Specific package to collect.
    #[arg(short = 'p', long)]
    pub package: Option<String>,

    /// Target filters.
    #[arg(long)]
    pub lib: bool,
    #[arg(long)]
    pub bins: bool,
    #[arg(long)]
    pub tests: bool,

    /// Extra arguments forwarded to `cargo test` for selecting targets.
    ///
    /// Pass them after a `--`, e.g. `cargo testmap collect -- --features foo`.
    #[arg(last = true)]
    pub cargo_args: Vec<String>,

    /// Only collect tests whose full path matches this regex.
    #[arg(long)]
    pub filter: Option<String>,

    /// Skip tests whose full path matches this regex.
    #[arg(long)]
    pub skip: Option<String>,

    /// Omit lines covered by >= N tests (default: 10).
    #[arg(long, default_value_t = 10)]
    pub threshold: u32,

    /// Number of parallel test runs (default: number of CPUs).
    #[arg(short = 'j', long = "jobs")]
    pub jobs: Option<usize>,

    /// Database output path (default: target/testmap/testmap.json).
    #[arg(long, default_value = "target/testmap/testmap.json")]
    pub output: String,

    /// Force a full re-collection, ignoring any staged results.
    #[arg(long)]
    pub clean: bool,

    /// Suppress cargo's own output (handled by testmap).
    #[arg(short = 'v', long, default_value_t = false)]
    pub verbose: bool,
}

#[derive(Args)]
pub struct ReportArgs {
    /// Database input path (default: target/testmap/testmap.json).
    #[arg(long, default_value = "target/testmap/testmap.json")]
    pub input: String,

    /// Report output directory (default: target/testmap/report).
    #[arg(long, default_value = "target/testmap/report")]
    pub output_dir: String,

    /// Syntax-highlighting theme.
    ///
    /// Available built-ins: base16-ocean.dark (default), base16-mocha.dark,
    /// base16-eighties.dark, base16-ocean.light, Solarized (dark),
    /// Solarized (light), InspiredGitHub.
    #[arg(long)]
    pub theme: Option<String>,

    /// Generate a single self-contained HTML file instead of a directory.
    #[arg(long, value_name = "PATH")]
    pub single_file: Option<String>,
}
