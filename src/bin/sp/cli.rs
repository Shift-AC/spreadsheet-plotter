use std::path::PathBuf;

use clap::{CommandFactory, Parser, ValueEnum};
use spreadsheet_plotter::{
    DataFormat, DataInput, DataSeriesOptions, Expr, GnuplotTemplate, OpSeq,
    PlainSelector,
};

/// Specify whether the input file has header row
#[derive(Debug, Clone, ValueEnum)]
pub enum HeaderPresence {
    Auto,
    True,
    False,
}

/// Specify how the plotter should behave
#[derive(Debug, Clone, ValueEnum)]
pub enum Mode {
    /// Plot the temporary datasheet
    Replot,
    /// Plot the data
    Plot,
    /// Dump the processed data to stdout
    Dump,
    /// Print the SQL query to stdout
    DryRun,
}

impl Default for Mode {
    fn default() -> Self {
        Self::Plot
    }
}

/// Spreadsheet plotter: manipulate spreadsheets and produce simple plots
#[derive(Parser, Debug)]
#[command(
    version = env!("VERSION"),
    after_help = "Smart mode: run `sp` with no arguments and pipe tabular input to stdin.\n\
It will try to plot column 1 as x and column 2 as y from the incoming data.",
    term_width = 80)]
pub struct Cli {
    /// OPSEQ = {[operator](arg)}+
    ///   operator =
    ///     a(range): moving average
    ///     c: cdf
    ///     d(range): derivation over a smooth window
    ///     i: integral
    ///     m: merge (sum of y values with the same x value)
    ///     o: sort by x axis
    ///     s: step (difference of the consecutive y values)
    ///     u: unique (preserve the first occurrence of each x value)
    ///     t: transpose x and y axes
    #[arg(short = 'e', verbatim_doc_comment)]
    pub opseq: Option<OpSeq>,

    /// Input file format
    #[arg(short = 'f')]
    input_format: Option<DataFormat>,

    /// Filter to apply on the input data (SQL expression)
    #[arg(long = "if")]
    input_filter: Option<String>,

    /// Filter to apply on the output data (SQL expression)
    #[arg(long = "of")]
    output_filter: Option<String>,

    /// gnuplot code snippet to be inserted to the default template
    #[arg(short = 'g')]
    gnuplot_snippet: Option<String>,

    /// Specify whether the input file has header row
    #[arg(long, default_value = "auto")]
    header: HeaderPresence,

    /// Input file (stdin if empty)
    #[arg(short, default_value = "/dev/stdin")]
    input_path: PathBuf,

    /// Mark character that indicates a column index
    #[arg(long = "index-mark", default_value("$"))]
    index_mark: char,

    /// Specify how the plotter should behave
    #[arg(short, default_value = "plot")]
    mode: Mode,

    /// Initial X axis expression (SQL expression)
    #[arg(short, default_value("1"))]
    xexpr: String,

    /// Initial Y axis expression (SQL expression)
    #[arg(short, default_value("1"))]
    yexpr: String,
}

pub struct ParsedCli {
    pub gnuplot_cmd: String,
    pub tmp_datasheet_path: PathBuf,
    pub data_input: DataInput,
    pub selector: PlainSelector,
    pub opseq: Option<OpSeq>,
    pub mode: Mode,
}

impl Cli {
    pub fn parse_args_from<I, T>(itr: I) -> anyhow::Result<ParsedCli>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let cli = Self::parse_from(itr);
        let data_input = DataInput::new(
            cli.input_format.unwrap_or_else(|| {
                if cli.input_path == PathBuf::from("/dev/stdin") {
                    DataFormat::Explicit("csv".to_string())
                } else {
                    DataFormat::Auto
                }
            }),
            cli.input_path.display().to_string(),
            match cli.header {
                HeaderPresence::Auto => None,
                HeaderPresence::True => Some(true),
                HeaderPresence::False => Some(false),
            },
        )?;
        let tmp_datasheet_path =
            std::env::temp_dir().join(format!("{}.spdata", env!("VERSION")));

        let ds = DataSeriesOptions::from_datasheet_path(
            tmp_datasheet_path.display().to_string(),
        );

        let gnuplot_template = GnuplotTemplate::default()
            .with_terminal(spreadsheet_plotter::Terminal::Dumb(None, None))
            .with_data_series_options(vec![ds])
            .with_additional_command(cli.gnuplot_snippet);

        let xexpr = Expr::new(&cli.xexpr, cli.index_mark);
        let yexpr = Expr::new(&cli.yexpr, cli.index_mark);
        let input_filter =
            cli.input_filter.map(|s| Expr::new(&s, cli.index_mark));
        let output_filter =
            cli.output_filter.map(|s| Expr::new(&s, cli.index_mark));

        Ok(ParsedCli {
            gnuplot_cmd: gnuplot_template.to_string(),
            tmp_datasheet_path,
            data_input,
            selector: PlainSelector::new(
                xexpr,
                yexpr,
                input_filter,
                output_filter,
            )?,
            opseq: cli.opseq,
            mode: cli.mode,
        })
    }

    pub fn print_usage() -> anyhow::Result<()> {
        let mut cmd = Self::command();
        let mut stderr = std::io::stderr();
        cmd.write_help(&mut stderr)?;
        eprintln!();
        Ok(())
    }
}

impl ParsedCli {
    pub fn with_smart_mode_input(mut self) -> anyhow::Result<Self> {
        self.selector = PlainSelector::new(
            Expr::new("$1", '$'),
            Expr::new("$2", '$'),
            None,
            None,
        )?;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;

    #[test]
    fn smart_mode_uses_first_two_columns_from_csv_input() {
        let parsed = Cli::parse_args_from(["sp"])
            .unwrap()
            .with_smart_mode_input()
            .unwrap();

        let sql = format!(
            "{}{}",
            parsed.data_input.to_sql("src_tbl"),
            parsed.selector.to_preprocess_sql("src_tbl", "t0"),
        );

        assert!(sql.contains("read_csv('/dev/stdin')"));
        assert!(sql.contains("cid = 0"));
        assert!(sql.contains("cid = 1"));
        assert!(sql.contains("AS x"));
        assert!(sql.contains("AS y"));
    }
}
