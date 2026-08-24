mod backend;
mod cli;
mod spec;

use std::{
    backtrace::BacktraceStatus,
    fs::File,
    process::{Child, Command, Stdio},
};

use anyhow::Context;

use crate::{
    backend::{create_backend, emit_prepare_report},
    cli::{Cli, Mode, get_stdin_reader},
    spec::PreparedSeries,
};

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

fn process_data_series(
    cli: &Cli,
    index: usize,
) -> anyhow::Result<(Child, Option<std::thread::JoinHandle<std::io::Result<()>>>)>
{
    let request = cli.request();
    let series = &request.data_prep.series[index];
    let input = request
        .data_prep
        .inputs
        .iter()
        .find(|input| input.index == series.input_ref)
        .with_context(|| {
            format!("Input reference {} is not registered", series.input_ref)
        })?;

    let output_path = cli.get_output_path(index);
    let log_path = cli.get_log_path(index);
    let stdout = File::create(&output_path).with_context(|| {
        format!(
            "Failed to create prepared data output '{}'",
            output_path.display()
        )
    })?;
    let stderr = File::create(&log_path).with_context(|| {
        format!("Failed to create log '{}'", log_path.display())
    })?;

    let mut command = Command::new("sp");
    if let Some(path) = &input.path {
        command.arg("-i").arg(path);
    }
    if let Some(header_presence) = input.header_presence {
        command.arg("--header").arg(if header_presence {
            "true"
        } else {
            "false"
        });
    }
    if let Some(format) = &input.format {
        command.arg("-f").arg(format.to_string());
    }

    command
        .arg("-m")
        .arg("dump")
        .arg("--if")
        .arg(&series.input_filter)
        .arg("--of")
        .arg(&series.output_filter)
        .arg("-x")
        .arg(&series.x_expr)
        .arg("-y")
        .arg(&series.y_expr)
        .arg("-e")
        .arg(&series.opseq)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .stdin(if input.path.is_none() {
            Stdio::piped()
        } else {
            Stdio::null()
        });

    log::info!("Command #{}: {:?}", index + 1, command);
    let mut child = command.spawn().context("Failed to launch sp")?;
    let stdin_handle = if input.path.is_none() {
        let mut stdin = child.stdin.take().unwrap();
        Some(std::thread::spawn(move || {
            std::io::copy(&mut get_stdin_reader(), &mut stdin)?;
            drop(stdin);
            Ok::<_, std::io::Error>(())
        }))
    } else {
        None
    };

    Ok((child, stdin_handle))
}

fn ensure_plot_dependencies(cli: &Cli) -> anyhow::Result<()> {
    if cli.request().backend.is_gnuplot() {
        if which::which("gnuplot").is_err() {
            return Err(anyhow::anyhow!("gnuplot is not installed"));
        }
    }
    Ok(())
}

fn try_main() -> anyhow::Result<()> {
    env_logger::init();
    let cli = cli::Cli::parse_args()?;

    if matches!(cli.mode, Mode::DryRun) {
        println!("{}", cli.request().describe());
        return Ok(());
    }

    let children = (0..cli.request().data_prep.series.len())
        .map(|i| process_data_series(&cli, i))
        .collect::<Result<Vec<_>, _>>()?;

    let mut prepared = Vec::with_capacity(children.len());
    for (index, (mut child, stdin_handle)) in children.into_iter().enumerate() {
        if let Some(handle) = stdin_handle {
            handle.join().map_err(|e| anyhow::anyhow!("{e:?}"))??;
        }
        let result = child.wait().context(format!(
            "sp failed (log in {})",
            cli.get_log_path(index).display(),
        ))?;
        if !result.success() {
            return Err(anyhow::anyhow!(
                "sp failed (exit code: {:?}, log in {})",
                result.code(),
                cli.get_log_path(index).display()
            ));
        }
        prepared.push(PreparedSeries {
            index,
            spec: cli.request().data_prep.series[index].clone(),
            output_path: cli.get_output_path(index),
            log_path: cli.get_log_path(index),
        });
    }
    log::info!("Prepared data generated");

    let backend = create_backend(cli.request().backend);
    let render_plan = backend.build_render_plan(cli.request(), &prepared)?;

    if matches!(cli.mode, Mode::Prepare) {
        print!("{}", emit_prepare_report(cli.request(), &prepared));
        println!("render_plan: {}", render_plan.description);
        if let Some(path) = render_plan.artifact_path {
            println!("artifact_hint: {}", path.display());
        }
        return Ok(());
    }

    ensure_plot_dependencies(&cli)?;
    backend.execute(&render_plan, cli.request())?;
    if let Some(path) = render_plan.artifact_path {
        println!("{}", path.display());
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    match try_main() {
        Ok(()) => Ok(()),
        Err(e) => {
            handle_err(e);
            std::process::exit(1)
        }
    }
}
