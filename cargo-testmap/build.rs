use clap::{Arg, Command};
use clap_complete::generate_to;
use std::fs;
use std::path::PathBuf;

/// Mirrors the CLI definition in src/cli.rs so completions stay in sync.
/// If you add or rename a subcommand/flag there, update this accordingly.
fn build_cli() -> Command {
    let collect_opts = &[
        Arg::new("workspace")
            .long("workspace")
            .help("Collect across all workspace members (default).")
            .action(clap::ArgAction::SetTrue),
        Arg::new("package")
            .short('p')
            .long("package")
            .value_name("PACKAGE")
            .help("Specific package to collect."),
        Arg::new("lib")
            .long("lib")
            .help("Include library targets.")
            .action(clap::ArgAction::SetTrue),
        Arg::new("bins")
            .long("bins")
            .help("Include binary targets.")
            .action(clap::ArgAction::SetTrue),
        Arg::new("tests")
            .long("tests")
            .help("Include test targets.")
            .action(clap::ArgAction::SetTrue),
        Arg::new("filter")
            .long("filter")
            .value_name("REGEX")
            .help("Only collect tests whose full path matches this regex."),
        Arg::new("skip")
            .long("skip")
            .value_name("REGEX")
            .help("Skip tests whose full path matches this regex."),
        Arg::new("threshold")
            .long("threshold")
            .default_value("10")
            .help("Omit lines covered by >= N tests."),
        Arg::new("jobs")
            .short('j')
            .long("jobs")
            .value_name("N")
            .help("Number of parallel test runs (default: number of CPUs)."),
        Arg::new("verbose")
            .short('v')
            .long("verbose")
            .help("Show additional output.")
            .action(clap::ArgAction::SetTrue),
        Arg::new("cargo_args")
            .help("Extra arguments forwarded to `cargo test`.")
            .trailing_var_arg(true)
            .num_args(1..)
            .allow_hyphen_values(true),
    ];

    Command::new("cargo-testmap")
        .version(env!("CARGO_PKG_VERSION"))
        .about("cargo-testmap — which tests cover this line of code?")
        .subcommand_required(true)
        .disable_help_subcommand(true)
        .subcommand(
            Command::new("collect")
                .about("Collect per-test coverage and build a testmap.json database.")
                .args(collect_opts)
                .arg(
                    Arg::new("output")
                        .long("output")
                        .default_value("target/testmap/testmap.json")
                        .help("Database output path."),
                ),
        )
        .subcommand(
            Command::new("report")
                .about("Generate an HTML report from a testmap.json database.")
                .arg(
                    Arg::new("input")
                        .long("input")
                        .default_value("target/testmap/testmap.json")
                        .help("Database input path."),
                )
                .arg(
                    Arg::new("output_dir")
                        .long("output-dir")
                        .default_value("target/testmap/report")
                        .help("Report output directory."),
                )
                .arg(
                    Arg::new("theme")
                        .long("theme")
                        .value_name("THEME")
                        .help("Syntax-highlighting theme."),
                )
                .arg(
                    Arg::new("single_file")
                        .long("single-file")
                        .value_name("PATH")
                        .help("Generate a single self-contained HTML file."),
                ),
        )
        .subcommand(
            Command::new("run")
                .about("Collect coverage and build the report in one go (collect then report).")
                .args(collect_opts)
                .arg(
                    Arg::new("output")
                        .long("output")
                        .default_value("target/testmap/testmap.json")
                        .help("Database path: collect writes here, report reads here."),
                )
                .arg(
                    Arg::new("output_dir")
                        .long("output-dir")
                        .default_value("target/testmap/report")
                        .help("Report output directory."),
                )
                .arg(
                    Arg::new("theme")
                        .long("theme")
                        .value_name("THEME")
                        .help("Syntax-highlighting theme."),
                )
                .arg(
                    Arg::new("single_file")
                        .long("single-file")
                        .value_name("PATH")
                        .help("Generate a single self-contained HTML file."),
                ),
        )
}

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let completions_dir = out_dir.join("completions");
    fs::create_dir_all(&completions_dir).unwrap();

    let mut app = build_cli();
    generate_to(
        clap_complete::Shell::Fish,
        &mut app,
        "cargo-testmap",
        completions_dir.clone(),
    )
    .unwrap();

    // Also emit into the crate source tree (next to Cargo.toml) so Nix
    // postInstall can install it from the naersk build copy.
    let src_completions = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("completions");
    let _ = fs::create_dir_all(&src_completions);
    let _ = fs::copy(
        completions_dir.join("cargo-testmap.fish"),
        src_completions.join("cargo-testmap.fish"),
    );

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/cli.rs");
}
