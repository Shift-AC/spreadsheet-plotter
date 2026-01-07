use std::{fmt::Display, str::FromStr};

#[derive(Debug, Clone)]
struct PlotSize {
    width: f64,
    height: f64,
}

impl Default for PlotSize {
    fn default() -> Self {
        Self {
            width: 1.0,
            height: 0.75,
        }
    }
}

impl Display for PlotSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{},{}", self.width, self.height)
    }
}

#[derive(Debug, Clone)]
struct Font {
    family: String,
    size: usize,
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
        write!(f, "\"{},{}\"", self.family, self.size)
    }
}

#[derive(Debug, Clone, Default)]
pub enum Terminal {
    X11,
    #[default]
    Postscript,
    Dumb(Option<u32>, Option<u32>),
}

impl Display for Terminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Terminal::X11 => write!(f, "x11 noenhanced"),
            Terminal::Postscript => {
                write!(f, "postscript eps color noenhanced")
            }
            Terminal::Dumb(width, height) => {
                write!(
                    f,
                    "dumb size {},{}",
                    width
                        .map(|w| w.to_string())
                        .unwrap_or("`tput cols`".to_string()),
                    height
                        .map(|h| h.to_string())
                        .unwrap_or("`echo $(($(tput lines) - 1))`".to_string()),
                )
            }
        }
    }
}

#[derive(Clone, Debug)]
enum AxisId {
    X,
    X2,
    Y,
    Y2,
}

impl Display for AxisId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AxisId::X => write!(f, "x"),
            AxisId::X2 => write!(f, "x2"),
            AxisId::Y => write!(f, "y"),
            AxisId::Y2 => write!(f, "y2"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AxisOptions {
    id: AxisId,

    /// Use logarithmic scale for axis (arg: base)
    logscale: Option<f64>,

    /// Range of axis (args: min, max) [default: auto]
    range: Option<std::ops::Range<f64>>,

    /// Label of axis (arg: label)
    label: Option<String>,

    /// Tics of axis (args: <pos, label>...)
    tics: Option<Vec<(f64, String)>>,
}

impl Default for AxisOptions {
    fn default() -> Self {
        Self {
            id: AxisId::X,
            logscale: None,
            range: None,
            label: None,
            tics: None,
        }
    }
}

impl AxisOptions {
    pub fn new_x() -> Self {
        Self {
            id: AxisId::X,
            ..Default::default()
        }
    }

    pub fn new_y() -> Self {
        Self {
            id: AxisId::Y,
            ..Default::default()
        }
    }

    pub fn new_x2() -> Self {
        Self {
            id: AxisId::X2,
            ..Default::default()
        }
    }

    pub fn new_y2() -> Self {
        Self {
            id: AxisId::Y2,
            ..Default::default()
        }
    }

    pub fn with_logscale(mut self, base: Option<f64>) -> Self {
        self.logscale = base;
        self
    }

    pub fn with_range(mut self, range: Option<std::ops::Range<f64>>) -> Self {
        self.range = range;
        self
    }

    pub fn with_label(mut self, label: Option<impl AsRef<str>>) -> Self {
        self.label = label.map(|s| s.as_ref().to_string());
        self
    }

    pub fn with_tics(mut self, has_tics: bool) -> Self {
        self.tics = if has_tics { None } else { Some(vec![]) };
        self
    }

    pub fn with_custom_tics(
        mut self,
        tics: Vec<(f64, impl AsRef<str>)>,
    ) -> Self {
        self.tics = Some(
            tics.into_iter()
                .map(|(pos, label)| (pos, label.as_ref().to_string()))
                .collect(),
        );
        self
    }

    fn need_configure(&self) -> bool {
        self.logscale.is_some()
            || self.range.is_some()
            || self.label.is_some()
            || self.tics.is_some()
    }
}

impl Display for AxisOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "## {} axis", self.id)?;
        if let Some(base) = self.logscale {
            let base = format!(" {base}");
            write!(f, "\nset logscale {}{}", self.id, base)?;
        }
        if let Some(range) = &self.range {
            write!(
                f,
                "\nset {}range [{}:{}]",
                self.id, range.start, range.end
            )?;
        }
        if let Some(label) = &self.label {
            write!(f, "\nset {}label \"{}\"", self.id, label)?;
        }
        if let Some(tics) = &self.tics {
            if tics.is_empty() {
                write!(f, "\nset {}tics", self.id)?;
            } else {
                write!(
                    f,
                    "\nset {}tics ({})",
                    self.id,
                    tics.iter()
                        .map(|(pos, label)| format!("\"{label}\" {pos}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum Color {
    Named(String),
    RGB(u8, u8, u8),
}

impl Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Color::Named(name) => write!(f, "\"{name}\""),
            Color::RGB(r, g, b) => {
                write!(f, "rgb \"#{r:02x}{g:02x}{b:02x}\"")
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct PointStyle {
    pub point_type: usize,
    pub size: f64,
}

impl Display for PointStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pt {} ps {}", self.point_type, self.size)
    }
}

#[derive(Clone, Debug)]
pub struct LineStyle {
    pub line_type: usize,
    pub color: Color,
    pub weight: f64,
}

impl Display for LineStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "lt {} lc {} w {}",
            self.line_type, self.color, self.weight
        )
    }
}

#[derive(Clone, Debug)]
pub enum PlotType {
    Points(Option<PointStyle>),
    Lines(Option<LineStyle>),
    Linespoints(Option<LineStyle>, Option<PointStyle>),
}

impl Display for PlotType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlotType::Points(None) => write!(f, "with points"),
            PlotType::Points(Some(style)) => write!(f, "with points {style}"),
            PlotType::Lines(None) => write!(f, "with lines"),
            PlotType::Lines(Some(style)) => write!(f, "with lines {style}"),
            PlotType::Linespoints(None, None) => write!(f, "with linespoints"),
            PlotType::Linespoints(None, Some(style)) => {
                write!(f, "with linespoints {style}")
            }
            PlotType::Linespoints(Some(style), None) => {
                write!(f, "with linespoints {style}")
            }
            PlotType::Linespoints(Some(line_style), Some(point_style)) => {
                write!(f, "with linespoints {line_style} {point_style}")
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct DataSeriesOptions {
    /// Path to the 2-column temporary datasheet file
    datasheet_path: String,

    /// Use x2 axis for this data series
    use_x2: bool,

    /// Use y2 axis for this data series
    use_y2: bool,

    /// Plot type to be used for this data series
    plot_type: PlotType,

    /// Label of this data series
    label: Option<String>,

    /// Additional options to be used for this data series
    additional_options: Option<String>,
}

impl Default for DataSeriesOptions {
    fn default() -> Self {
        Self {
            datasheet_path: "".to_string(),
            use_x2: false,
            use_y2: false,
            plot_type: PlotType::Points(None),
            label: None,
            additional_options: None,
        }
    }
}

impl DataSeriesOptions {
    pub fn from_datasheet_path(datasheet_path: impl AsRef<str>) -> Self {
        Self {
            datasheet_path: datasheet_path.as_ref().to_string(),
            ..Default::default()
        }
    }

    pub fn with_datasheet_path(
        mut self,
        datasheet_path: impl AsRef<str>,
    ) -> Self {
        self.datasheet_path = datasheet_path.as_ref().to_string();
        self
    }

    pub fn with_plot_type(mut self, plot_type: PlotType) -> Self {
        self.plot_type = plot_type;
        self
    }

    pub fn with_label(mut self, label: Option<impl AsRef<str>>) -> Self {
        self.label = label.map(|s| s.as_ref().to_string());
        self
    }

    pub fn with_additional_option(
        mut self,
        option: Option<impl AsRef<str>>,
    ) -> Self {
        self.additional_options = option.map(|s| s.as_ref().to_string());
        self
    }

    pub fn with_use_x2(mut self, use_x2: bool) -> Self {
        self.use_x2 = use_x2;
        self
    }

    pub fn with_use_y2(mut self, use_y2: bool) -> Self {
        self.use_y2 = use_y2;
        self
    }
}

impl Display for DataSeriesOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "'{}' using 1:2 axis x{}y{} {}",
            self.datasheet_path,
            if self.use_x2 { "2" } else { "1" },
            if self.use_y2 { "2" } else { "1" },
            self.plot_type,
        )?;
        if let Some(lbl) = &self.label {
            write!(f, " title \"{lbl}\"")?;
        }
        if let Some(additional_options) = &self.additional_options {
            write!(f, " {additional_options}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct GnuplotTemplate {
    /// Additional gnuplot commands to be used before the 'plot' command
    additional_command: Option<String>,

    /// Size of the plot (width, height)
    plot_size: PlotSize,

    /// Font to be used for all labels (family, size)
    font: Option<Font>,

    /// Position of legends
    key_position: String,

    /// Font size to be used for all keys [default: same as --font]
    key_font: Option<Font>,

    /// Terminal to be used for plotting
    terminal: Terminal,

    /// Gnuplot output destination
    output: Option<String>,

    /// Options for x1 axis
    xopt: AxisOptions,

    /// Options for x2 axis
    x2opt: AxisOptions,

    /// Options for y1 axis
    yopt: AxisOptions,

    /// Options for y2 axis
    y2opt: AxisOptions,

    /// Display grid
    grid: bool,

    /// Data series options
    data_series_options: Vec<DataSeriesOptions>,
}

impl Default for GnuplotTemplate {
    fn default() -> Self {
        Self {
            additional_command: None,
            plot_size: PlotSize::default(),
            font: None,
            key_position: "top right".to_string(),
            key_font: None,
            terminal: Terminal::Postscript,
            output: None,
            xopt: AxisOptions::new_x(),
            x2opt: AxisOptions::new_x2(),
            yopt: AxisOptions::new_y(),
            y2opt: AxisOptions::new_y2(),
            grid: false,
            data_series_options: Vec::new(),
        }
    }
}

impl GnuplotTemplate {
    pub fn from_data_series_options(
        data_series_options: Vec<DataSeriesOptions>,
    ) -> Self {
        Self {
            data_series_options,
            ..Default::default()
        }
    }

    pub fn with_data_series_options(
        mut self,
        data_series_options: Vec<DataSeriesOptions>,
    ) -> Self {
        self.data_series_options = data_series_options;
        self
    }
    pub fn with_additional_command(
        mut self,
        additional_command: Option<impl AsRef<str>>,
    ) -> Self {
        self.additional_command =
            additional_command.map(|s| s.as_ref().to_string());
        self
    }
    pub fn with_plot_size(mut self, width: f64, height: f64) -> Self {
        self.plot_size = PlotSize { width, height };
        self
    }
    pub fn with_font(mut self, font: Option<(impl AsRef<str>, usize)>) -> Self {
        self.font = font.map(|(family, size)| Font {
            family: family.as_ref().to_string(),
            size,
        });
        self
    }
    pub fn with_key_position(mut self, key_position: impl AsRef<str>) -> Self {
        self.key_position = key_position.as_ref().to_string();
        self
    }
    pub fn with_key_font(
        mut self,
        key_font: Option<(impl AsRef<str>, usize)>,
    ) -> Self {
        self.key_font = key_font.map(|(family, size)| Font {
            family: family.as_ref().to_string(),
            size,
        });
        self
    }
    pub fn with_terminal(mut self, terminal: Terminal) -> Self {
        self.terminal = terminal;
        self
    }
    pub fn with_output(mut self, output: Option<impl AsRef<str>>) -> Self {
        self.output = output.map(|s| s.as_ref().to_string());
        self
    }
    pub fn with_grid(mut self, grid: bool) -> Self {
        self.grid = grid;
        self
    }
    pub fn with_xopt(mut self, xopt: AxisOptions) -> Self {
        self.xopt = xopt;
        self
    }
    pub fn with_x2opt(mut self, x2opt: AxisOptions) -> Self {
        self.x2opt = x2opt;
        self
    }
    pub fn with_yopt(mut self, yopt: AxisOptions) -> Self {
        self.yopt = yopt;
        self
    }
    pub fn with_y2opt(mut self, y2opt: AxisOptions) -> Self {
        self.y2opt = y2opt;
        self
    }
}

impl Display for GnuplotTemplate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "#!/usr/bin/env -S gnuplot -p")?;

        writeln!(f, "# Preamble")?;
        writeln!(f, "set encoding utf8")?;
        writeln!(f, "set datafile separator ','")?;
        writeln!(f, "set key autotitle columnhead")?;
        write!(
            f,
            "set terminal {}{}\n\n",
            self.terminal,
            match &self.font {
                Some(font) => format!(" font {font}"),
                None => "".to_string(),
            }
        )?;

        writeln!(f, "# Axes")?;
        if self.xopt.need_configure() {
            writeln!(f, "{}", self.xopt)?;
        }
        if self.yopt.need_configure() {
            writeln!(f, "{}", self.yopt)?;
        }
        if self.data_series_options.iter().any(|opt| opt.use_x2)
            && self.x2opt.need_configure()
        {
            writeln!(f, "{}", self.x2opt)?;
        }
        if self.data_series_options.iter().any(|opt| opt.use_y2)
            && self.y2opt.need_configure()
        {
            writeln!(f, "{}", self.y2opt)?;
        }
        writeln!(f)?;

        writeln!(f, "# Global appearance")?;
        if let Some(font) = &self.key_font {
            writeln!(f, "set key font \"{},{}\"", font.family, font.size)?;
        }
        writeln!(f, "set size {}", self.plot_size)?;
        writeln!(f, "set key {}", self.key_position)?;
        if self.grid {
            writeln!(f, "set grid")?;
        }
        writeln!(f)?;

        if let Some(cmd) = &self.additional_command {
            writeln!(f, "# Custom commands")?;
            write!(f, "{cmd}\n\n")?;
        }

        // note that currently only Postscript terminal may generate files.
        // in this case we directly pass the output to ps2pdf to compile the
        // postscript file into a pdf document.
        if let Some(output) = &self.output {
            writeln!(f, "set output '|ps2pdf -dEPSCrop - {output}'")?;
        }
        write!(
            f,
            "plot\\\n\t{}\n",
            self.data_series_options
                .iter()
                .map(|opt| format!("{opt}"))
                .collect::<Vec<_>>()
                .join(",\\\n\t")
        )?;

        Ok(())
    }
}

#[test]
fn test_gnuplot_script_display() {
    let xopt = AxisOptions::new_x()
        .with_label(Some("Time"))
        .with_custom_tics(vec![(2.0, "2"), (0.5, "1/2")])
        .with_range(Some(0.1f64..10f64));

    let yopt = AxisOptions::new_y()
        .with_label(Some("Income (K$)"))
        .with_tics(true)
        .with_range(Some(0f64..1f64));

    let y2opt = AxisOptions::new_y2()
        .with_label(Some("Cost ($)"))
        .with_tics(true)
        .with_range(Some(0f64..1f64))
        .with_logscale(Some(10.0));

    let income_line_style = LineStyle {
        line_type: 1,
        color: Color::Named("red".to_string()),
        weight: 2.0,
    };

    let cost_line_style = LineStyle {
        line_type: 2,
        color: Color::Named("blue".to_string()),
        weight: 2.0,
    };

    let alice_point_style = PointStyle {
        point_type: 1,
        size: 2.0,
    };

    let bob_point_style = PointStyle {
        point_type: 2,
        size: 2.0,
    };

    let ds_1 =
        DataSeriesOptions::from_datasheet_path("alice.monthly.income.csv")
            .with_label(Some("Alice"))
            .with_plot_type(PlotType::Linespoints(
                Some(income_line_style.clone()),
                Some(alice_point_style.clone()),
            ));

    let ds_2 = DataSeriesOptions::from_datasheet_path("bob.monthly.income.csv")
        .with_label(Some("Bob"))
        .with_plot_type(PlotType::Linespoints(
            Some(income_line_style),
            Some(bob_point_style.clone()),
        ));

    let ds_3 = ds_1
        .clone()
        .with_datasheet_path("alice.cost.csv")
        .with_plot_type(PlotType::Linespoints(
            Some(cost_line_style.clone()),
            Some(alice_point_style),
        ))
        .with_use_y2(true);

    let ds_4 = ds_2
        .clone()
        .with_datasheet_path("bob.cost.csv")
        .with_plot_type(PlotType::Linespoints(
            Some(cost_line_style),
            Some(bob_point_style),
        ))
        .with_use_y2(true);

    let script = GnuplotTemplate::default()
        .with_terminal(Terminal::Postscript)
        .with_key_position("top right")
        .with_font(Some(("Times New Roman", 12)))
        .with_plot_size(1.0, 0.75)
        .with_grid(true)
        .with_xopt(xopt)
        .with_yopt(yopt)
        .with_y2opt(y2opt)
        .with_output(Some("1.pdf"))
        .with_data_series_options(vec![ds_1, ds_2, ds_3, ds_4])
        .with_additional_command(Some("set title 'Test Plot'"));

    println!("{script}");
}
