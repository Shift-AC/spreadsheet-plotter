use std::{
    backtrace::BacktraceStatus,
    io::{self, Cursor, IsTerminal, Read},
    process::{Command, Stdio, exit},
    thread::JoinHandle,
};

use anyhow::bail;
use spreadsheet_plotter::{DataSeriesSource, Plotter};
use sqlformat::{FormatOptions, QueryParams};

use crate::cli::{Cli, Mode, ParsedCli};

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

const SMART_MODE_DESC: &str = "sp was called without arguments, so it tried to \
plot stdin automatically using column 1 as x and column 2 as y.";

struct SmartModeInputSource {
    prefix: Cursor<Vec<u8>>,
    stdin: io::Stdin,
}

impl SmartModeInputSource {
    fn new(prefix: Vec<u8>, stdin: io::Stdin) -> Self {
        Self {
            prefix: Cursor::new(prefix),
            stdin,
        }
    }
}

impl Read for SmartModeInputSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let prefix_read = self.prefix.read(buf)?;
        if prefix_read != 0 {
            return Ok(prefix_read);
        }
        self.stdin.read(buf)
    }
}

fn fail_smart_mode_no_input(msg: &str) -> ! {
    let _ = Cli::print_usage();
    eprintln!("Error: {msg}");
    exit(1);
}

fn create_smart_mode_input_source() -> anyhow::Result<SmartModeInputSource> {
    let stdin = io::stdin();
    if stdin.is_terminal() {
        fail_smart_mode_no_input("No stdin input available for automatic plotting.");
    }

    let mut prefix = vec![0; 4096];
    let prefix_len = {
        let mut stdin_lock = stdin.lock();
        stdin_lock.read(&mut prefix)?
    };
    if prefix_len == 0 {
        fail_smart_mode_no_input("Stdin was empty; nothing to plot automatically.");
    }
    prefix.truncate(prefix_len);

    Ok(SmartModeInputSource::new(prefix, stdin))
}

fn pipe_stdin_to_child(
    child: &mut std::process::Child,
    mut input_source: SmartModeInputSource,
) -> anyhow::Result<JoinHandle<io::Result<()>>> {
    let mut stdin = child.stdin.take().unwrap();
    Ok(std::thread::spawn(move || {
        std::io::copy(&mut input_source, &mut stdin)?;
        drop(stdin);
        Ok::<_, io::Error>(())
    }))
}

fn run(
    cli: ParsedCli,
    smart_mode_input: Option<SmartModeInputSource>,
) -> anyhow::Result<()> {
    check_dependencies()?;
    let is_smart_mode = smart_mode_input.is_some();
    let mut smart_mode_input = smart_mode_input;

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
            let mut child = Command::new("duckdb")
                .arg("-csv")
                .arg("-bail")
                .arg("-c")
                .arg(complete_sql.clone())
                .stdin(if is_smart_mode {
                    Stdio::piped()
                } else {
                    Stdio::inherit()
                })
                .stdout(Stdio::inherit())
                .spawn()?;
            let stdin_handle =
                smart_mode_input.take().map(|input_source| {
                    pipe_stdin_to_child(&mut child, input_source)
                }).transpose()?;
            let status = child.wait()?;
            if let Some(handle) = stdin_handle {
                handle.join().map_err(|e| anyhow::anyhow!("{e:?}"))??;
            }
            if !status.success() {
                if is_smart_mode {
                    bail!(
                        "duckdb failed with {status}\n{SMART_MODE_DESC}\nOriginal SQL:\n{complete_sql}"
                    );
                } else {
                    bail!(
                        "duckdb failed with {status}\nOriginal SQL:\n{complete_sql}"
                    );
                }
            }
            return Ok(());
        }

        let mut child = Command::new("duckdb")
            .arg("-csv")
            .arg("-bail")
            .arg("-c")
            .arg(complete_sql)
            .stdin(if is_smart_mode {
                Stdio::piped()
            } else {
                Stdio::inherit()
            })
            .stdout(Stdio::piped())
            .spawn()?;
        let stdin_handle =
            smart_mode_input.take().map(|input_source| {
                pipe_stdin_to_child(&mut child, input_source)
            }).transpose()?;
        let stdout = child.stdout.take().unwrap();
        let dss = DataSeriesSource::Child(stdout);
        dss.dump(Some(cli.tmp_datasheet_path))?;
        let status = child.wait()?;
        if let Some(handle) = stdin_handle {
            handle.join().map_err(|e| anyhow::anyhow!("{e:?}"))??;
        }
        if !status.success() {
            if is_smart_mode {
                bail!("duckdb failed with {status}\n{SMART_MODE_DESC}");
            } else {
                bail!("duckdb failed with {status}");
            }
        }

        if which::which("gnuplot").is_err() {
            bail!("gnuplot is not installed");
        }
        Plotter::plot(&cli.gnuplot_cmd)?;
        if is_smart_mode {
            eprintln!("{SMART_MODE_DESC}");
        }
    }

    Ok(())
}

fn try_main() -> anyhow::Result<()> {
    env_logger::init();

    let raw_args = std::env::args_os().collect::<Vec<_>>();
    let smart_mode = if raw_args.len() == 1 {
        Some(create_smart_mode_input_source()?)
    } else {
        None
    };

    let cli = Cli::parse_args_from(raw_args)?;
    let cli = if smart_mode.is_some() {
        cli.with_smart_mode_input()?
    } else {
        cli
    };

    run(cli, smart_mode)
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::SmartModeInputSource;

    #[test]
    fn smart_mode_input_source_consumes_prefix_before_stdin() {
        let mut source = SmartModeInputSource::new(b"ab".to_vec(), std::io::stdin());
        let mut out = [0_u8; 2];

        let len = source.read(&mut out).unwrap();

        assert_eq!(len, 2);
        assert_eq!(&out, b"ab");
    }
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
