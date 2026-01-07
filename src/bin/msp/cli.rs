use std::{
    collections::HashMap,
    env,
    fmt::Display,
    hash::Hash,
    io::{Cursor, Read},
    path::PathBuf,
    str::FromStr,
    sync::{Arc, LazyLock, Mutex, OnceLock},
};

use anyhow::{Context, bail};
use clap::{Parser, ValueEnum, builder::ArgPredicate};
use rand::Rng;
use spreadsheet_plotter::{
    AxisOptions, DataFormat, DataSeriesOptions, GnuplotTemplate, PlotType,
};
use strum::Display;

#[derive(Debug, Clone)]
struct InputDataSeries {
    axis: Field<String>,
    file: Field<usize>,
    ifilter: Field<String>,
    ofilter: Field<String>,
    opseq: Field<String>,
    plot_type: Field<String>,
    style: Field<String>,
    title: Field<String>,
    xexpr: Field<String>,
    yexpr: Field<String>,
}

static DEFAULT_INPUT_DATA_SERIES: LazyLock<Arc<Mutex<InputDataSeries>>> =
    LazyLock::new(|| {
        Arc::new(Mutex::new(InputDataSeries {
            file: Field::Default,
            xexpr: Field::Default,
            yexpr: Field::Default,
            opseq: Field::Default,
            title: Field::Default,
            plot_type: Field::Default,
            axis: Field::Default,
            style: Field::Default,
            ifilter: Field::Default,
            ofilter: Field::Default,
        }))
    });

impl Default for InputDataSeries {
    fn default() -> Self {
        (*DEFAULT_INPUT_DATA_SERIES.lock().unwrap()).clone()
    }
}

impl InputDataSeries {
    const KEYS: [&str; 10] = [
        "axis", "file", "ifilter", "ofilter", "opseq", "plot", "style",
        "title", "xexpr", "yexpr",
    ];
    fn do_get_matched_key(
        abs: &str,
        match_ref: bool,
    ) -> anyhow::Result<String> {
        // try to parse a reference
        if match_ref && abs.starts_with('r') {
            let key = Self::do_get_matched_key(&abs[1..], false)?;
            return match key.as_str() {
                "file" => Err(anyhow::anyhow!("Key rfile is illegal")),
                _ => Ok(format!("r{key}")),
            };
        }
        let matched_keys = Self::KEYS
            .iter()
            .filter(|k| k.starts_with(abs))
            .map(|k| k.to_string())
            .collect::<Vec<_>>();
        if matched_keys.is_empty() {
            bail!("Unknown key: {abs}");
        } else if matched_keys.len() == 1 {
            Ok(matched_keys[0].to_string())
        } else {
            bail!(
                "Ambiguous key: '{}' (possible variants: {})",
                abs,
                matched_keys.join(", ")
            );
        }
    }

    fn get_matched_key(abs: &str) -> anyhow::Result<String> {
        Self::do_get_matched_key(abs, true)
    }
}

impl FromStr for InputDataSeries {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() < 2 {
            bail!("Empty data series string");
        }
        let options = SeparatedOptions::<String>::from_str(s)?;

        let mut ids = InputDataSeries::default();

        for part in options.opts {
            let kv = part.splitn(2, '=').collect::<Vec<_>>();
            if kv.len() != 2 {
                bail!("Invalid data series part: {part}");
            }
            let (k, v) = (kv[0], kv[1]);
            let k = InputDataSeries::get_matched_key(k)
                .context(format!("\nOriginal key-value: {k}={v}"))?;

            match k.as_str() {
                "file" => ids.file = v.parse()?,
                "axis" => ids.axis = Field::Instant(v.to_string()),
                "raxis" => ids.axis = v.parse()?,
                "ifilter" => ids.ifilter = Field::Instant(v.to_string()),
                "rifilter" => ids.ifilter = v.parse()?,
                "ofilter" => ids.ofilter = Field::Instant(v.to_string()),
                "rofilter" => ids.ofilter = v.parse()?,
                "opseq" => ids.opseq = Field::Instant(v.to_string()),
                "ropseq" => ids.opseq = v.parse()?,
                "plot" => ids.plot_type = Field::Instant(v.to_string()),
                "rplot" => ids.plot_type = v.parse()?,
                "style" => ids.style = Field::Instant(v.to_string()),
                "rstyle" => ids.style = v.parse()?,
                "title" => ids.title = Field::Instant(v.to_string()),
                "rtitle" => ids.title = v.parse()?,
                "xexpr" => ids.xexpr = Field::Instant(v.to_string()),
                "rxexpr" => ids.xexpr = v.parse()?,
                "yexpr" => ids.yexpr = Field::Instant(v.to_string()),
                "ryexpr" => ids.yexpr = v.parse()?,
                _ => bail!("Unknown key: {k}"),
            }
        }

        Ok(ids)
    }
}

#[derive(Debug, Clone)]
pub struct DataSeries {
    pub file: usize,
    pub ifilter: String,
    pub ofilter: String,
    pub xexpr: String,
    pub yexpr: String,
    pub opseq: String,
    pub title: String,
    pub style: String,
    pub plot_type: String,
    axis: String,
    pub use_x2: bool,
    pub use_y2: bool,
}

impl TryFrom<InputDataSeries> for DataSeries {
    type Error = anyhow::Error;

    fn try_from(ids: InputDataSeries) -> Result<Self, Self::Error> {
        let axis: String = ids.axis.try_into()?;
        let (use_x2, use_y2) = match axis.as_str() {
            "11" => (false, false),
            "21" => (true, false),
            "12" => (false, true),
            "22" => (true, true),
            _ => bail!("Unknown axis: {axis}"),
        };
        Ok(Self {
            file: ids.file.try_into()?,
            ifilter: ids.ifilter.try_into()?,
            ofilter: ids.ofilter.try_into()?,
            xexpr: ids.xexpr.try_into()?,
            yexpr: ids.yexpr.try_into()?,
            opseq: ids.opseq.try_into()?,
            title: ids.title.try_into()?,
            style: ids.style.try_into()?,
            plot_type: ids.plot_type.try_into()?,
            axis,
            use_x2,
            use_y2,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PlotSize {
    pub width: f32,
    pub height: f32,
}

impl FromStr for PlotSize {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        let mut parts = s.splitn(2, ',');
        let width =
            parts.next().unwrap().parse().map_err(|e| {
                anyhow::anyhow!("Failed to parse plot width: {e}")
            })?;
        let height =
            parts.next().unwrap().parse().map_err(|e| {
                anyhow::anyhow!("Failed to parse plot height: {e}")
            })?;
        Ok(Self { width, height })
    }
}

impl Display for PlotSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{},{}", self.width, self.height)
    }
}

#[derive(Debug, Clone)]
pub struct Font {
    pub family: String,
    pub size: usize,
}

impl FromStr for Font {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        let mut parts = s.splitn(2, ',');
        let family = parts.next().unwrap().to_string();
        let size =
            parts.next().unwrap().parse().map_err(|e| {
                anyhow::anyhow!("Failed to parse font size: {e}")
            })?;
        Ok(Self { family, size })
    }
}

impl Display for Font {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{},{}", self.family, self.size)
    }
}

#[derive(ValueEnum, Display, Clone, Debug, Default)]
pub enum Terminal {
    X11,
    #[default]
    Postscript,
    Dumb,
}

impl From<Terminal> for spreadsheet_plotter::Terminal {
    fn from(value: Terminal) -> Self {
        match value {
            Terminal::X11 => Self::X11,
            Terminal::Postscript => Self::Postscript,
            Terminal::Dumb => Self::Dumb(None, None),
        }
    }
}

#[derive(Clone, Debug)]
enum Field<T: Clone + std::fmt::Debug + std::fmt::Display> {
    PositiveRelative(usize),
    NegativeRelative(usize),
    Absolute(usize),
    Instant(T),
    Default,
}

macro_rules! impl_try_from_field {
    ($t:ty) => {
        impl TryFrom<Field<$t>> for $t {
            type Error = anyhow::Error;

            fn try_from(value: Field<$t>) -> Result<Self, Self::Error> {
                Ok(match value {
                    Field::Instant(instant) => instant,
                    _ => {
                        bail!(
                            "Failed to retrieve instant value from field {:?}",
                            value
                        )
                    }
                })
            }
        }
    };
}

impl_try_from_field!(usize);
impl_try_from_field!(String);

impl<T> FromStr for Field<T>
where
    T: Clone + std::fmt::Debug + std::fmt::Display,
{
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with('+') {
            let index = s.strip_prefix('+').unwrap().parse().map_err(|e| {
                anyhow::anyhow!("Failed to parse relative input index: {e}")
            })?;
            Ok(Self::PositiveRelative(index))
        } else if s.starts_with('-') {
            let index = s.strip_prefix('-').unwrap().parse().map_err(|e| {
                anyhow::anyhow!("Failed to parse relative input index: {e}")
            })?;
            if index == 0 {
                bail!("Negative relative index must be non-zero");
            }
            Ok(Self::NegativeRelative(index))
        } else {
            let index = s.parse().map_err(|e| {
                anyhow::anyhow!("Failed to parse absolute input index: {e}")
            })?;
            Ok(Self::Absolute(index))
        }
    }
}

impl<T> Display for Field<T>
where
    T: Clone + std::fmt::Debug + std::fmt::Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PositiveRelative(index) => write!(f, "+{index}"),
            Self::NegativeRelative(index) => write!(f, "-{index}"),
            Self::Absolute(index) => write!(f, "{index}"),
            Self::Default => write!(f, ""),
            Self::Instant(instant) => write!(f, "{instant}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HeaderPresence {
    pub presence: bool,
    pub index: usize,
}

impl FromStr for HeaderPresence {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let presence = match &s[..1] {
            "+" => true,
            "-" => false,
            _ => bail!("Failed to parse header presence: {s}"),
        };
        let index = s[1..].parse().map_err(|e| {
            anyhow::anyhow!("Failed to parse header index: {e}")
        })?;
        Ok(Self { presence, index })
    }
}

#[derive(Debug, Clone)]
pub struct FileFormat {
    pub format: DataFormat,
    pub index: usize,
}

impl FromStr for FileFormat {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(2, '=');
        let index =
            parts.next().unwrap().parse().map_err(|e| {
                anyhow::anyhow!("Failed to parse file index: {e}")
            })?;
        let format =
            parts.next().unwrap().parse().map_err(|e| {
                anyhow::anyhow!("Failed to parse file format: {e}")
            })?;
        Ok(Self { format, index })
    }
}

static STDIN_CONTENT: OnceLock<String> = OnceLock::new();

pub fn get_stdin_reader() -> Cursor<&'static str> {
    Cursor::new(STDIN_CONTENT.get().unwrap())
}

#[derive(Debug, Clone)]
struct TicItem(f64, String);

impl FromStr for TicItem {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(2, ':');
        let pos = parts.next().unwrap().parse()?;
        let label = parts.next().unwrap().to_string();
        Ok(Self(pos, label))
    }
}

type CustomTics = SeparatedOptions<TicItem>;

#[derive(Debug, Clone)]
struct StandardTics(spreadsheet_plotter::StandardTics);

impl FromStr for StandardTics {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use spreadsheet_plotter::StandardTics as Stics;
        if s.is_empty() {
            bail!("Empty tics options");
        }
        if !s.contains(',') {
            Ok(Self(Stics {
                range: None,
                step: s.parse()?,
            }))
        } else {
            let mut parts = s.splitn(3, ',');
            let start = parts.next().map(|s| s.parse::<f64>()).transpose()?;
            let step = parts.next().map(|s| s.parse::<f64>()).transpose()?;
            let end = parts.next().map(|s| s.parse::<f64>()).transpose()?;
            if start.is_none() || step.is_none() || end.is_none() {
                bail!("Invalid tics range with step: {s}");
            }
            Ok(Self(Stics {
                range: Some(start.unwrap()..end.unwrap()),
                step: step.unwrap(),
            }))
        }
    }
}

impl From<StandardTics> for spreadsheet_plotter::StandardTics {
    fn from(stics: StandardTics) -> Self {
        stics.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum AxisId {
    X,
    Y,
    X2,
    Y2,
}

impl FromStr for AxisId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "x" => Ok(Self::X),
            "y" => Ok(Self::Y),
            "x2" => Ok(Self::X2),
            "y2" => Ok(Self::Y2),
            _ => bail!("Failed to parse axis id: {s}"),
        }
    }
}

#[derive(Debug, Clone)]
struct Range(std::ops::Range<f64>);

impl FromStr for Range {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut iter = s.splitn(2, ':');
        let start = iter.next().unwrap().parse::<f64>()?;
        let end = iter.next().unwrap().parse::<f64>()?;
        Ok(Self(start..end))
    }
}

impl From<Range> for std::ops::Range<f64> {
    fn from(range: Range) -> Self {
        range.0
    }
}

#[derive(Debug, Clone)]
struct AxisAssociatedOption<T>
where
    T: std::fmt::Debug + Clone + FromStr,
    T::Err: Display,
{
    axis: AxisId,
    opt: T,
}

impl<T> AxisAssociatedOption<T>
where
    T: std::fmt::Debug + Clone + FromStr,
    T::Err: Display,
{
    fn unzip(self) -> (AxisId, T) {
        (self.axis, self.opt)
    }
}

impl<T> FromStr for AxisAssociatedOption<T>
where
    T: std::fmt::Debug + Clone + FromStr,
    T::Err: Display,
{
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(2, '=');
        let axis = parts.next().unwrap().parse()?;
        let opt = parts.next().unwrap().parse().map_err(|e| {
            anyhow::anyhow!("Failed to parse axis associated option: {e}")
        })?;
        Ok(Self { axis, opt })
    }
}

#[derive(Debug, Clone)]
pub struct SeparatedOptions<T>
where
    T: std::fmt::Debug + Clone + FromStr,
    T::Err: Display,
{
    opts: Vec<T>,
}

impl<T> FromStr for SeparatedOptions<T>
where
    T: std::fmt::Debug + Clone + FromStr,
    T::Err: Display,
{
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Ok(Self { opts: Vec::new() });
        }

        let (delimeter, start_pos) =
            if s.chars().next().unwrap().is_alphanumeric() {
                (',', 0)
            } else {
                (s.chars().next().unwrap(), 1)
            };
        let opts = s[start_pos..]
            .split(delimeter)
            .map(|part| {
                part.parse().map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to parse separated option: {e}\n\
                        Hint: are you sure to use '{delimeter}' as delimeter?"
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { opts })
    }
}

impl<T> SeparatedOptions<T>
where
    T: std::fmt::Debug + Clone + FromStr,
    T::Err: Display,
{
    pub fn as_slice(&self) -> &'_ [T] {
        &self.opts
    }
}

#[derive(Debug, Clone, ValueEnum)]
pub enum Mode {
    /// Plot the data
    Plot,
    /// Prepare the datasheet for plotting
    Prepare,
    /// Generate the gnuplot script only
    DryRun,
}

/// Multi-spreadsheet plotter: sp wrapper for creating complex plots with
/// multiple data series
#[derive(Parser, Debug)]
#[command(
    version = env!("VERSION"),
    term_width = 80)]
pub struct Cli {
    /// SERIES = LIST<KEY=VALUE>
    ///   LIST<ITEM>: (DELIM)<ITEM>(<DELIM><ITEM>)...
    ///     DELIM = non-alphanumeric character to be used as delimiter
    ///       (',' if the first character is alphanumeric)
    ///     ITEM = arbitrary string not containing delimeter
    ///   KEY:
    ///     axis = axis indexes to plot on ("12" for x1y2)
    ///     file = REF of data source file
    ///     ifilter = input filter expression
    ///     ofilter = output filter expression
    ///     opseq = transforms to apply on the data
    ///     plot-type = plot type of the data series
    ///     style = plotting style of the data series
    ///     title = title of the data series
    ///     xexpr = x-axis expression
    ///     yexpr = y-axis expression
    ///     rKEY = KEY's value of series[REF]
    ///       (rfile is illegal)
    /// REF = (+|-)?[num]
    ///   [num]: Absolute index (1-based),
    ///     (0 for stdin if referring to input file)
    ///   (+|-)[num]: Relative index
    ///     Current index +/- num when referring fields
    ///     Previous file index +/- num when referring files
    /// NOTE: prefix of keys is also supported (e.g. a for axis).
    /// Example:
    ///   file=0 => delimeter=',' (omitted), read from stdin
    ///   |x=$1|op=c|a=21 => delimeter='|', xexpr="$1", opseq="c", axis="21"
    ///   ,rx=1,ry=-1 =>
    ///     delimeter=',',
    ///     xexpr=series[1].xexpr,
    ///     yexpr=previous_series.yexpr
    #[arg(verbatim_doc_comment, required = true, value_name = "SERIES")]
    input_data_series: Vec<InputDataSeries>,

    /// Specify how the plotter should behave
    #[arg(short = 'm', default_value = "plot")]
    pub mode: Mode,

    /// Path to input file, specify multiple times for multiple files
    #[arg(short = 'i', value_name = "PATH")]
    pub input_paths: Vec<PathBuf>,

    /// List of presence of header in input files ([+-]INDEX)
    #[arg(short = 'H', value_name = "LIST<HEADER>", default_value = "")]
    pub header_presence: SeparatedOptions<HeaderPresence>,

    /// List of format (INDEX=EXT_NAME) of input files
    #[arg(short = 'f', value_name = "LIST<FORMAT>", default_value = "")]
    pub format: SeparatedOptions<FileFormat>,

    /// Path of the output directory [default: system temporary directory]
    #[arg(short = 'p', value_name = "PATH")]
    pub out_path: Option<PathBuf>,

    /// Default axis for all data series
    #[arg(long = "axis", value_name = "AXIS_INDEX", default_value = "11")]
    axis: String,

    /// Default input file index for all data series
    #[arg(
        long = "file",
        value_name = "REFERENCE",
        default_value = "+1",
        allow_negative_numbers = true
    )]
    file: Field<usize>,

    /// Default input filter expression for all data series
    #[arg(long = "ifilter", value_name = "FILTER", default_value = "true")]
    ifilter: String,

    /// Default output filter expression for all data series
    #[arg(long = "ofilter", value_name = "FILTER", default_value = "true")]
    ofilter: String,

    /// Default operation sequence for all data series
    #[arg(long = "opseq", default_value = "")]
    opseq: String,

    /// Default plot type for all data series
    #[arg(long = "plot", default_value = "points")]
    plot_type: String,

    /// Default additional plotting style for all data series
    #[arg(long = "style", default_value = "")]
    style: String,

    /// Default title for all data series
    #[arg(long = "title", default_value = "")]
    title: String,

    /// Default x-axis expression for all data series
    #[arg(long = "xexpr", default_value = "1")]
    xexpr: String,

    /// Default y-axis expression for all data series
    #[arg(long = "yexpr", default_value = "1")]
    yexpr: String,

    /// Additional gnuplot commands to be used before the 'plot' command
    #[arg(short = 'g', value_name = "CMD", default_value = "")]
    additional_gnuplot_cmd: String,

    /// Size of the plot (width, height)
    #[arg(long = "size", default_value = "1,0.75")]
    plot_size: PlotSize,

    /// Font to be used for all labels (family, size)
    #[arg(
        long = "font",
        default_value_if("terminal", ArgPredicate::Equals("postscript".into()), "Helvetica,20"))]
    font: Option<Font>,

    /// Position of legends
    #[arg(long = "kpos", value_name = "POSITION", default_value = "top right")]
    key_position: String,

    /// Font size to be used for all legends [default: same as --font]
    #[arg(
        long = "kfont", 
        value_name = "FONT", 
        default_value_if("terminal", ArgPredicate::Equals("postscript".into()), "Helvetica,20"))]
    key_font: Option<Font>,

    /// Terminal to be used for plotting
    #[arg(long = "term", default_value_t = Terminal::X11)]
    terminal: Terminal,

    /// Gnuplot output destination
    #[arg(
        long = "gpout",
        value_name = "PATH",
        default_value = "./msp_out.pdf"
    )]
    gp_out: String,

    /// List of axes (x|y|x2|y2) to use log scale
    #[arg(long, value_name = "LIST<AXIS>", default_value = "")]
    log: SeparatedOptions<AxisId>,

    /// List of value ranges of specified axes (AXIS=START:END)
    #[arg(long, value_name = "LIST<RANGE>", default_value = "")]
    range: SeparatedOptions<AxisAssociatedOption<Range>>,

    /// List of labels of specified axes (AXIS=CONTENT)
    #[arg(long, value_name = "LIST<LABEL>", default_value = "")]
    label: SeparatedOptions<AxisAssociatedOption<String>>,

    /// List of standard tics (STEP|START:STEP:END) of specified axes
    #[arg(long, value_name = "LIST<TICS>", default_value = "")]
    tics: SeparatedOptions<AxisAssociatedOption<StandardTics>>,

    /// List of custom tics (VALUE:LABEL) of single axis, specify
    /// multiple times for multiple axes
    #[arg(long, value_name = "LIST<CUSTOM_TICS>")]
    custom_tics: Vec<AxisAssociatedOption<CustomTics>>,

    /// Show grid with the default style
    #[arg(long)]
    grid: bool,

    #[clap(skip)]
    pub output_prefix: String,

    #[clap(skip)]
    pub gpcmd: String,

    #[clap(skip)]
    pub data_series: Vec<DataSeries>,
}

impl Cli {
    pub fn get_temp_file_name(&self, suffix: &str) -> PathBuf {
        self.out_path
            .as_ref()
            .unwrap()
            .join(format!("msp-{}-{}", self.output_prefix, suffix))
    }

    fn gen_output_prefix() -> String {
        let mut rng = rand::rng();
        const CHARSET: &[u8] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        (0..8)
            .map(|_| {
                let idx = rng.random_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect()
    }

    pub fn get_output_path(&self, index: usize) -> PathBuf {
        self.out_path.as_ref().unwrap().join(format!(
            "msp-{}-{}.csv",
            self.output_prefix,
            index + 1
        ))
    }

    pub fn get_log_path(&self, index: usize) -> PathBuf {
        self.out_path.as_ref().unwrap().join(format!(
            "msp-{}-{}.log",
            self.output_prefix,
            index + 1
        ))
    }

    fn convert_single_data_series(
        ds: &mut InputDataSeries,
        default_series: &InputDataSeries,
        converted_dss: &mut Vec<DataSeries>,
    ) -> anyhow::Result<()> {
        // use separated logic for input_file
        if matches!(ds.file, Field::Default) {
            ds.file = default_series.file.clone();
        }
        let last_index = converted_dss.last().map(|ds| ds.file).unwrap_or(0);
        match ds.file {
            Field::PositiveRelative(index) => {
                ds.file = Field::Instant(last_index + index);
            }
            Field::NegativeRelative(index) => {
                if index > last_index {
                    bail!(
                        "Referencing minus file index (required {}, base {})",
                        ds.file,
                        last_index
                    );
                }
                ds.file = Field::Instant(last_index - index);
            }
            Field::Absolute(index) => {
                ds.file = Field::Instant(index);
            }
            _ => {}
        };

        let index = converted_dss.len();
        macro_rules! convert_field {
            ($field:ident) => {
                match ds.$field {
                    Field::Default => ds.$field = default_series.$field.clone(),
                    Field::Instant(_) => {}
                    Field::Absolute(i) => {
                        if i > index {
                            bail!(
                                "Index {} larger then current index {}",
                                i,
                                index
                            );
                        }
                        ds.$field =
                            Field::Instant(converted_dss[i - 1].$field.clone());
                    }
                    Field::NegativeRelative(i) => {
                        if i >= index {
                            bail!(
                                "Index -{} is out of range (expected [1, {}])",
                                i,
                                index
                            );
                        }

                        ds.$field = Field::Instant(
                            converted_dss[index - i - 1].$field.clone(),
                        );
                    }
                    Field::PositiveRelative(_) => {
                        bail!("Forward reference is not allowed");
                    }
                }
            };
        }
        convert_field!(axis);
        convert_field!(style);
        convert_field!(title);
        convert_field!(ifilter);
        convert_field!(ofilter);
        convert_field!(xexpr);
        convert_field!(yexpr);
        convert_field!(opseq);
        convert_field!(plot_type);

        converted_dss.push(ds.clone().try_into()?);

        Ok(())
    }

    fn convert_fields(&mut self) -> anyhow::Result<()> {
        let default_series = InputDataSeries::default();
        self.data_series = self.input_data_series.iter_mut().try_fold(
            Vec::<DataSeries>::new(),
            |mut converted_dss, ds| {
                Self::convert_single_data_series(
                    ds,
                    &default_series,
                    &mut converted_dss,
                )?;

                Ok::<_, anyhow::Error>(converted_dss)
            },
        )?;
        Ok(())
    }

    fn check_file(&mut self) -> anyhow::Result<()> {
        // check if all file indexes and related files are valid
        self.data_series
            .iter()
            .zip(self.input_data_series.iter())
            .try_for_each(|(ds, ids)| {
                if ds.file == 0 {
                    return Ok(());
                }
                if self.input_paths.len() < ds.file {
                    bail!(
                        "File index {} ({}) is out of range",
                        ds.file,
                        ids.file
                    );
                }
                if !matches!(self.mode, Mode::DryRun)
                    && !self.input_paths[ds.file - 1].exists()
                {
                    bail!(
                        "File #{} ('{}', {}) does not exist",
                        ds.file,
                        ids.file,
                        self.input_paths[ds.file - 1].display(),
                    );
                }
                Ok(())
            })
    }

    fn build_stdin_content(&self) -> anyhow::Result<String> {
        // if nobody references stdin, do not bother reading it
        if self.data_series.iter().all(|ds| ds.file != 0) {
            return Ok("".to_string());
        }

        let mut stdin_content = String::new();
        std::io::stdin().read_to_string(&mut stdin_content)?;
        Ok(stdin_content)
    }

    fn build_gnuplot_cmd(&self) -> anyhow::Result<String> {
        let data_series_options = self
            .data_series
            .iter()
            .enumerate()
            .map(|(i, ds)| {
                let plot_type = if ds.plot_type.is_empty() {
                    &self.plot_type
                } else {
                    &ds.plot_type
                };
                let plot_type = match plot_type.to_ascii_lowercase().as_str() {
                    "points" => PlotType::Points(None),
                    "lines" => PlotType::Lines(None),
                    "linespoints" => PlotType::Linespoints(None, None),
                    _ => bail!("Unknown plot type '{plot_type}'"),
                };
                let style = if ds.style.is_empty() {
                    None
                } else {
                    Some(&ds.style)
                };
                let title = if ds.title.is_empty() {
                    None
                } else {
                    Some(&ds.title)
                };
                let options = DataSeriesOptions::from_datasheet_path(
                    self.get_output_path(i).display().to_string(),
                )
                .with_plot_type(plot_type)
                .with_additional_option(style)
                .with_label(title)
                .with_use_x2(ds.use_x2)
                .with_use_y2(ds.use_y2);
                Ok(options)
            })
            .collect::<Result<Vec<DataSeriesOptions>, anyhow::Error>>()?;

        fn build_axis_options(
            opt: AxisOptions,
            range: Option<&Range>,
            label: Option<&String>,
            logscale: bool,
            std_tics: Option<&StandardTics>,
            custom_tics: Option<&CustomTics>,
        ) -> anyhow::Result<AxisOptions> {
            let range = range.map(|r| r.clone().into());
            let log = if logscale { Some(10.0) } else { None };
            let opt =
                opt.with_range(range).with_label(label).with_logscale(log);
            let opt =
                opt.with_standard_tics(std_tics.map(|t| t.clone().into()));
            let opt = match custom_tics {
                Some(tics) => opt.with_custom_tics(
                    tics.as_slice()
                        .iter()
                        .map(|TicItem(x, s)| (*x, s.clone()))
                        .collect::<Vec<_>>(),
                ),
                None => opt,
            };
            Ok(opt)
        }

        let range = self
            .range
            .as_slice()
            .iter()
            .map(|o| o.clone().unzip())
            .collect::<HashMap<AxisId, Range>>();
        let label = self
            .label
            .as_slice()
            .iter()
            .map(|o| o.clone().unzip())
            .collect::<HashMap<AxisId, String>>();
        let tics = self
            .tics
            .as_slice()
            .iter()
            .map(|o| o.clone().unzip())
            .collect::<HashMap<AxisId, StandardTics>>();
        let custom_tics = self
            .custom_tics
            .as_slice()
            .iter()
            .map(|o| o.clone().unzip())
            .collect::<HashMap<AxisId, CustomTics>>();

        let xopt = build_axis_options(
            AxisOptions::new_x(),
            range.get(&AxisId::X),
            label.get(&AxisId::X),
            self.log.opts.contains(&AxisId::X),
            tics.get(&AxisId::X),
            custom_tics.get(&AxisId::X),
        )?;
        let yopt = build_axis_options(
            AxisOptions::new_y(),
            range.get(&AxisId::Y),
            label.get(&AxisId::Y),
            self.log.opts.contains(&AxisId::Y),
            tics.get(&AxisId::Y),
            custom_tics.get(&AxisId::Y),
        )?;
        let x2opt = build_axis_options(
            AxisOptions::new_x2(),
            range.get(&AxisId::X2),
            label.get(&AxisId::X2),
            self.log.opts.contains(&AxisId::X2),
            tics.get(&AxisId::X2),
            custom_tics.get(&AxisId::X2),
        )?;
        let y2opt = build_axis_options(
            AxisOptions::new_y2(),
            range.get(&AxisId::Y2),
            label.get(&AxisId::Y2),
            self.log.opts.contains(&AxisId::Y2),
            tics.get(&AxisId::Y2),
            custom_tics.get(&AxisId::Y2),
        )?;

        let font = self.font.as_ref().map(|f| (f.family.as_str(), f.size));
        let key_font = self
            .key_font
            .as_ref()
            .map(|f| (f.family.as_str(), f.size))
            .or(font);

        let gnuplot_template = GnuplotTemplate::default()
            .with_additional_command(Some(self.additional_gnuplot_cmd.clone()))
            .with_data_series_options(data_series_options)
            .with_xopt(xopt)
            .with_yopt(yopt)
            .with_x2opt(x2opt)
            .with_y2opt(y2opt)
            .with_terminal(self.terminal.clone().into())
            .with_font(font)
            .with_grid(self.grid)
            .with_key_font(key_font)
            .with_key_position(self.key_position.clone())
            .with_output(Some(&self.gp_out))
            .with_plot_size(
                self.plot_size.width as f64,
                self.plot_size.height as f64,
            );

        Ok(gnuplot_template.to_string())
    }

    /// Set default value of InputDataSeries according to command line options
    fn fill_defaults(&mut self) {
        let ds_wrap = DEFAULT_INPUT_DATA_SERIES.clone();
        let mut ds = ds_wrap.lock().unwrap();

        ds.file = self.file.clone();
        ds.ifilter = Field::Instant(self.ifilter.clone());
        ds.ofilter = Field::Instant(self.ofilter.clone());
        ds.xexpr = Field::Instant(self.xexpr.clone());
        ds.yexpr = Field::Instant(self.yexpr.clone());
        ds.opseq = Field::Instant(self.opseq.clone());
        ds.title = Field::Instant(self.title.clone());
        ds.style = Field::Instant(self.style.clone());
        ds.plot_type = Field::Instant(self.plot_type.clone());
        ds.axis = Field::Instant(self.axis.clone());
    }

    pub fn parse_args() -> anyhow::Result<Self> {
        let mut cli = Self::parse();

        if !matches!(cli.mode, Mode::DryRun) && which::which("sp").is_err() {
            bail!("sp is not installed");
        }

        cli.fill_defaults();
        cli.convert_fields()?;
        cli.check_file()?;

        cli.output_prefix = Self::gen_output_prefix();

        let stdin_content = cli.build_stdin_content()?;
        STDIN_CONTENT.get_or_init(|| stdin_content);

        if cli.out_path.is_none() {
            cli.out_path = Some(env::temp_dir());
        }

        if !matches!(cli.mode, Mode::DryRun)
            && !cli.out_path.as_ref().unwrap().is_dir()
        {
            std::fs::create_dir_all(cli.out_path.as_ref().unwrap()).context(
                format!(
                    "Failed to create output directory '{}'",
                    cli.out_path.as_ref().unwrap().display()
                ),
            )?;
        }

        if cli.key_font.is_none() {
            cli.key_font = cli.font.clone();
        }

        if !matches!(cli.mode, Mode::DryRun)
            && matches!(cli.terminal, Terminal::Postscript)
            && which::which("ps2pdf").is_err()
        {
            bail!("ps2pdf is not installed");
        }

        cli.gpcmd = cli.build_gnuplot_cmd()?;

        Ok(cli)
    }
}
