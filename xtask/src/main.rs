//! `cargo xtask <command>`: repository checks run by `just ci`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod gen_ts;
mod spec_trace;
mod versions;

#[derive(Debug, Parser)]
#[command(name = "xtask")]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Debug, Subcommand)]
enum Task {
    /// List requirement citations and fail on broken traceability (TST-040).
    SpecTrace,
    /// Check that the omp release pins agree (ENV-010, ENV-011).
    CheckVersions,
    /// Regenerate the TypeScript protocol types from the daemon's JSON Schemas (AGT-007).
    GenTs,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the repository root")
        .to_path_buf()
}

fn main() -> anyhow::Result<ExitCode> {
    let root = repo_root();
    match Cli::parse().command {
        Task::GenTs => {
            gen_ts::run(&root)?;
            println!("wrote {}", gen_ts::OUTPUT);
            Ok(ExitCode::SUCCESS)
        }
        Task::SpecTrace => {
            let report = spec_trace::run(&root)?;
            for (id, files) in &report.citations {
                let files: Vec<&str> = files.iter().map(String::as_str).collect();
                println!("{id:<8} {}", files.join(", "));
            }
            for (id, places) in &report.not_run {
                let places: Vec<&str> = places.iter().map(String::as_str).collect();
                println!("not run: {id:<8} {}", places.join(", "));
            }
            for error in &report.errors {
                eprintln!("error: {error}");
            }
            Ok(if report.errors.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Task::CheckVersions => {
            let observed = versions::observe(&root)?;
            let problems = versions::mismatches(&observed);
            for problem in &problems {
                eprintln!("error: {problem}");
            }
            if problems.is_empty() {
                println!(
                    "omp {} (flake {} / omp-rpc {})",
                    observed.omp_version, observed.flake_ref, observed.cargo_tag
                );
            }
            Ok(if problems.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
    }
}
