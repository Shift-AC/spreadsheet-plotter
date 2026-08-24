use std::{
    collections::{BTreeSet, HashMap},
    env,
    ffi::OsString,
    fmt::Display,
    hash::Hash,
    io::{Cursor, Read},
    path::PathBuf,
    str::FromStr,
    sync::{Arc, LazyLock, Mutex, OnceLock},
};

use anyhow::{Context, bail};
use clap::{
    CommandFactory, FromArgMatches, Parser, ValueEnum, parser::ValueSource,
};
use rand::Rng;
use spreadsheet_plotter::DataFormat;

use crate::spec::{
    self, AxisDimension, AxisNumberFormat, AxisRef, AxisScale, AxisSpec,
    BackendKind, BackendOptions, DataPrepSpec, EchartsBackendOptions,
    EchartsOutputMode, EchartsRuntimeMode, FontSpec, GnuplotBackendOptions,
    LayoutSpec, LegendSpec, PlotAxes, PlotSpec, RegisteredInput, RenderTarget,
    ResolvedMspRequest, SeriesAxisBinding, SeriesMark, SeriesSpec, SeriesStyle,
    StandardTickSpec, ThemeSpec, TickSpec,
};

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
struct DataSeries {
    file: usize,
    ifilter: String,
    ofilter: String,
    xexpr: String,
    yexpr: String,
    opseq: String,
    title: String,
    style: String,
    plot_type: String,
    axis: SeriesAxisBinding,
}

impl TryFrom<InputDataSeries> for DataSeries {
    type Error = anyhow::Error;

    fn try_from(ids: InputDataSeries) -> Result<Self, Self::Error> {
        let axis = String::try_from(ids.axis)?.parse()?;
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

#[derive(Debug, Clone)]
struct BackendOptionArg {
    key: String,
    value: String,
}

impl FromStr for BackendOptionArg {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(2, '=');
        let key = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
        let value = parts.next().unwrap_or_default().to_string();
        if key.is_empty() {
            bail!("Backend option key cannot be empty");
        }
        Ok(Self { key, value })
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CliAxisId(AxisRef);

impl FromStr for CliAxisId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
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

#[derive(Debug, Clone)]
struct AxisAssociatedOption<T>
where
    T: std::fmt::Debug + Clone + FromStr,
    T::Err: Display,
{
    axis: CliAxisId,
    opt: T,
}

impl<T> AxisAssociatedOption<T>
where
    T: std::fmt::Debug + Clone + FromStr,
    T::Err: Display,
{
    fn unzip(self) -> (AxisRef, T) {
        (self.axis.0, self.opt)
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

        let (delimiter, start_pos) =
            if s.chars().next().unwrap().is_alphanumeric() {
                (',', 0)
            } else {
                (s.chars().next().unwrap(), 1)
            };
        let opts = s[start_pos..]
            .split(delimiter)
            .map(|part| {
                part.parse().map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to parse separated option: {e}\n\
                        Hint: are you sure to use '{delimiter}' as delimiter?"
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
    /// Prepare data and render with the selected backend
    Plot,
    /// Prepare data and emit the resolved request plus generated artifact paths
    Prepare,
    /// Print the resolved backend-neutral request without running subprocesses
    DryRun,
}

impl From<Mode> for spec::ExecutionMode {
    fn from(value: Mode) -> Self {
        match value {
            Mode::Plot => Self::Plot,
            Mode::Prepare => Self::Prepare,
            Mode::DryRun => Self::DryRun,
        }
    }
}

#[derive(Debug, Clone, ValueEnum)]
enum CliBackendKind {
    #[value(name = "gnuplot.postscript")]
    GnuplotPostscript,
    #[value(name = "gnuplot.dumb")]
    GnuplotDumb,
    #[value(name = "gnuplot.x11")]
    GnuplotX11,
    Echarts,
}

impl From<CliBackendKind> for BackendKind {
    fn from(value: CliBackendKind) -> Self {
        match value {
            CliBackendKind::GnuplotPostscript => Self::GnuplotPostscript,
            CliBackendKind::GnuplotDumb => Self::GnuplotDumb,
            CliBackendKind::GnuplotX11 => Self::GnuplotX11,
            CliBackendKind::Echarts => Self::Echarts,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum CliOptionId {
    Style,
    PlotTitle,
    MaxPoints,
    NumberFormat,
}

impl CliOptionId {
    const fn cli_flag(self) -> &'static str {
        match self {
            Self::Style => "--style",
            Self::PlotTitle => "--plot-title",
            Self::MaxPoints => "--max-points",
            Self::NumberFormat => "--number-format",
        }
    }
}

#[derive(Debug, Clone, Default)]
struct CliUsage {
    global_style: bool,
    plot_title: bool,
    max_points: bool,
}

impl CliUsage {
    fn from_matches(matches: &clap::ArgMatches) -> Self {
        Self {
            global_style: matches.value_source("style")
                == Some(ValueSource::CommandLine),
            plot_title: matches.value_source("plot_title")
                == Some(ValueSource::CommandLine),
            max_points: matches.value_source("max_points")
                == Some(ValueSource::CommandLine),
        }
    }
}

const GNUPLOT_BACKENDS: &[BackendKind] = &[
    BackendKind::GnuplotPostscript,
    BackendKind::GnuplotDumb,
    BackendKind::GnuplotX11,
];
const ECHARTS_BACKENDS: &[BackendKind] = &[BackendKind::Echarts];

// New backend-sensitive CLI options must be added to this registry with an
// empty support set first. Support is opt-in: only mark a backend here when the
// implementation is intentionally wired, and let validation fail closed until
// that happens.
fn cli_option_support(option: CliOptionId) -> &'static [BackendKind] {
    match option {
        CliOptionId::Style => GNUPLOT_BACKENDS,
        CliOptionId::PlotTitle => ECHARTS_BACKENDS,
        CliOptionId::MaxPoints => ECHARTS_BACKENDS,
        CliOptionId::NumberFormat => ECHARTS_BACKENDS,
    }
}

/// Multi-spreadsheet plotter: prepare multiple series with `sp` and render them
/// with pluggable backends.
#[derive(Parser, Debug)]
#[command(version = env!("VERSION"), term_width = 80)]
pub struct Cli {
    /// SERIES = LIST<KEY=VALUE>
    ///   LIST<ITEM>: (DELIM)<ITEM>(<DELIM><ITEM>)...
    ///     DELIM = non-alphanumeric character to be used as delimiter
    ///       (',' if the first character is alphanumeric)
    ///     ITEM = arbitrary string not containing delimiter
    ///   KEY:
    ///     axis = axis binding to plot on ("x1y2" for x1y2, legacy "12" also works)
    ///     file = REF of data source file
    ///     ifilter = input filter expression
    ///     ofilter = output filter expression
    ///     opseq = transforms to apply on the data
    ///     plot = plot mark of the data series
    ///     style = backend-specific series style hint
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
    #[arg(verbatim_doc_comment, required = true, value_name = "SERIES")]
    input_data_series: Vec<InputDataSeries>,

    /// Specify how msp should behave
    #[arg(short = 'm', default_value = "plot")]
    pub mode: Mode,

    /// Rendering backend
    #[arg(short = 'b', long = "backend", default_value = "gnuplot.postscript")]
    backend: CliBackendKind,

    /// Path to input file, specify multiple times for multiple files
    #[arg(short = 'i', value_name = "PATH")]
    pub input_paths: Vec<PathBuf>,

    /// List of presence of header in input files ([+-]INDEX)
    #[arg(short = 'H', value_name = "LIST<HEADER>", default_value = "")]
    pub header_presence: SeparatedOptions<HeaderPresence>,

    /// List of format (INDEX=EXT_NAME) of input files
    #[arg(short = 'f', value_name = "LIST<FORMAT>", default_value = "")]
    pub format: SeparatedOptions<FileFormat>,

    /// Working directory for generated intermediate files
    #[arg(short = 'p', value_name = "PATH")]
    pub work_dir: Option<PathBuf>,

    /// Final backend output path
    #[arg(long = "out", value_name = "PATH")]
    pub out: Option<PathBuf>,

    /// Output format hint for the selected backend
    #[arg(long = "format", value_name = "NAME")]
    pub output_format: Option<String>,

    /// Open the generated artifact when supported by the backend
    #[arg(long = "open")]
    pub open: bool,

    /// Default axis for all data series
    #[arg(long = "axis", value_name = "AXIS_BINDING", default_value = "x1y1")]
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

    /// Default plot mark for all data series
    #[arg(long = "plot", default_value = "points")]
    plot_type: String,

    /// Default backend-specific style hint for all data series
    #[arg(long = "style", default_value = "")]
    style: String,

    /// Default title for all data series
    #[arg(long = "title", default_value = "")]
    title: String,

    /// Plot title for backends that support a chart-level title
    #[arg(long = "plot-title", default_value = "")]
    plot_title: String,

    /// Default x-axis expression for all data series
    #[arg(long = "xexpr", default_value = "1")]
    xexpr: String,

    /// Default y-axis expression for all data series
    #[arg(long = "yexpr", default_value = "1")]
    yexpr: String,

    /// Backend-specific option (KEY=VALUE), repeat as needed
    #[arg(long = "backend-opt", value_name = "KEY=VALUE")]
    backend_opt: Vec<BackendOptionArg>,

    /// Maximum number of points embedded per series for the echarts backend
    #[arg(long = "max-points", value_name = "COUNT")]
    max_points: Option<usize>,

    /// Size of the plot (width, height)
    #[arg(long = "size", default_value = "1,1")]
    plot_size: PlotSize,

    /// Font to be used for labels (family, size)
    #[arg(long = "font")]
    font: Option<Font>,

    /// Position of legends
    #[arg(long = "kpos", value_name = "POSITION", default_value = "top right")]
    key_position: String,

    /// Font to be used for legends [default: same as --font]
    #[arg(long = "kfont", value_name = "FONT")]
    key_font: Option<Font>,

    /// List of axes (x1|x2|y1|y2|y3...) to use log scale
    #[arg(long, value_name = "LIST<AXIS>", default_value = "")]
    log: SeparatedOptions<CliAxisId>,

    /// List of value ranges of specified axes (AXIS=START:END)
    #[arg(long, value_name = "LIST<RANGE>", default_value = "")]
    range: SeparatedOptions<AxisAssociatedOption<Range>>,

    /// List of labels of specified axes (AXIS=CONTENT)
    #[arg(long, value_name = "LIST<LABEL>", default_value = "")]
    label: SeparatedOptions<AxisAssociatedOption<String>>,

    /// List of number formats of specified axes (AXIS=plain|suffix|scientific)
    #[arg(
        long = "number-format",
        value_name = "LIST<NUMBER_FORMAT>",
        default_value = ""
    )]
    number_format: SeparatedOptions<AxisAssociatedOption<AxisNumberFormat>>,

    /// List of standard tics (STEP|START:STEP:END) of specified axes
    #[arg(long, value_name = "LIST<TICS>", default_value = "")]
    tics: SeparatedOptions<AxisAssociatedOption<StandardTics>>,

    /// List of custom tics (VALUE:LABEL) of single axis, specify multiple times
    #[arg(long, value_name = "LIST<CUSTOM_TICS>")]
    custom_tics: Vec<AxisAssociatedOption<CustomTics>>,

    /// Show grid with the default style
    #[arg(long)]
    grid: bool,

    #[clap(skip)]
    pub output_prefix: String,

    #[clap(skip)]
    data_series: Vec<DataSeries>,

    #[clap(skip)]
    pub request: Option<ResolvedMspRequest>,
}

impl Cli {
    fn supported_backends_text(backends: &'static [BackendKind]) -> String {
        if backends.is_empty() {
            return "none".to_string();
        }
        backends
            .iter()
            .map(|backend| backend.display_name())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn get_output_path(&self, index: usize) -> PathBuf {
        self.work_dir.as_ref().unwrap().join(format!(
            "msp-{}-{}.csv",
            self.output_prefix,
            index + 1
        ))
    }

    pub fn get_log_path(&self, index: usize) -> PathBuf {
        self.work_dir.as_ref().unwrap().join(format!(
            "msp-{}-{}.log",
            self.output_prefix,
            index + 1
        ))
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

    fn convert_single_data_series(
        ds: &mut InputDataSeries,
        default_series: &InputDataSeries,
        converted_dss: &mut Vec<DataSeries>,
    ) -> anyhow::Result<()> {
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
        match ds.axis {
            Field::Default => ds.axis = default_series.axis.clone(),
            Field::Instant(_) => {}
            Field::Absolute(i) => {
                if i > index {
                    bail!("Index {} larger then current index {}", i, index);
                }
                ds.axis = Field::Instant(converted_dss[i - 1].axis.to_string());
            }
            Field::NegativeRelative(i) => {
                if i >= index {
                    bail!(
                        "Index -{} is out of range (expected [1, {}])",
                        i,
                        index
                    );
                }
                ds.axis = Field::Instant(
                    converted_dss[index - i - 1].axis.to_string(),
                );
            }
            Field::PositiveRelative(_) => {
                bail!("Forward reference is not allowed");
            }
        }
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
        if self.data_series.iter().all(|ds| ds.file != 0) {
            return Ok(String::new());
        }

        let mut stdin_content = String::new();
        std::io::stdin().read_to_string(&mut stdin_content)?;
        Ok(stdin_content)
    }

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

    fn parse_mark(s: &str) -> anyhow::Result<(SeriesMark, Option<usize>)> {
        let normalized = s.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "points" => Ok((SeriesMark::Points, None)),
            "lines" => Ok((SeriesMark::Lines, None)),
            "linespoints" => Ok((SeriesMark::LinesPoints, None)),
            "bar" => Ok((SeriesMark::Bar, None)),
            "boxplot" => Ok((SeriesMark::Boxplot, None)),
            _ => {
                let Some(group) = normalized.strip_prefix("boxplot") else {
                    bail!("Unknown plot type '{s}'");
                };
                let group = group.parse::<usize>().with_context(|| {
                    format!(
                        "Invalid boxplot group in plot type '{s}'; expected boxplot<N>"
                    )
                })?;
                if group == 0 {
                    bail!(
                        "Invalid boxplot group in plot type '{s}'; group index must be >= 1"
                    );
                }
                Ok((SeriesMark::Boxplot, Some(group)))
            }
        }
    }

    fn build_inputs(&self) -> Vec<RegisteredInput> {
        let mut inputs = Vec::new();
        if self.data_series.iter().any(|ds| ds.file == 0) {
            inputs.push(RegisteredInput {
                index: 0,
                path: None,
                header_presence: self
                    .header_presence
                    .as_slice()
                    .iter()
                    .find(|item| item.index == 0)
                    .map(|item| item.presence),
                format: self
                    .format
                    .as_slice()
                    .iter()
                    .find(|item| item.index == 0)
                    .map(|item| item.format.clone()),
            });
        }
        inputs.extend(self.input_paths.iter().enumerate().map(
            |(idx, path)| {
                RegisteredInput {
                    index: idx + 1,
                    path: Some(path.clone()),
                    header_presence: self
                        .header_presence
                        .as_slice()
                        .iter()
                        .find(|item| item.index == idx + 1)
                        .map(|item| item.presence),
                    format: self
                        .format
                        .as_slice()
                        .iter()
                        .find(|item| item.index == idx + 1)
                        .map(|item| item.format.clone()),
                }
            },
        ));
        inputs
    }

    fn build_series(&self) -> anyhow::Result<Vec<SeriesSpec>> {
        self.data_series
            .iter()
            .map(|ds| {
                let (mark, boxplot_group) = Self::parse_mark(&ds.plot_type)?;
                Ok(SeriesSpec {
                    axis_binding: ds.axis,
                    input_ref: ds.file,
                    input_filter: ds.ifilter.clone(),
                    output_filter: ds.ofilter.clone(),
                    opseq: ds.opseq.clone(),
                    x_expr: ds.xexpr.clone(),
                    y_expr: ds.yexpr.clone(),
                    mark,
                    boxplot_group,
                    name: if ds.title.is_empty() {
                        None
                    } else {
                        Some(ds.title.clone())
                    },
                    style: SeriesStyle {
                        raw: if ds.style.is_empty() {
                            None
                        } else {
                            Some(ds.style.clone())
                        },
                    },
                })
            })
            .collect()
    }

    fn validate_boxplot_groups(
        &self,
        request: &ResolvedMspRequest,
    ) -> anyhow::Result<()> {
        let mut grouped_axes = HashMap::<usize, SeriesAxisBinding>::new();
        for series in &request.data_prep.series {
            let Some(group) = series.boxplot_group else {
                continue;
            };
            let existing =
                grouped_axes.entry(group).or_insert(series.axis_binding);
            if *existing != series.axis_binding {
                bail!(
                    "boxplot group {} mixes axis bindings ({} and {}); series with the same boxplot number must use the same axis",
                    group,
                    existing,
                    series.axis_binding
                );
            }
        }
        Ok(())
    }

    fn axis_spec_from_maps(
        &self,
        axis_id: AxisRef,
        number_format: &HashMap<AxisRef, AxisNumberFormat>,
        range: &HashMap<AxisRef, std::ops::Range<f64>>,
        label: &HashMap<AxisRef, String>,
        tics: &HashMap<AxisRef, StandardTickSpec>,
        custom_tics: &HashMap<AxisRef, Vec<(f64, String)>>,
    ) -> AxisSpec {
        AxisSpec {
            scale: if self.log.as_slice().iter().any(|axis| axis.0 == axis_id) {
                AxisScale::Log10
            } else {
                AxisScale::Linear
            },
            number_format: number_format
                .get(&axis_id)
                .copied()
                .unwrap_or_default(),
            range: range.get(&axis_id).cloned(),
            label: label.get(&axis_id).cloned(),
            ticks: TickSpec {
                major: tics.get(&axis_id).cloned(),
                custom: custom_tics.get(&axis_id).cloned().unwrap_or_default(),
            },
        }
    }

    fn build_plot_spec(&self) -> PlotSpec {
        let number_format = self
            .number_format
            .as_slice()
            .iter()
            .map(|o| o.clone().unzip())
            .collect::<HashMap<_, _>>();
        let range = self
            .range
            .as_slice()
            .iter()
            .map(|o| {
                let (axis, range) = o.clone().unzip();
                (axis, range.0)
            })
            .collect::<HashMap<_, _>>();
        let label = self
            .label
            .as_slice()
            .iter()
            .map(|o| {
                let (axis, label) = o.clone().unzip();
                (axis, label)
            })
            .collect::<HashMap<_, _>>();
        let tics = self
            .tics
            .as_slice()
            .iter()
            .map(|o| {
                let (axis, tics) = o.clone().unzip();
                (
                    axis,
                    StandardTickSpec {
                        range: tics.0.range,
                        step: tics.0.step,
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        let custom_tics = self
            .custom_tics
            .as_slice()
            .iter()
            .map(|o| {
                let (axis, tics) = o.clone().unzip();
                (
                    axis,
                    tics.as_slice()
                        .iter()
                        .map(|TicItem(x, label)| (*x, label.clone()))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<HashMap<_, _>>();
        let font = self.font.as_ref().map(|font| FontSpec {
            family: font.family.clone(),
            size: font.size,
        });
        let key_font = self
            .key_font
            .as_ref()
            .map(|font| FontSpec {
                family: font.family.clone(),
                size: font.size,
            })
            .or_else(|| font.clone());
        let mut configured_axes = BTreeSet::from([
            AxisRef::x(1),
            AxisRef::x(2),
            AxisRef::y(1),
            AxisRef::y(2),
        ]);
        configured_axes.extend(
            self.data_series.iter().flat_map(|series| {
                [series.axis.x_axis(), series.axis.y_axis()]
            }),
        );
        configured_axes.extend(self.log.as_slice().iter().map(|axis| axis.0));
        configured_axes.extend(number_format.keys().copied());
        configured_axes.extend(range.keys().copied());
        configured_axes.extend(label.keys().copied());
        configured_axes.extend(tics.keys().copied());
        configured_axes.extend(custom_tics.keys().copied());
        let mut axes = PlotAxes::default();
        for axis_id in configured_axes {
            axes.insert(
                axis_id,
                self.axis_spec_from_maps(
                    axis_id,
                    &number_format,
                    &range,
                    &label,
                    &tics,
                    &custom_tics,
                ),
            );
        }

        PlotSpec {
            title: if self.plot_title.is_empty() {
                None
            } else {
                Some(self.plot_title.clone())
            },
            layout: LayoutSpec {
                width: self.plot_size.width,
                height: self.plot_size.height,
            },
            theme: ThemeSpec { font },
            legend: LegendSpec {
                position: self.key_position.clone(),
                font: key_font,
            },
            axes,
            grid: self.grid,
        }
    }

    fn validate_supported_cli_option(
        &self,
        request: &ResolvedMspRequest,
        option: CliOptionId,
    ) -> anyhow::Result<()> {
        let supported_backends = cli_option_support(option);
        if supported_backends.contains(&request.backend) {
            return Ok(());
        }
        if supported_backends.is_empty() {
            bail!("{} is not supported by any backend yet", option.cli_flag());
        }
        bail!(
            "backend '{}' does not support {}; supported backends: {}",
            request.backend.display_name(),
            option.cli_flag(),
            Self::supported_backends_text(supported_backends)
        );
    }

    fn series_uses_style(series: &DataSeries) -> bool {
        !series.style.is_empty()
    }

    fn uses_plot_title(&self, usage: &CliUsage) -> bool {
        usage.plot_title && !self.plot_title.is_empty()
    }

    fn uses_global_style(&self, usage: &CliUsage) -> bool {
        usage.global_style && !self.style.is_empty()
    }

    fn used_axis_option_ids(&self) -> impl Iterator<Item = AxisRef> + '_ {
        self.log
            .as_slice()
            .iter()
            .map(|axis| axis.0)
            .chain(self.range.as_slice().iter().map(|item| item.axis.0))
            .chain(self.label.as_slice().iter().map(|item| item.axis.0))
            .chain(self.number_format.as_slice().iter().map(|item| item.axis.0))
            .chain(self.tics.as_slice().iter().map(|item| item.axis.0))
            .chain(self.custom_tics.as_slice().iter().map(|item| item.axis.0))
    }

    fn validate_backend_support(
        &self,
        request: &ResolvedMspRequest,
        usage: &CliUsage,
    ) -> anyhow::Result<()> {
        if self.uses_global_style(usage)
            || self.data_series.iter().any(Self::series_uses_style)
        {
            self.validate_supported_cli_option(request, CliOptionId::Style)?;
        }
        if self.uses_plot_title(usage) {
            self.validate_supported_cli_option(
                request,
                CliOptionId::PlotTitle,
            )?;
        }
        if usage.max_points {
            self.validate_supported_cli_option(
                request,
                CliOptionId::MaxPoints,
            )?;
        }
        if !self.number_format.as_slice().is_empty() {
            self.validate_supported_cli_option(
                request,
                CliOptionId::NumberFormat,
            )?;
        }

        if request.backend.is_echarts() {
            return Ok(());
        }

        if let Some(series) = request
            .data_prep
            .series
            .iter()
            .find(|series| matches!(series.mark, SeriesMark::Bar))
        {
            bail!(
                "gnuplot backend does not support plot=bar on series '{}'; use --backend echarts",
                series.name.as_deref().unwrap_or("unnamed")
            );
        }

        let unsupported_series_axis = request
            .data_prep
            .series
            .iter()
            .find(|series| series.axis_binding.y_index > 2)
            .map(|series| series.axis_binding);
        let unsupported_config_axis =
            self.used_axis_option_ids().find(|axis| {
                matches!(axis.dimension, AxisDimension::Y) && axis.index > 2
            });

        if let Some(axis) = unsupported_series_axis {
            bail!(
                "gnuplot backend supports only y1 and y2; series requests {}, use --backend echarts for y3+",
                axis
            );
        }
        if let Some(axis) = unsupported_config_axis {
            bail!(
                "gnuplot backend supports only y1 and y2; axis option '{}' requires --backend echarts for y3+",
                axis
            );
        }
        Ok(())
    }

    fn build_backend_options(&self) -> anyhow::Result<BackendOptions> {
        let opts = self
            .backend_opt
            .iter()
            .map(|opt| (opt.key.as_str(), opt.value.as_str()))
            .collect::<HashMap<_, _>>();
        if opts.contains_key("terminal") {
            bail!(
                "gnuplot terminal is now selected by --backend (for example: gnuplot.postscript, gnuplot.dumb, gnuplot.x11); --backend-opt terminal=... is no longer supported"
            );
        }
        let backend = BackendKind::from(self.backend.clone());
        if backend.is_gnuplot() {
            for key in opts.keys() {
                if !matches!(*key, "snippet") {
                    bail!("Unknown gnuplot backend option '{key}'");
                }
            }
            return Ok(BackendOptions::Gnuplot(GnuplotBackendOptions {
                pre_plot_snippet: opts.get("snippet").map(|s| s.to_string()),
            }));
        }

        match backend {
            BackendKind::Echarts => {
                for key in opts.keys() {
                    if !matches!(
                        *key,
                        "theme" | "max-points" | "mode" | "runtime"
                    ) {
                        bail!("Unknown echarts backend option '{key}'");
                    }
                }
                let backend_max_points = opts
                    .get("max-points")
                    .map(|s| {
                        s.parse::<usize>().map_err(|e| {
                            anyhow::anyhow!(
                                "Invalid echarts max-points value '{}': {}",
                                s,
                                e
                            )
                        })
                    })
                    .transpose()?;
                let output_mode = match opts.get("mode").copied() {
                    None | Some("page") => EchartsOutputMode::Page,
                    Some("embed") => EchartsOutputMode::Embed,
                    Some(mode) => bail!("Unknown echarts mode '{mode}'"),
                };
                let runtime_mode = match opts.get("runtime").copied() {
                    None | Some("cdn") => EchartsRuntimeMode::Cdn,
                    Some("external") => EchartsRuntimeMode::External,
                    Some(runtime) => {
                        bail!("Unknown echarts runtime '{runtime}'")
                    }
                };
                let max_points =
                    self.max_points.or(backend_max_points).unwrap_or(200);
                Ok(BackendOptions::Echarts(EchartsBackendOptions {
                    theme: opts.get("theme").map(|s| s.to_string()),
                    max_points: Some(max_points),
                    output_mode,
                    runtime_mode,
                }))
            }
            BackendKind::GnuplotPostscript
            | BackendKind::GnuplotDumb
            | BackendKind::GnuplotX11 => unreachable!(),
        }
    }

    fn build_render_target(&self) -> RenderTarget {
        RenderTarget {
            work_dir: self.work_dir.as_ref().unwrap().clone(),
            out: self.out.clone(),
            format_hint: self.output_format.clone(),
            open: self.open,
        }
    }

    fn build_resolved_request(&self) -> anyhow::Result<ResolvedMspRequest> {
        Ok(ResolvedMspRequest {
            mode: self.mode.clone().into(),
            backend: self.backend.clone().into(),
            data_prep: DataPrepSpec {
                inputs: self.build_inputs(),
                series: self.build_series()?,
            },
            plot: self.build_plot_spec(),
            render_target: self.build_render_target(),
            backend_options: self.build_backend_options()?,
        })
    }

    fn finish_parse(&mut self, usage: CliUsage) -> anyhow::Result<()> {
        let request = self.build_resolved_request()?;
        self.validate_boxplot_groups(&request)?;
        self.validate_backend_support(&request, &usage)?;
        self.request = Some(request);
        Ok(())
    }

    fn try_parse_cli<I, T>(args: I) -> anyhow::Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let matches = Self::command().try_get_matches_from(args)?;
        let usage = CliUsage::from_matches(&matches);
        let mut cli = Self::from_arg_matches(&matches)?;

        if !matches!(cli.mode, Mode::DryRun) && which::which("sp").is_err() {
            bail!("sp is not installed");
        }

        cli.fill_defaults();
        cli.convert_fields()?;
        cli.check_file()?;
        cli.output_prefix = Self::gen_output_prefix();

        let stdin_content = cli.build_stdin_content()?;
        STDIN_CONTENT.get_or_init(|| stdin_content);

        if cli.work_dir.is_none() {
            cli.work_dir = Some(env::temp_dir());
        }
        if !matches!(cli.mode, Mode::DryRun)
            && !cli.work_dir.as_ref().unwrap().is_dir()
        {
            std::fs::create_dir_all(cli.work_dir.as_ref().unwrap()).context(
                format!(
                    "Failed to create work directory '{}'",
                    cli.work_dir.as_ref().unwrap().display()
                ),
            )?;
        }

        cli.finish_parse(usage)?;
        Ok(cli)
    }

    pub fn parse_args() -> anyhow::Result<Self> {
        Self::try_parse_cli(env::args_os())
    }

    pub fn request(&self) -> &ResolvedMspRequest {
        self.request.as_ref().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use clap::Parser;

    use super::{Cli, Field, InputDataSeries, SeparatedOptions};
    use crate::spec::{
        AxisNumberFormat, AxisRef, BackendKind, BackendOptions,
        EchartsOutputMode, EchartsRuntimeMode, SeriesAxisBinding, SeriesMark,
    };

    #[test]
    fn series_reference_resolution_still_works() {
        let default = InputDataSeries {
            axis: Field::Instant("x1y1".to_string()),
            file: Field::Instant(1),
            ifilter: Field::Instant("true".to_string()),
            ofilter: Field::Instant("true".to_string()),
            opseq: Field::Instant(String::new()),
            plot_type: Field::Instant("points".to_string()),
            style: Field::Instant(String::new()),
            title: Field::Instant(String::new()),
            xexpr: Field::Instant("1".to_string()),
            yexpr: Field::Instant("1".to_string()),
        };
        let mut first = ",xexpr=$1,yexpr=$2,title=a".parse().unwrap();
        let mut second = ",rx=1,ry=1,rtitle=1".parse().unwrap();
        let mut out = Vec::new();

        Cli::convert_single_data_series(&mut first, &default, &mut out)
            .unwrap();
        Cli::convert_single_data_series(&mut second, &default, &mut out)
            .unwrap();

        assert_eq!(out[1].xexpr, "$1");
        assert_eq!(out[1].yexpr, "$2");
        assert_eq!(out[1].title, "a");
    }

    #[test]
    fn separated_options_support_custom_delimiter() {
        let opts = SeparatedOptions::<String>::from_str("|a|b|c").unwrap();
        assert_eq!(opts.as_slice(), &["a", "b", "c"]);
    }

    #[test]
    fn data_series_maps_to_backend_neutral_fields() {
        let mut cli = Cli::parse_from([
            "msp",
            ",xexpr=$1,yexpr=$2,plot=linespoints,axis=22,title=test",
            "--backend",
            "echarts",
        ]);
        cli.fill_defaults();
        cli.convert_fields().unwrap();
        cli.work_dir = Some(std::env::temp_dir());
        let request = cli.build_resolved_request().unwrap();
        let series = &request.data_prep.series[0];
        assert_eq!(series.axis_binding, SeriesAxisBinding::new(2, 2));
        assert!(matches!(series.mark, SeriesMark::LinesPoints));
        assert_eq!(series.name.as_deref(), Some("test"));
    }

    #[test]
    fn data_series_parses_bar_mark() {
        let mut cli = Cli::parse_from([
            "msp",
            ",xexpr=year,yexpr=value,plot=bar,title=Revenue",
            "--backend",
            "echarts",
        ]);
        cli.fill_defaults();
        cli.convert_fields().unwrap();
        cli.work_dir = Some(std::env::temp_dir());

        let request = cli.build_resolved_request().unwrap();
        let series = &request.data_prep.series[0];
        assert!(matches!(series.mark, SeriesMark::Bar));
        assert_eq!(series.boxplot_group, None);
        assert_eq!(series.name.as_deref(), Some("Revenue"));
    }

    #[test]
    fn data_series_accepts_named_axis_binding() {
        let mut cli = Cli::parse_from([
            "msp",
            ",xexpr=$1,yexpr=$2,axis=x1y3",
            "--backend",
            "echarts",
        ]);
        cli.fill_defaults();
        cli.convert_fields().unwrap();
        cli.work_dir = Some(std::env::temp_dir());

        let request = cli.build_resolved_request().unwrap();
        let series = &request.data_prep.series[0];
        assert_eq!(series.axis_binding, SeriesAxisBinding::new(1, 3));
        assert_eq!(request.plot.axes.axis(AxisRef::y(3)).unwrap().label, None);
    }

    #[test]
    fn data_series_keeps_legacy_axis_binding() {
        let mut cli = Cli::parse_from([
            "msp",
            ",xexpr=$1,yexpr=$2,axis=12",
            "--backend",
            "echarts",
        ]);
        cli.fill_defaults();
        cli.convert_fields().unwrap();
        cli.work_dir = Some(std::env::temp_dir());

        let request = cli.build_resolved_request().unwrap();
        assert_eq!(
            request.data_prep.series[0].axis_binding,
            SeriesAxisBinding::new(1, 2)
        );
    }

    #[test]
    fn data_series_parses_numbered_boxplot_group() {
        let mut cli = Cli::parse_from([
            "msp",
            ",xexpr=metric,yexpr=value,plot=boxplot2,title=Latency",
            "--backend",
            "echarts",
        ]);
        cli.fill_defaults();
        cli.convert_fields().unwrap();
        cli.work_dir = Some(std::env::temp_dir());

        let request = cli.build_resolved_request().unwrap();
        let series = &request.data_prep.series[0];
        assert!(matches!(series.mark, SeriesMark::Boxplot));
        assert_eq!(series.boxplot_group, Some(2));
        assert_eq!(series.name.as_deref(), Some("Latency"));
    }

    #[test]
    fn boxplot_group_rejects_mixed_axis_bindings() {
        let mut cli = Cli::parse_from([
            "msp",
            ",xexpr=metric,yexpr=value,plot=boxplot1,axis=x1y1",
            ",xexpr=metric,yexpr=value,plot=boxplot1,axis=x1y2",
            "--backend",
            "echarts",
        ]);
        cli.fill_defaults();
        cli.convert_fields().unwrap();
        cli.work_dir = Some(std::env::temp_dir());

        let request = cli.build_resolved_request().unwrap();
        let err = cli.validate_boxplot_groups(&request).unwrap_err();

        assert!(
            err.to_string()
                .contains("boxplot group 1 mixes axis bindings")
        );
    }

    #[test]
    fn plot_spec_supports_y3_axis_options() {
        let mut cli = Cli::parse_from([
            "msp",
            ",xexpr=$1,yexpr=$2,axis=x1y3",
            "--backend",
            "echarts",
            "--label",
            "y3=Latency",
            "--range",
            "y3=0:10",
            "--log",
            "y3",
        ]);
        cli.fill_defaults();
        cli.convert_fields().unwrap();
        cli.work_dir = Some(std::env::temp_dir());

        let request = cli.build_resolved_request().unwrap();
        let axis = request.plot.axes.axis(AxisRef::y(3)).unwrap();
        assert_eq!(axis.label.as_deref(), Some("Latency"));
        assert_eq!(
            axis.range.as_ref().map(|r| (r.start, r.end)),
            Some((0.0, 10.0))
        );
        assert_eq!(axis.scale, crate::spec::AxisScale::Log10);
    }

    #[test]
    fn plot_spec_supports_per_axis_number_format() {
        let mut cli = Cli::parse_from([
            "msp",
            ",xexpr=$1,yexpr=$2,axis=x1y2",
            "--backend",
            "echarts",
            "--number-format",
            "y1=suffix,y2=scientific",
        ]);
        cli.fill_defaults();
        cli.convert_fields().unwrap();
        cli.work_dir = Some(std::env::temp_dir());

        let request = cli.build_resolved_request().unwrap();
        assert_eq!(
            request.plot.axes.axis(AxisRef::y(1)).unwrap().number_format,
            AxisNumberFormat::Suffix
        );
        assert_eq!(
            request.plot.axes.axis(AxisRef::y(2)).unwrap().number_format,
            AxisNumberFormat::Scientific
        );
        assert_eq!(
            request.plot.axes.axis(AxisRef::x(1)).unwrap().number_format,
            AxisNumberFormat::Plain
        );
    }

    #[test]
    fn plot_spec_carries_plot_title() {
        let mut cli = Cli::parse_from([
            "msp",
            ",xexpr=$1,yexpr=$2,title=Series A",
            "--backend",
            "echarts",
            "--plot-title",
            "Network Percentiles",
        ]);
        cli.fill_defaults();
        cli.convert_fields().unwrap();
        cli.work_dir = Some(std::env::temp_dir());

        let request = cli.build_resolved_request().unwrap();
        assert_eq!(request.plot.title.as_deref(), Some("Network Percentiles"));
        assert_eq!(
            request.data_prep.series[0].name.as_deref(),
            Some("Series A")
        );
    }

    #[test]
    fn invalid_axis_binding_is_rejected() {
        assert!("x3y1".parse::<SeriesAxisBinding>().is_err());
        assert!("x1y0".parse::<SeriesAxisBinding>().is_err());
        assert!("xy3".parse::<SeriesAxisBinding>().is_err());
    }

    #[test]
    fn gnuplot_rejects_y3_axis_usage() {
        let mut cli = Cli::parse_from([
            "msp",
            ",xexpr=$1,yexpr=$2,axis=x1y3",
            "--backend",
            "gnuplot.postscript",
        ]);
        cli.fill_defaults();
        cli.convert_fields().unwrap();
        cli.work_dir = Some(std::env::temp_dir());

        let request = cli.build_resolved_request().unwrap();
        let err = cli
            .validate_backend_support(&request, &Default::default())
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("gnuplot backend supports only y1 and y2")
        );
    }

    #[test]
    fn gnuplot_rejects_bar_series() {
        let mut cli = Cli::parse_from([
            "msp",
            ",xexpr=year,yexpr=value,plot=bar,title=Revenue",
            "--backend",
            "gnuplot.dumb",
        ]);
        cli.fill_defaults();
        cli.convert_fields().unwrap();
        cli.work_dir = Some(std::env::temp_dir());

        let request = cli.build_resolved_request().unwrap();
        let err = cli
            .validate_backend_support(&request, &Default::default())
            .unwrap_err();
        assert!(err.to_string().contains("does not support plot=bar"));
        assert!(err.to_string().contains("--backend echarts"));
    }

    #[test]
    fn gnuplot_rejects_number_format_axis_option() {
        let mut cli = Cli::parse_from([
            "msp",
            ",xexpr=$1,yexpr=$2",
            "--backend",
            "gnuplot.postscript",
            "--number-format",
            "y1=suffix",
        ]);
        cli.fill_defaults();
        cli.convert_fields().unwrap();
        cli.work_dir = Some(std::env::temp_dir());

        let request = cli.build_resolved_request().unwrap();
        let err = cli
            .validate_backend_support(&request, &Default::default())
            .unwrap_err();
        assert!(err.to_string().contains("does not support --number-format"));
    }

    #[test]
    fn invalid_axis_number_format_is_rejected() {
        let err = "engineering".parse::<AxisNumberFormat>().unwrap_err();
        assert!(
            err.to_string()
                .contains("Unknown axis number format 'engineering'")
        );
    }

    #[test]
    fn echarts_max_points_defaults_to_200() {
        let cli = Cli::parse_from([
            "msp",
            ",xexpr=$1,yexpr=$2",
            "--backend",
            "echarts",
        ]);

        let opts = cli.build_backend_options().unwrap();
        let BackendOptions::Echarts(opts) = opts else {
            panic!("expected echarts backend options");
        };
        assert_eq!(opts.max_points, Some(200));
        assert_eq!(opts.output_mode, EchartsOutputMode::Page);
        assert_eq!(opts.runtime_mode, EchartsRuntimeMode::Cdn);
    }

    #[test]
    fn echarts_max_points_cli_arg_overrides_backend_opt() {
        let cli = Cli::parse_from([
            "msp",
            ",xexpr=$1,yexpr=$2",
            "--backend",
            "echarts",
            "--backend-opt",
            "max-points=123",
            "--max-points",
            "50",
        ]);

        let opts = cli.build_backend_options().unwrap();
        let BackendOptions::Echarts(opts) = opts else {
            panic!("expected echarts backend options");
        };
        assert_eq!(opts.max_points, Some(50));
    }

    #[test]
    fn echarts_backend_options_parse_embed_and_external_runtime() {
        let cli = Cli::parse_from([
            "msp",
            ",xexpr=$1,yexpr=$2",
            "--backend",
            "echarts",
            "--backend-opt",
            "mode=embed",
            "--backend-opt",
            "runtime=external",
        ]);

        let opts = cli.build_backend_options().unwrap();
        let BackendOptions::Echarts(opts) = opts else {
            panic!("expected echarts backend options");
        };
        assert_eq!(opts.output_mode, EchartsOutputMode::Embed);
        assert_eq!(opts.runtime_mode, EchartsRuntimeMode::External);
    }

    #[test]
    fn echarts_unknown_mode_is_rejected() {
        let cli = Cli::parse_from([
            "msp",
            ",xexpr=$1,yexpr=$2",
            "--backend",
            "echarts",
            "--backend-opt",
            "mode=fragment",
        ]);

        let err = cli.build_backend_options().unwrap_err();
        assert!(err.to_string().contains("Unknown echarts mode 'fragment'"));
    }

    #[test]
    fn echarts_unknown_runtime_is_rejected() {
        let cli = Cli::parse_from([
            "msp",
            ",xexpr=$1,yexpr=$2",
            "--backend",
            "echarts",
            "--backend-opt",
            "runtime=inline",
        ]);

        let err = cli.build_backend_options().unwrap_err();
        assert!(err.to_string().contains("Unknown echarts runtime 'inline'"));
    }

    #[test]
    fn default_backend_is_gnuplot_postscript() {
        let cli = Cli::try_parse_cli([
            "msp",
            ",xexpr=$1,yexpr=$2",
            "-m",
            "dry-run",
            "-i",
            "input.csv",
        ])
        .unwrap();

        assert_eq!(cli.request().backend, BackendKind::GnuplotPostscript);
    }

    #[test]
    fn plain_gnuplot_backend_alias_is_rejected() {
        let err = Cli::try_parse_cli([
            "msp",
            ",xexpr=$1,yexpr=$2",
            "-m",
            "dry-run",
            "-i",
            "input.csv",
            "--backend",
            "gnuplot",
        ])
        .unwrap_err();

        assert!(err.to_string().contains("invalid value 'gnuplot'"));
    }

    #[test]
    fn gnuplot_terminal_backend_opt_is_rejected_with_guidance() {
        let cli = Cli::parse_from([
            "msp",
            ",xexpr=$1,yexpr=$2",
            "--backend",
            "gnuplot.postscript",
            "--backend-opt",
            "terminal=dumb",
        ]);

        let err = cli.build_backend_options().unwrap_err();
        assert!(err.to_string().contains("--backend-opt terminal=..."));
        assert!(err.to_string().contains("gnuplot.dumb"));
    }

    #[test]
    fn style_is_allowed_for_gnuplot_backends() {
        let cli = Cli::try_parse_cli([
            "msp",
            ",xexpr=$1,yexpr=$2,style=lw 2",
            "-m",
            "dry-run",
            "-i",
            "input.csv",
            "--backend",
            "gnuplot.dumb",
        ])
        .unwrap();

        assert_eq!(cli.request().backend, BackendKind::GnuplotDumb);
    }

    #[test]
    fn style_is_rejected_for_echarts() {
        let err = Cli::try_parse_cli([
            "msp",
            ",xexpr=$1,yexpr=$2,style=lw 2",
            "-m",
            "dry-run",
            "-i",
            "input.csv",
            "--backend",
            "echarts",
        ])
        .unwrap_err();

        assert!(err.to_string().contains("does not support --style"));
    }

    #[test]
    fn style_is_not_reported_for_echarts_when_left_empty() {
        let cli = Cli::try_parse_cli([
            "msp",
            ",xexpr=$1,yexpr=$2",
            "-m",
            "dry-run",
            "-i",
            "input.csv",
            "--backend",
            "echarts",
        ])
        .unwrap();

        assert_eq!(cli.request().backend, BackendKind::Echarts);
        assert!(cli.request().data_prep.series[0].style.raw.is_none());
    }

    #[test]
    fn max_points_is_rejected_for_gnuplot_backends() {
        let err = Cli::try_parse_cli([
            "msp",
            ",xexpr=$1,yexpr=$2",
            "-m",
            "dry-run",
            "-i",
            "input.csv",
            "--backend",
            "gnuplot.x11",
            "--max-points",
            "50",
        ])
        .unwrap_err();

        assert!(err.to_string().contains("does not support --max-points"));
    }

    #[test]
    fn plot_title_is_rejected_for_gnuplot_backends() {
        let err = Cli::try_parse_cli([
            "msp",
            ",xexpr=$1,yexpr=$2",
            "-m",
            "dry-run",
            "-i",
            "input.csv",
            "--backend",
            "gnuplot.postscript",
            "--plot-title",
            "Example",
        ])
        .unwrap_err();

        assert!(err.to_string().contains("does not support --plot-title"));
    }

    #[test]
    fn field_parses_relative_indexes() {
        let plus: Field<usize> = "+2".parse().unwrap();
        let minus: Field<usize> = "-1".parse().unwrap();
        assert!(matches!(plus, Field::PositiveRelative(2)));
        assert!(matches!(minus, Field::NegativeRelative(1)));
    }
}
