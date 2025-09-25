mod cli;

use std::{
    backtrace::BacktraceStatus,
    io::Write,
    process::{Child, Stdio},
};

use anyhow::Context;

use crate::cli::Cli;

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

fn process_data_series(cli: &Cli, index: usize) -> anyhow::Result<Child> {
    let ds = &cli.data_series[index];
    let input_str = if ds.file_index == 0 {
        "".to_string()
    } else {
        format!(" -i '{}'", cli.input_paths[ds.file_index - 1].display())
    };
    let headless_str = if cli.headless_indexes.contains(&ds.file_index) {
        " -H".to_string()
    } else {
        "".to_string()
    };
    let output_path = cli.get_output_path(index).display().to_string();
    let log_path = cli.get_log_path(index).display().to_string();

    let command = format!(
        "sp{}{} -x '{}' -y '{}' -e '{}O' > '{}' 2> '{}'",
        input_str,
        headless_str,
        ds.xexpr,
        ds.yexpr,
        ds.opseq,
        output_path,
        log_path
    );
    log::info!("Command #{}: {}", index + 1, command);

    let mut child = std::process::Command::new("bash")
        .arg("-c")
        .arg(&command)
        .stdin(Stdio::piped())
        .spawn()?;
    if !headless_str.is_empty() {
        let mut stdin = child.stdin.take().unwrap();
        std::io::copy(&mut cli.get_stdin_reader(), &mut stdin)?;
        drop(stdin);
    }

    Ok(child)
}

fn call_gnuplot(gpcmd: &str) -> anyhow::Result<()> {
    let mut child = std::process::Command::new("gnuplot")
        .stdin(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(gpcmd.as_bytes())?;
    drop(stdin);
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

    for (index, mut child) in children.into_iter().enumerate() {
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
        call_gnuplot(&cli.gpcmd)?;
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
