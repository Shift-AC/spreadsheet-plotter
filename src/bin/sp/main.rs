use std::{
    backtrace::BacktraceStatus,
    process::{Command, Stdio, exit},
};

use anyhow::bail;
use spreadsheet_plotter::{DataSeriesSource, Plotter};
use sqlformat::{FormatOptions, QueryParams};

use crate::cli::{Cli, Mode};

mod cli;

fn handle_err(e: anyhow::Error) {
    e.chain().for_each(|e| eprintln!("Error: {e}"));
    let bt = e.backtrace();
    match bt.status() {
        BacktraceStatus::Captured => {
            eprintln!("Backtrace:\n{bt}");
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
    Ok(())
}

fn try_main() -> anyhow::Result<()> {
    env_logger::init();
    let cli = Cli::parse_args()?;
    check_dependencies()?;

    if matches!(cli.mode, Mode::Replot) {
        if which::which("gnuplot").is_err() {
            bail!("gnuplot is not installed");
        }
        Plotter::plot(&cli.gnuplot_cmd)?;
    } else {
        let complete_sql = format!(
            "{}{}{}{}",
            cli.data_input.to_sql("src_tbl"),
            cli.selector.to_preprocess_sql("src_tbl", "t0"),
            match &cli.opseq {
                Some(opseq) => opseq.to_sql("t0", "x", "y"),
                None => "".to_string(),
            },
            cli.selector.to_postprocess_sql(&match &cli.opseq {
                Some(opseq) => opseq.get_tmp_table_name(),
                None => "t0".to_string(),
            }),
        );

        if matches!(cli.mode, Mode::DryRun) {
            let options = FormatOptions {
                indent: sqlformat::Indent::Spaces(4),
                uppercase: Some(true),
                lines_between_queries: 1,
                max_inline_arguments: Some(80),
                max_inline_top_level: Some(80),
                joins_as_top_level: true,
                dialect: sqlformat::Dialect::Generic,
                ..Default::default()
            };
            let formatted_sql =
                sqlformat::format(&complete_sql, &QueryParams::None, &options);
            println!("{formatted_sql}");
            return Ok(());
        }

        if which::which("duckdb").is_err() {
            bail!("duckdb is not installed");
        }

        if matches!(cli.mode, Mode::Dump) {
            let status = Command::new("duckdb")
                .arg("-csv")
                .arg("-bail")
                .arg("-c")
                .arg(complete_sql.clone())
                .stdout(Stdio::inherit())
                .spawn()?
                .wait()?;
            if !status.success() {
                bail!(
                    "duckdb failed with {status}\nOriginal SQL:\n{complete_sql}"
                );
            }
            return Ok(());
        }

        let mut child = Command::new("duckdb")
            .arg("-csv")
            .arg("-bail")
            .arg("-c")
            .arg(complete_sql)
            .stdout(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().unwrap();
        let dss = DataSeriesSource::Child(stdout);
        dss.dump(Some(cli.tmp_datasheet_path))?;
        let status = child.wait()?;
        if !status.success() {
            bail!("duckdb failed with {status}");
        }

        if which::which("gnuplot").is_err() {
            bail!("gnuplot is not installed");
        }
        Plotter::plot(&cli.gnuplot_cmd)?;
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
