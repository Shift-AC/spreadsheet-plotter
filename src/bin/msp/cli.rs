use std::{
    env,
    fmt::Display,
    fs::File,
    io::{self, Cursor, Read},
    path::PathBuf,
    str::FromStr,
    sync::{Arc, LazyLock, Mutex},
    usize,
};

use anyhow::{Context, bail};
use clap::{Parser, ValueEnum};
use rand::Rng;

#[derive(Debug, Clone)]
struct InputDataSeries {
    axis: Field<String>,
    file_index: Field<usize>,
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
            file_index: Field::Default,
            xexpr: Field::Default,
            yexpr: Field::Default,
            opseq: Field::Default,
            title: Field::Default,
            plot_type: Field::Default,
            axis: Field::Default,
            style: Field::Default,
        }))
    });

impl Default for InputDataSeries {
    fn default() -> Self {
        (*DEFAULT_INPUT_DATA_SERIES.lock().unwrap()).clone()
    }
}

impl InputDataSeries {
    fn do_get_matched_key(
        abs: &str,
        match_ref: bool,
    ) -> anyhow::Result<String> {
        const KEYS: [&str; 7] = [
            "axis",
            "file_index",
            "opseq",
            "plot_type",
            "title",
            "xexpr",
            "yexpr",
        ];

        // try to parse a reference
        if match_ref && abs.starts_with('r') {
            let key = Self::do_get_matched_key(&abs[1..], false)?;
            return match key.as_str() {
                "file" => Err(anyhow::anyhow!("Key rfile is illegal")),
                _ => Ok(format!("r{}", key)),
            };
        }

        let matched_keys = KEYS
            .iter()
            .filter(|k| k.starts_with(abs))
            .map(|k| k.to_string())
            .collect::<Vec<_>>();
        if matched_keys.is_empty() {
            bail!("Unknown key: {}", abs);
        } else if matched_keys.len() == 1 {
            Ok(matched_keys[0].to_string())
        } else {
            bail!(
                "Ambiguous key: {} (possible variants: {})",
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
        let delimeter = s.chars().next().unwrap();

        let mut ids = InputDataSeries::default();

        for part in s[1..].split(delimeter) {
            let kv = part.splitn(2, '=').collect::<Vec<_>>();
            if kv.len() != 2 {
                bail!("Invalid data series part: {}", part);
            }
            let (k, v) = (kv[0], kv[1]);
            let k = InputDataSeries::get_matched_key(k)?;

            match k.as_str() {
                "file_index" => ids.file_index = v.parse()?,
                "xexpr" => ids.xexpr = Field::Instant(v.to_string()),
                "rxexpr" => ids.xexpr = v.parse()?,
                "yexpr" => ids.yexpr = Field::Instant(v.to_string()),
                "ryexpr" => ids.yexpr = v.parse()?,
                "opseq" => ids.opseq = Field::Instant(v.to_string()),
                "ropseq" => ids.opseq = v.parse()?,
                "title" => ids.title = Field::Instant(v.to_string()),
                "rtitle" => ids.title = v.parse()?,
                "plot_type" => ids.plot_type = Field::Instant(v.to_string()),
                "rplot_type" => ids.plot_type = v.parse()?,
                "axis" => ids.axis = Field::Instant(v.to_string()),
                "raxis" => ids.axis = v.parse()?,
                _ => bail!("Unknown key: {}", k),
            }
        }

        Ok(ids)
    }
}

#[derive(Debug, Clone)]
pub struct DataSeries {
    pub file_index: usize,
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
            "x" => (false, false),
            "x2" => (true, false),
            "y" => (false, true),
            "y2" => (true, true),
            _ => bail!("Unknown axis: {}", axis),
        };
        Ok(Self {
            file_index: ids.file_index.try_into()?,
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
        let width = parts.next().unwrap().parse().map_err(|e| {
            anyhow::anyhow!("Failed to parse plot width: {}", e)
        })?;
        let height = parts.next().unwrap().parse().map_err(|e| {
            anyhow::anyhow!("Failed to parse plot height: {}", e)
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
                anyhow::anyhow!("Failed to parse font size: {}", e)
            })?;
        Ok(Self { family, size })
    }
}

impl Display for Font {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{},{}", self.family, self.size)
    }
}

#[derive(ValueEnum, Clone, Debug)]
pub enum Terminal {
    X11,
    POSTSCRIPT,
}

impl Default for Terminal {
    fn default() -> Self {
        Terminal::POSTSCRIPT
    }
}

impl Display for Terminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Terminal::X11 => write!(f, "x11"),
            Terminal::POSTSCRIPT => write!(f, "postscript eps color"),
        }
    }
}

#[derive(Clone, Debug)]
enum Field<T: Clone + std::fmt::Debug + std::fmt::Display> {
    Relative(isize),
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
                            "Failed to retrieve instant value from field {}",
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

impl<T> Field<T>
where
    T: Clone + std::fmt::Debug + std::fmt::Display,
{
    fn to_absolute(&mut self, base: usize) -> anyhow::Result<()> {
        Ok(match self {
            Self::Relative(index) => {
                if *index < 0 && index.abs() as usize > base {
                    bail!(
                        "Requiring minus index (required {}, base {})",
                        self,
                        base
                    );
                }
                *self = Self::Absolute((base as isize + *index) as usize);
            }
            _ => {}
        })
    }
}

impl<T> FromStr for Field<T>
where
    T: Clone + std::fmt::Debug + std::fmt::Display,
{
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with('+') {
            let index = s[1..].parse().map_err(|e| {
                anyhow::anyhow!("Failed to parse relative input index: {}", e)
            })?;
            Ok(Self::Relative(index))
        } else {
            let index = s.parse().map_err(|e| {
                anyhow::anyhow!("Failed to parse absolute input index: {}", e)
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
            Self::Relative(index) => write!(f, "+{}", index),
            Self::Absolute(index) => write!(f, "{}", index),
            Self::Default => write!(f, ""),
            Self::Instant(instant) => write!(f, "{}", instant),
        }
    }
}

/// Multi-spreadsheet plotter: sp wrapper for creating complex plots with
/// multiple data series
#[derive(Parser, Debug)]
#[command(
    version = env!("VERSION"),
    term_width = 80)]
pub struct Cli {
    /// SERIES = ([d]key=value)...
    ///   d = single character to be used as delimiter
    ///   keys:
    ///     axis = axises to plot on ("12" for x1y2)
    ///     file-index = REF of data source file
    ///     opseq = transforms to apply on the data
    ///     plot-type = plot type of the data series
    ///     style = plotting style of the data series
    ///     title = title of the data series
    ///     xexpr = x-axis expression
    ///     yexpr = y-axis expression
    ///     rKEY = KEY's value of series[REF]
    ///       (rfile_index is illegal)
    /// REF = (+)[num]
    ///   [num]: Absolute index (1-based),
    ///     (0 for stdin if referring to input file)
    ///   [+num]: Relative index (current index + num),
    /// NOTE: prefix of keys is also supported (e.g. a for axis).
    /// Example:
    ///   ,input=0 => input=stdin
    ///   |x=${a,}|op=c|a=21 => xexpr="${a,}", opseq="c", axis="21"
    ///   ,rx=1,ry=+-1 =>
    ///     xexpr=series[1].xexpr,
    ///     yexpr=series[current_index - 1].yexpr
    #[arg(verbatim_doc_comment, required = true, value_name = "SERIES")]
    input_data_series: Vec<InputDataSeries>,

    /// Dry-run mode: do not plot, produce all output datasheets and print
    /// the gnuplot script to be used to stdout (implies -p ./msp_out)
    #[arg(short = 'd')]
    pub dry_run: bool,

    /// Index of headerless input file (specify multiple times for multiple
    /// files)
    #[arg(short = 'H', value_name = "INDEX")]
    pub headless_indexes: Vec<usize>,

    /// Path to input file (specify multiple times for multiple files)
    #[arg(short = 'i', value_name = "PATH")]
    pub input_paths: Vec<PathBuf>,

    /// Path of the output directory [default: system temporary directory]
    #[arg(short = 'p')]
    pub out_path: Option<PathBuf>,

    /// Default axis for all data series
    #[arg(long = "axis", default_value = "11")]
    axis: String,

    /// Default input index for all data series
    #[arg(long = "file-index", default_value = "+1")]
    file_index: Field<usize>,

    /// Default x-axis expression for all data series
    #[arg(long = "xexpr", default_value = "1")]
    xexpr: String,

    /// Default y-axis expression for all data series
    #[arg(long = "yexpr", default_value = "1")]
    yexpr: String,

    /// Default operation sequence for all data series
    #[arg(long = "opseq", default_value = "")]
    opseq: String,

    /// Default plotting style for all data series
    #[arg(long = "style", default_value = "")]
    style: String,

    /// Default title for all data series
    #[arg(long = "title", default_value = "")]
    title: String,

    /// Default plot type for all data series
    #[arg(long = "plot-type", default_value = "points")]
    plot_type: String,

    /// Path to the gnuplot script to be used (use prefix . "[i].csv" for
    /// i-th data series), overwrites all other gnuplot options and the default
    /// gnuplot template
    #[arg(short = 'G')]
    gnuplot_file: Option<PathBuf>,

    /// Additional gnuplot commands to be used before the 'plot' command
    #[arg(short = 'g', value_name = "CMD", default_value = "")]
    additional_gnuplot_cmd: String,

    /// Size of the plot (width, height)
    #[arg(long = "size", default_value = "1,0.75")]
    plot_size: PlotSize,

    /// Font to be used for all labels (family, size)
    #[arg(long = "font", default_value = "Helvetica,24")]
    font: Font,

    /// Position of legends
    #[arg(long = "kpos", default_value = "top right")]
    key_position: String,

    /// Font size to be used for all keys [default: same as --font]
    #[arg(long = "kfont", value_name = "FONT")]
    key_font: Option<Font>,

    /// Terminal to be used for plotting
    #[arg(long = "term", default_value_t = Terminal::X11)]
    terminal: Terminal,

    /// Gnuplot output destination
    #[arg(long = "gpout", default_value = "./msp_out.pdf")]
    gp_out: String,

    /// Range of x1 axis [default: auto]
    #[arg(long = "xr")]
    xrange: Option<String>,

    /// Range of x2 axis [default: auto]
    #[arg(long = "x2r")]
    x2range: Option<String>,

    /// Range of y1 axis [default: auto]
    #[arg(long = "yr")]
    yrange: Option<String>,

    /// Range of y2 axis [default: auto]
    #[arg(long = "y2r")]
    y2range: Option<String>,

    /// Label of x1 axis
    #[arg(long = "xl")]
    xlabel: Option<String>,

    /// Label of x2 axis
    #[arg(long = "x2l")]
    x2label: Option<String>,

    /// Label of y1 axis
    #[arg(long = "yl")]
    ylabel: Option<String>,

    /// Label of y2 axis
    #[arg(long = "y2l")]
    y2label: Option<String>,

    #[clap(skip)]
    pub output_prefix: String,

    #[clap(skip)]
    pub gpcmd: String,

    #[clap(skip)]
    pub stdin_content: String,

    #[clap(skip)]
    pub data_series: Vec<DataSeries>,
}

impl Cli {
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
        if matches!(ds.file_index, Field::Default) {
            ds.file_index = default_series.file_index.clone();
        }
        let last_index =
            converted_dss.last().map(|ds| ds.file_index).unwrap_or(0);
        ds.file_index.to_absolute(last_index)?;

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
                    Field::Relative(i) => {
                        let i = i + index as isize;
                        if i <= 0 || i as usize > index {
                            bail!(
                                "Index {} is out of range (expected [1, {}])",
                                i,
                                index
                            );
                        }

                        ds.$field = Field::Instant(
                            converted_dss[i as usize - 1].$field.clone(),
                        );
                    }
                }
            };
        }
        convert_field!(axis);
        convert_field!(style);
        convert_field!(title);
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

    fn check_file_index(&mut self) -> anyhow::Result<()> {
        // check if all file indexes and related files are valid
        self.data_series
            .iter()
            .zip(self.input_data_series.iter())
            .try_for_each(|(ds, ids)| {
                if self.input_paths.len() <= ds.file_index {
                    bail!(
                        "File index {} ({}) is out of range",
                        ds.file_index,
                        ids.file_index
                    );
                }
                if !self.input_paths[ds.file_index].exists() {
                    bail!(
                        "File {} ({}, {}) does not exist",
                        ds.file_index,
                        ids.file_index,
                        self.input_paths[ds.file_index].display(),
                    );
                }
                Ok(())
            })
    }

    pub fn get_stdin_reader(&self) -> Cursor<&str> {
        io::Cursor::new(&self.stdin_content)
    }

    fn build_stdin_content(&self) -> anyhow::Result<String> {
        // if nobody references stdin, do not bother reading it
        if self.data_series.iter().all(|ds| ds.file_index != 0) {
            return Ok("".to_string());
        }

        let mut stdin_content = String::new();
        std::io::stdin().read_to_string(&mut stdin_content)?;
        Ok(stdin_content)
    }

    fn build_gnuplot_cmd(&self) -> anyhow::Result<String> {
        if let Some(path) = &self.gnuplot_file {
            let mut buf = String::new();
            File::open(path)?.read_to_string(&mut buf)?;
            let macros = (0..self.data_series.len())
                .map(|i| {
                    format!(
                        "ds_{} = '{}'\n",
                        i + 1,
                        self.out_path
                            .as_ref()
                            .unwrap()
                            .join(self.get_output_path(i))
                            .display()
                    )
                })
                .collect::<String>();

            let cmd = format!(
                "set macro\n\
                {}\n\
                {}",
                macros, buf
            );
            return Ok(cmd);
        }

        // build the plot command
        let plot_cmd = self
            .data_series
            .iter()
            .scan(0, |last_input_index, ds| {
                let i = ds.file_index;
                *last_input_index = i;
                let input_path = self.get_output_path(i);
                let plot_type = if ds.plot_type.is_empty() {
                    &self.plot_type
                } else {
                    &ds.plot_type
                };
                let title = if ds.title.is_empty() {
                    "".to_string()
                } else {
                    format!(" title '{}'", ds.title)
                };

                Some(format!(
                    "    '{}' using 1:2 axis x{}y{}{} with {} {}",
                    input_path.display(),
                    if ds.use_x2 { "2" } else { "1" },
                    if ds.use_y2 { "2" } else { "1" },
                    title,
                    plot_type,
                    ds.style,
                ))
            })
            .collect::<Vec<String>>()
            .join(",\\\n");

        macro_rules! optional_cmd {
            ($cmd:ident, $fmt:expr) => {
                if let Some(val) = &self.$cmd {
                    format!($fmt, val)
                } else {
                    "".to_string()
                }
            };
        }

        let xr_cmd = optional_cmd!(xrange, "set xrange [{}]\n");
        let yr_cmd = optional_cmd!(yrange, "set yrange [{}]\n");
        let xl_cmd = optional_cmd!(xlabel, "set xlabel '{}'\n");
        let yl_cmd = optional_cmd!(ylabel, "set ylabel '{}'\n");

        let x2r_cmd = optional_cmd!(x2range, "set x2range [{}]\n");
        let y2r_cmd = optional_cmd!(y2range, "set yr2ange [{}]\n");
        let x2l_cmd = optional_cmd!(x2label, "set xla2bel '{}'\n");
        let y2l_cmd = optional_cmd!(y2label, "set ylab2el '{}'\n");

        let optional_cmds = vec![
            xr_cmd, yr_cmd, xl_cmd, yl_cmd, x2r_cmd, y2r_cmd, x2l_cmd, y2l_cmd,
        ]
        .join("");

        let gp_out = if matches!(self.terminal, Terminal::POSTSCRIPT) {
            format!("|ps2pdf -dEPSCrop - {}", self.gp_out)
        } else {
            self.gp_out.clone()
        };

        Ok(format!(
            "set datafile separator ','\n\
            set key autotitle columnhead\n\
            set terminal {} font '{}'\n\
            set size {}\n\
            set key font '{}'\n\
            set key {}\n\
            set output '{}'\n\
            {}\
            {}\n\
            plot\\\n\
            {}",
            self.terminal,
            self.font,
            self.plot_size,
            self.key_font.as_ref().unwrap().to_string(),
            self.key_position,
            gp_out,
            optional_cmds,
            self.additional_gnuplot_cmd,
            plot_cmd,
        ))
    }

    /// Set default value of InputDataSeries according to command line options
    fn fill_defaults(&mut self) {
        let ds_wrap = DEFAULT_INPUT_DATA_SERIES.clone();
        let mut ds = ds_wrap.lock().unwrap();

        ds.file_index = self.file_index.clone();
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

        if !which::which("sp").is_ok() {
            bail!("sp is not installed");
        }

        cli.fill_defaults();
        cli.convert_fields()?;
        cli.check_file_index()?;

        cli.output_prefix = Self::gen_output_prefix();
        cli.stdin_content = cli.build_stdin_content()?;

        if cli.out_path.is_none() {
            if cli.dry_run {
                cli.out_path = Some(PathBuf::from("./msp_out"));
            } else {
                cli.out_path = Some(env::temp_dir());
            }
        }

        if !cli.out_path.as_ref().unwrap().is_dir() {
            std::fs::create_dir_all(cli.out_path.as_ref().unwrap()).context(
                format!(
                    "Failed to create output directory '{}'",
                    cli.out_path.as_ref().unwrap().display()
                ),
            )?;
        }

        if cli.key_font.is_none() {
            cli.key_font = Some(cli.font.clone());
        }

        if matches!(cli.terminal, Terminal::POSTSCRIPT) {
            if !which::which("ps2pdf").is_ok() {
                bail!("ps2pdf is not installed");
            }
        }

        cli.gpcmd = cli.build_gnuplot_cmd()?;

        Ok(cli)
    }
}
