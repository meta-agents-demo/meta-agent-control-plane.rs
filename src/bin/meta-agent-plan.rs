use std::{
    fs,
    io::{self, Read},
    path::PathBuf,
    process::ExitCode,
};

use clap::Parser;
use meta_agent_control_plane::{
    coordination::{PlanningError, PlanningPolicy, build_plan_with_policy},
    metacognition::AnalysisPolicy,
    store::Snapshot,
};
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(
    name = "meta-agent-plan",
    about = "Build a bounded, dependency-safe coordination plan from a retained snapshot"
)]
struct Args {
    /// Snapshot JSON file. Omit or pass '-' to read stdin.
    #[arg(value_name = "SNAPSHOT")]
    input: Option<PathBuf>,

    #[arg(long, default_value_t = 16)]
    max_assignments: usize,

    #[arg(long, default_value_t = 2)]
    max_assignments_per_agent: usize,

    #[arg(long, default_value_t = 32)]
    max_interventions: usize,

    #[arg(long, default_value_t = 64)]
    max_holds: usize,

    #[arg(long, default_value_t = 15 * 60)]
    stale_after_seconds: i64,

    #[arg(long, default_value_t = 3)]
    retry_loop_attempts: u32,

    #[arg(long, default_value_t = 0.45)]
    low_confidence_threshold: f32,

    #[arg(long, default_value_t = false)]
    pretty: bool,
}

#[derive(Debug, Error)]
enum CliError {
    #[error("failed to read snapshot: {0}")]
    Read(#[from] io::Error),
    #[error("snapshot JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Planning(#[from] PlanningError),
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("meta-agent-plan: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<String, CliError> {
    let input = read_input(args.input.as_ref())?;
    let snapshot = serde_json::from_str::<Snapshot>(&input)?;
    let plan = build_plan_with_policy(
        &snapshot,
        AnalysisPolicy {
            stale_after_seconds: args.stale_after_seconds,
            retry_loop_attempts: args.retry_loop_attempts,
            low_confidence_threshold: args.low_confidence_threshold,
        },
        PlanningPolicy {
            max_assignments: args.max_assignments,
            max_assignments_per_agent: args.max_assignments_per_agent,
            max_interventions: args.max_interventions,
            max_holds: args.max_holds,
        },
    )?;
    if args.pretty {
        serde_json::to_string_pretty(&plan).map_err(CliError::from)
    } else {
        serde_json::to_string(&plan).map_err(CliError::from)
    }
}

fn read_input(path: Option<&PathBuf>) -> Result<String, io::Error> {
    match path {
        Some(path) if path.as_os_str() != "-" => fs::read_to_string(path),
        _ => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input)?;
            Ok(input)
        }
    }
}
