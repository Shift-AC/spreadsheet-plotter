use std::{
    env,
    fmt::Display,
    fs::File,
    io::{self, Cursor, Read},
    path::PathBuf,
    str::FromStr,
    usize,
};

use anyhow::{Context, bail};
use clap::{Parser, ValueEnum};
use rand::Rng;

#[derive(Debug, Clone)]
pub struct DataSeries {
    pub file_index: usize,
    pub xexpr: String,
    pub yexpr: String,
    pub opseq: String,
    pub title: String,
    pub plot_type: String,
    pub use_x2: bool,
    pub use_y2: bool,
}

impl DataSeries {
    pub fn get_matched_key(abs: &str) -> anyhow::Result<String> {
        const KEYS: [&str; 7] = [
            "file_index",
            "xexpr",
            "yexpr",
            "opseq",
            "title",
            "plot_type",
            "axis",
        ];

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
}

impl FromStr for DataSeries {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() < 2 {
            bail!("Empty data series string");
        }
        let delimeter = s.chars().next().unwrap();

        let mut file_index = usize::MAX;
        let mut xexpr = "1".to_string();
        let mut yexpr = "1".to_string();
        let mut opseq = "".to_string();
        let mut title = "".to_string();
        let mut plot_type = "".to_string();
        let mut axis = "11".to_string();

        for part in s[1..].split(delimeter) {
            let kv = part.splitn(2, '=').collect::<Vec<_>>();
            if kv.len() != 2 {
                bail!("Invalid data series part: {}", part);
            }
            let (k, v) = (kv[0], kv[1]);
            let k = DataSeries::get_matched_key(k)?;
            match k.as_str() {
                "file_index" => {
                    file_index = v.parse().map_err(|e| {
                        anyhow::anyhow!("Failed to parse file index: {}", e)
                    })?
                }
                "xexpr" => xexpr = v.to_string(),
                "yexpr" => yexpr = v.to_string(),
                "opseq" => opseq = v.to_string(),
                "title" => title = v.to_string(),
                "plot_type" => plot_type = v.to_string(),
                "axis" => axis = v.to_string(),
                _ => bail!("Unknown key: {}", k),
            }
        }

        let (use_x2, use_y2) = match axis.as_str() {
            "11" => (false, false),
            "21" => (true, false),
            "12" => (false, true),
            "22" => (true, true),
            _ => bail!("Invalid axis string: {}", axis),
        };

        Ok(Self {
            file_index,
            xexpr,
            yexpr,
            opseq,
            title,
            plot_type,
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
    ///     input = index of the file to be used as data source
    ///       [default: k-th file for k-th data series]
    ///       (0 = stdin, 1.. = paths provided with -i).
    ///     xexpr = expression to be used as x-axis [default: 1]
    ///     yexpr = expression to be used as y-axis [default: 1]
    ///     op = transforms to be applied on the data [default: ""]
    ///     title = title of the data series [default: ""]
    ///       ("" for using the column header from `sp` output)
    ///     type = plot type of the data series [default: use --type]
    ///     axis = axises for plotting the data series [default: "11"]
    ///       ("ab" for xayb)
    /// NOTE: prefix of keys is also supported.
    /// Example:
    ///   ,input=0 => (stdin, 1, 1, "", "", "", "11")
    ///   |x=${a,}|op=c|a=21 => (File#1, "${a,}", 1, "c", "", "", "21")
    #[arg(verbatim_doc_comment, required = true, value_name = "SERIES")]
    pub data_series: Vec<DataSeries>,

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

    /// Path to the gnuplot script to be used (use prefix . "[i].csv" for
    /// i-th data series), overwrites all other gnuplot options and the default
    /// gnuplot template
    #[arg(short = 'G')]
    gnuplot_file: Option<PathBuf>,

    /// Additional gnuplot commands to be used before the 'plot' command
    #[arg(short = 'g', value_name = "CMD", default_value = "")]
    additional_gnuplot_cmd: String,

    /// Default plot type for all data series
    #[arg(long = "type", default_value = "points")]
    plot_type: String,

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
    // #[arg(long = "term", default_value_t = Terminal::X11)]
    // terminal: Terminal,
    #[clap(skip)]
    terminal: Terminal,

    /// Gnuplot output destination
    #[arg(long = "gpout", default_value = "./msp_out.pdf")]
    gp_out: String,

    /// Range of x1 axis [default: auto]
    #[arg(long = "xr")]
    xrange: Option<String>,

    /// Range of y1 axis [default: auto]
    #[arg(long = "yr")]
    yrange: Option<String>,

    /// Label of x1 axis
    #[arg(long = "xl")]
    xlabel: Option<String>,

    /// Label of y1 axis
    #[arg(long = "yl")]
    ylabel: Option<String>,

    #[clap(skip)]
    pub output_prefix: String,

    #[clap(skip)]
    pub gpcmd: String,

    #[clap(skip)]
    pub stdin_content: String,
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
            let cmd = format!(
                "set macro\n\
                set prefix = '{}'\n\
                {}",
                self.out_path
                    .as_ref()
                    .unwrap()
                    .join(&self.output_prefix)
                    .display(),
                buf
            );
            return Ok(cmd);
        }

        // build the plot command
        let plot_cmd = self
            .data_series
            .iter()
            .enumerate()
            .map(|(i, ds)| {
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

                format!(
                    "    '{}' using 1:2 axis x{}y{}{} with {}",
                    input_path.display(),
                    if ds.use_x2 { "2" } else { "1" },
                    if ds.use_y2 { "2" } else { "1" },
                    title,
                    plot_type
                )
            })
            .collect::<Vec<String>>()
            .join(",\\\n");

        let xr_cmd = if let Some(xr) = &self.xrange {
            format!("set xrange [{}]\n", xr)
        } else {
            "".to_string()
        };

        let yr_cmd = if let Some(yr) = &self.yrange {
            format!("set yrange [{}]\n", yr)
        } else {
            "".to_string()
        };

        let xl_cmd = if let Some(xl) = &self.xlabel {
            format!("set xlabel '{}'\n", xl)
        } else {
            "".to_string()
        };

        let yl_cmd = if let Some(yl) = &self.ylabel {
            format!("set ylabel '{}'\n", yl)
        } else {
            "".to_string()
        };

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
            {}\
            {}\
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
            xl_cmd,
            yl_cmd,
            xr_cmd,
            yr_cmd,
            self.additional_gnuplot_cmd,
            plot_cmd,
        ))
    }

    pub fn parse_args() -> anyhow::Result<Self> {
        let mut cli = Self::parse();

        if !which::which("sp").is_ok() {
            bail!("sp is not installed");
        }

        cli.data_series.iter_mut().enumerate().for_each(|(i, ds)| {
            ds.file_index = match ds.file_index {
                usize::MAX => i + 1,
                _ => ds.file_index,
            }
        });

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

    fn check_file_index(&self) -> anyhow::Result<()> {
        for ds in &self.data_series {
            if ds.file_index != 0 {
                match self.input_paths.get(ds.file_index - 1) {
                    Some(path) => {
                        if !path.exists() {
                            bail!("File {} does not exist", path.display());
                        }
                    }
                    None => {
                        bail!("File index {} is out of range", ds.file_index);
                    }
                }
                continue;
            }
        }
        Ok(())
    }

    pub fn get_stdin_reader(&self) -> Cursor<&str> {
        io::Cursor::new(&self.stdin_content)
    }
}
