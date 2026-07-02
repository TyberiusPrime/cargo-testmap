use anyhow::Result;
use std::process::ExitCode;

mod cli;
mod collect;
mod config;
mod report;
mod util;

fn main() -> ExitCode {
    // When invoked as `cargo testmap ...`, cargo passes `testmap` as argv[1].
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args.len() >= 2 && args[1] == "testmap" {
        args.remove(1);
    }

    match try_main(args) {
        Ok(code) => code,
        Err(e) => {
            let mut chain = e.chain();
            if let Some(first) = chain.next() {
                eprint!("error: {first}");
                for cause in chain {
                    eprint!("\n  caused by: {cause}");
                }
                eprintln!();
            }
            ExitCode::from(1)
        }
    }
}

fn try_main(args: Vec<std::ffi::OsString>) -> Result<ExitCode> {
    use clap::Parser;
    let cli = cli::Cli::parse_from(args);

    match cli.command {
        cli::Command::Collect(c) => {
            let mut c = c;
            c.output = cli::resolve_default_path(&c.output);
            collect::run(c)?;
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::Report(c) => {
            let mut c = c;
            c.input = cli::resolve_default_path(&c.input);
            c.output_dir = cli::resolve_default_path(&c.output_dir);
            report::run(c)?;
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::Run(r) => {
            // `run` = collect (writes the database) immediately followed by
            // report (reads that same database). The shared `output` path is
            // what links the two phases.
            let db = cli::resolve_default_path(&r.output);
            let output_dir = cli::resolve_default_path(&r.output_dir);
            let opts = r.opts;
            collect::run(cli::CollectArgs {
                opts,
                output: db.clone(),
            })?;
            report::run(cli::ReportArgs {
                input: db,
                output_dir,
                theme: r.theme,
                single_file: r.single_file,
            })?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
