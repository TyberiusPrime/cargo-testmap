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
            collect::run(c)?;
            Ok(ExitCode::SUCCESS)
        }
        cli::Command::Report(c) => {
            report::run(c)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
