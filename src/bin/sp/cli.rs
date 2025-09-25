use std::{fs, path::PathBuf};

use anyhow::Context;
use clap::{Args, Parser, ValueEnum};
use spreadsheet_plotter::{DatasheetFormat, OpSeq, Plotter};

const GNUPLOT_INIT_CMD: &str = r"
set key autotitle columnhead
set terminal dumb size `tput cols`,`echo $(($(tput lines) - 1))`
set datafile separator ','
";

#[derive(Debug, Clone)]
pub struct GnuplotCommand {
    cmd: String,
}

impl Default for GnuplotCommand {
    fn default() -> Self {
        Self::from_additional_cmd("")
    }
}

impl GnuplotCommand {
    // fit provided additional commands into hard-coded command template
    fn from_additional_cmd(additional_cmd: &str) -> Self {
        let cmd = format!(
            "{}\n{}\nplot input_file using xaxis:yaxis",
            GNUPLOT_INIT_CMD, additional_cmd
        );
        Self { cmd }
    }
    // read gnuplot command from file
    fn from_file(path: &PathBuf) -> anyhow::Result<Self> {
        let cmd = fs::read_to_string(path)
            .context("Error reading gnuplot command file")?;
        Ok(Self { cmd })
    }

    pub fn to_full_cmd(
        &self,
        datasheet_path: &str,
        xaxis: &str,
        yaxis: &str,
    ) -> String {
        let macro_cmd = format!(
            "set macro\n\
            input_file = '{}'\n\
            xaxis={}\n\
            yaxis={}\n",
            datasheet_path, xaxis, yaxis
        );

        format!("{}{}\n", macro_cmd, self.cmd)
    }
}

#[derive(Debug, Clone, ValueEnum)]
enum InputType {
    Csv,
    Lnk,
}

#[derive(Debug, Clone, ValueEnum)]
enum OutputType {
    Csv,
}

#[derive(Args, Debug)]
#[group(multiple = false, required = true)] // Exactly one allowed
pub struct Input {}

/// Spreadsheet plotter: manipulate and plot spreadsheets
#[derive(Parser, Debug)]
#[command(
    version = env!("VERSION"),
    term_width = 80)]
pub struct Cli {
    /// OPSEQ = {[operator](arg)}+
    ///   operator =
    ///     c: cdf
    ///     d(smooth-window): derivation
    ///       smooth-window: minimum x interval or derivation computation
    ///     i: integral
    ///     m: merge (sum of y values with consecutive same x value)
    ///     o: sort by x axis
    ///     r: rotate (swap x and y)
    ///     s: step (difference of the consecutive y values)
    ///     C: save current datasheet as file
    ///     O: print current datasheet and exit
    ///     P: plot current datasheet and exit
    #[arg(
        short = 'e',
        verbatim_doc_comment,
        required_unless_present = "replot",
        conflicts_with = "replot"
    )]
    pub opseq: Option<String>,

    /// Input file format
    #[arg(short = 'f', value_enum, default_value_t = InputType::Csv)]
    input_type: InputType,

    /// Output file format
    #[arg(short = 'F', value_enum, default_value_t = OutputType::Csv)]
    output_type: OutputType,

    /// gnuplot code snippet to be inserted to the default template
    #[arg(short = 'g')]
    gnuplot_snippet: Option<String>,

    /// Path to gnuplot script, overwrites -g
    #[arg(short = 'G')]
    gnuplot_path: Option<PathBuf>,

    /// Do not interpret the first line as column header (for csv inputs only)
    #[arg(short = 'H')]
    pub headless_input: bool,

    /// Input file (stdin if empty)
    #[arg(short)]
    pub input_path: Option<PathBuf>,

    /// Plot temporary datasheet generated from last execution
    #[arg(short)]
    pub replot: bool,

    /// Output cache prefix
    #[arg(long = "ocprefix", default_value = "sp-")]
    pub output_cache_prefix: String,

    /// X axis expression (mlr expression)
    #[arg(short, default_value("1"))]
    pub xexpr: String,

    /// Y axis expression (mlr expression)
    #[arg(short, default_value("1"))]
    pub yexpr: String,

    #[clap(skip)]
    pub input_format: DatasheetFormat,

    #[clap(skip)]
    pub output_format: DatasheetFormat,

    #[clap(skip)]
    pub gnuplot_cmd: String,
}

impl Cli {
    pub fn parse_args() -> anyhow::Result<Self> {
        let mut cli = Self::parse();
        match cli.input_type {
            InputType::Csv => {
                cli.input_format =
                    DatasheetFormat::new_raw("csv", !cli.headless_input)?;
            }
            InputType::Lnk => cli.input_format = DatasheetFormat::SPLNK,
        }
        match cli.output_type {
            OutputType::Csv => {
                cli.output_format = DatasheetFormat::new_raw("csv", true)?;
            }
        }
        let gnuplot_cmd = if let Some(gnuplot_path) = &cli.gnuplot_path {
            GnuplotCommand::from_file(gnuplot_path)?
        } else {
            GnuplotCommand::from_additional_cmd(
                &cli.gnuplot_snippet.as_deref().unwrap_or_default(),
            )
        };
        if !cli.replot {
            OpSeq::check_string(cli.opseq.as_deref().unwrap())?;
        }

        let tmp_datasheet_path = Plotter::get_temp_datasheet_path();
        cli.gnuplot_cmd = gnuplot_cmd.to_full_cmd(
            tmp_datasheet_path.to_str().unwrap(),
            "1",
            "2",
        );
        Ok(cli)
    }
}
