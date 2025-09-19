use std::{backtrace::BacktraceStatus, process::exit};

use anyhow::bail;
use spreadsheet_plotter::Plotter;

use crate::cli::Cli;

mod cli;

fn handle_err(e: anyhow::Error) {
    e.chain().for_each(|e| eprintln!("Error: {}", e));
    let bt = e.backtrace();
    match bt.status() {
        BacktraceStatus::Captured => {
            eprintln!("Backtrace:\n{}", bt);
        }
        BacktraceStatus::Unsupported => {
            eprintln!("Backtrace is unsupported.");
        }
        BacktraceStatus::Disabled => {
            eprintln!("Backtrace is disabled.");
        }
        _ => {
            eprintln!("Unknown backtrace status: {:?}", bt.status());
        }
    }
}

fn check_dependencies() -> anyhow::Result<()> {
    if !which::which("gnuplot").is_ok() {
        bail!("gnuplot is not installed");
    }
    if !which::which("mlr").is_ok() {
        bail!("mlr is not installed");
    }
    Ok(())
}

fn try_main() -> anyhow::Result<()> {
    env_logger::init();
    let cli = Cli::parse_args()?;
    check_dependencies()?;

    if cli.replot {
        Plotter::plot(&cli.gnuplot_cmd)?;
        return Ok(());
    } else {
        let mut plotter = Plotter::from_single_input_file(
            cli.input_path,
            cli.opseq.unwrap(),
            cli.xexpr,
            cli.yexpr,
            cli.input_format,
            cli.output_format,
            cli.gnuplot_cmd,
            cli.output_cache_prefix,
        )?;

        plotter.apply()?;
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    match try_main() {
        Ok(()) => Ok(()),
        Err(e) => {
            handle_err(e);
            exit(1)
        }
    }
}
