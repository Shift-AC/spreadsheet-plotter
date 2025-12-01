mod cli;

use std::{
    backtrace::BacktraceStatus,
    fs::File,
    io::Write,
    process::{Child, Stdio},
};

use anyhow::Context;

use crate::cli::{Cli, get_stdin_reader};

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

fn process_data_series(
    cli: &Cli,
    index: usize,
) -> anyhow::Result<(Child, Option<std::thread::JoinHandle<std::io::Result<()>>>)>
{
    let ds = &cli.data_series[index];
    let file = ds.file;

    fn escape(s: &str) -> String {
        s.replace("'", "'\\''")
    }

    let input_str = if file == 0 {
        "".to_string()
    } else {
        format!(
            " -i '{}'",
            escape(&cli.input_paths[file - 1].display().to_string())
        )
    };
    let header_str =
        if let Some(p) = cli.header_presence.iter().find(|p| p.index == file) {
            if p.presence {
                "--header true".to_string()
            } else {
                "--header false".to_string()
            }
        } else {
            "".to_string()
        };
    let format_str =
        if let Some(p) = cli.format.iter().find(|p| p.index == file) {
            format!(" --format {}", p.format)
        } else {
            "".to_string()
        };

    let output_path = cli.get_output_path(index).display().to_string();
    let log_path = cli.get_log_path(index).display().to_string();

    let command = format!(
        "sp{}{}{} --mode dump --if '{}' --of '{}' -x '{}' -y '{}' -e '{}' > '{}' 2> '{}'",
        input_str,
        header_str,
        format_str,
        escape(&ds.ifilter),
        escape(&ds.ofilter),
        escape(&ds.xexpr),
        escape(&ds.yexpr),
        escape(&ds.opseq),
        escape(&output_path),
        escape(&log_path)
    );
    log::info!("Command #{}: {}", index + 1, command);

    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .stdin(Stdio::piped())
        .spawn()?;
    let stdin_handle = if input_str.is_empty() {
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

fn call_gnuplot(cli: &Cli) -> anyhow::Result<()> {
    let gpcmd = &cli.gpcmd;
    let out_gp_name = cli.get_temp_file_name(".gp");
    let mut out_gp = File::create(out_gp_name.clone())?;

    log::info!("gnuplot file: {}", out_gp_name.display());
    writeln!(out_gp, "{}", gpcmd)?;
    drop(out_gp);
    let mut child = std::process::Command::new("gnuplot")
        .arg("-p")
        .arg(out_gp_name)
        .spawn()?;
    let result = child.wait().context("gnuplot failed")?;
    if result.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "gnuplot failed (exit code: {:?})",
            result.code()
        ))
    }
}

fn try_main() -> anyhow::Result<()> {
    env_logger::init();
    let cli = cli::Cli::parse_args()?;

    let children = (0..cli.data_series.len())
        .map(|i| process_data_series(&cli, i))
        .collect::<Result<Vec<_>, _>>()?;

    for (index, (mut child, stdin_handle)) in children.into_iter().enumerate() {
        if let Some(handle) = stdin_handle {
            handle.join().map_err(|e| anyhow::anyhow!("{:?}", e))??;
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
    }
    log::info!("Datasheet generated");

    if cli.dry_run {
        println!("{}", cli.gpcmd);
    } else {
        call_gnuplot(&cli)?;
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
