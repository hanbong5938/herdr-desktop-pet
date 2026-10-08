mod helper_ui;
mod runner;

use herdr_update_coordinator::{protocol, read_plan, OperationPhase};
use std::path::PathBuf;

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let command = args.next().ok_or("usage: herdr-update-coordinator update-capabilities | run/status/recover --state-dir PATH --operation-id ID [--start]")?;
    if command == "update-capabilities" {
        if args.next().is_some() {
            return Err("update-capabilities accepts no arguments".into());
        }
        println!("{{\"protocol\":{}}}", protocol::UPDATER_PROTOCOL);
        return Ok(());
    }
    let mut state_dir = None;
    let mut operation_id = None;
    let mut start = false;
    while let Some(arg) = args.next() {
        if arg == "--state-dir" && state_dir.is_none() {
            state_dir = Some(PathBuf::from(
                args.next().ok_or("--state-dir requires a path")?,
            ));
        } else if arg == "--operation-id" && operation_id.is_none() {
            operation_id = Some(
                args.next()
                    .ok_or("--operation-id requires a value")?
                    .into_string()
                    .map_err(|_| "invalid operation ID encoding")?,
            );
        } else if arg == "--start" && !start {
            start = true;
        } else {
            return Err(format!(
                "unexpected updater argument: {}",
                arg.to_string_lossy()
            ));
        }
    }
    let state_dir = state_dir.ok_or("missing --state-dir")?;
    let operation_id = operation_id.ok_or("missing --operation-id")?;
    protocol::validate_operation_id(&operation_id)?;
    if start && command != "recover" {
        return Err("--start is only valid with recover".into());
    }
    let plan = read_plan(&state_dir, &operation_id)?;
    match command.to_str().ok_or("invalid updater command encoding")? {
        "run" => runner::run(plan),
        "status" => {
            let status = runner::inspect(&plan)?;
            println!(
                "{}",
                serde_json::to_string(&status).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        "recover" => {
            let result = runner::recover(plan, start)?;
            println!(
                "{}",
                serde_json::to_string(&result).map_err(|e| e.to_string())?
            );
            if result.phase == OperationPhase::Completed {
                Ok(())
            } else {
                Err(format!("update was not applied: {}", result.detail))
            }
        }
        _ => Err("unknown updater command".into()),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("herdr-update-coordinator: {error}");
        std::process::exit(1);
    }
}
