use std::{
    collections::BTreeMap,
    fmt::{Display, Write},
    ops::Range,
    path::PathBuf,
    str::FromStr,
};

use anyhow::{Context, bail};
use spreadsheet_plotter::DataFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Plot,
    Prepare,
    DryRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    GnuplotPostscript,
    GnuplotDumb,
    GnuplotX11,
    Echarts,
}

impl BackendKind {
    pub const fn is_gnuplot(self) -> bool {
        matches!(
            self,
            Self::GnuplotPostscript | Self::GnuplotDumb | Self::GnuplotX11
        )
    }

    pub const fn is_echarts(self) -> bool {
        matches!(self, Self::Echarts)
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::GnuplotPostscript => "gnuplot.postscript",
            Self::GnuplotDumb => "gnuplot.dumb",
            Self::GnuplotX11 => "gnuplot.x11",
            Self::Echarts => "echarts",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesMark {
    Points,
    Lines,
    LinesPoints,
    Bar,
    #[allow(dead_code)]
    Boxplot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AxisDimension {
    X,
    Y,
}

impl Display for AxisDimension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::X => write!(f, "x"),
            Self::Y => write!(f, "y"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AxisRef {
    pub dimension: AxisDimension,
    pub index: usize,
}

impl AxisRef {
    pub const fn x(index: usize) -> Self {
        Self {
            dimension: AxisDimension::X,
            index,
        }
    }

    pub const fn y(index: usize) -> Self {
        Self {
            dimension: AxisDimension::Y,
            index,
        }
    }
}

impl Display for AxisRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.dimension, self.index)
    }
}

impl FromStr for AxisRef {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_ascii_lowercase();
        let normalized = match normalized.as_str() {
            "x" => "x1",
            "y" => "y1",
            _ => normalized.as_str(),
        };

        let (dimension, index) = match normalized.chars().next() {
            Some('x') => (
                AxisDimension::X,
                normalized[1..].parse::<usize>().with_context(|| {
                    format!("Failed to parse x axis index from '{s}'")
                })?,
            ),
            Some('y') => (
                AxisDimension::Y,
                normalized[1..].parse::<usize>().with_context(|| {
                    format!("Failed to parse y axis index from '{s}'")
                })?,
            ),
            _ => bail!("Failed to parse axis id: {s}"),
        };

        match dimension {
            AxisDimension::X if !(1..=2).contains(&index) => {
                bail!("Only x1 and x2 are supported, got '{s}'")
            }
            AxisDimension::Y if index == 0 => {
                bail!("Y axis index must be >= 1, got '{s}'")
            }
            _ => Ok(Self { dimension, index }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisScale {
    Linear,
    Log10,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AxisNumberFormat {
    #[default]
    Plain,
    Suffix,
    Scientific,
}

impl FromStr for AxisNumberFormat {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "plain" => Ok(Self::Plain),
            "suffix" => Ok(Self::Suffix),
            "scientific" => Ok(Self::Scientific),
            _ => bail!("Unknown axis number format '{s}'"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeriesAxisBinding {
    pub x_index: usize,
    pub y_index: usize,
}

impl SeriesAxisBinding {
    pub const fn new(x_index: usize, y_index: usize) -> Self {
        Self { x_index, y_index }
    }

    pub fn x_axis(self) -> AxisRef {
        AxisRef::x(self.x_index)
    }

    pub fn y_axis(self) -> AxisRef {
        AxisRef::y(self.y_index)
    }

    pub fn use_x2(self) -> bool {
        self.x_index == 2
    }

    pub fn use_y2(self) -> bool {
        self.y_index > 1
    }
}

impl Display for SeriesAxisBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "x{}y{}", self.x_index, self.y_index)
    }
}

impl FromStr for SeriesAxisBinding {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_ascii_lowercase();
        let normalized = match normalized.as_str() {
            "11" => "x1y1".to_string(),
            "12" => "x1y2".to_string(),
            "21" => "x2y1".to_string(),
            "22" => "x2y2".to_string(),
            _ => normalized,
        };

        let Some(rest) = normalized.strip_prefix('x') else {
            bail!("Failed to parse axis binding: {s}");
        };
        let Some((x_part, y_part)) = rest.split_once('y') else {
            bail!("Failed to parse axis binding: {s}");
        };
        let x_index = x_part.parse::<usize>().with_context(|| {
            format!("Failed to parse x axis index from '{s}'")
        })?;
        let y_index = y_part.parse::<usize>().with_context(|| {
            format!("Failed to parse y axis index from '{s}'")
        })?;
        if !(1..=2).contains(&x_index) {
            bail!("Only x1 and x2 are supported, got '{s}'");
        }
        if y_index == 0 {
            bail!("Y axis index must be >= 1, got '{s}'");
        }
        Ok(Self::new(x_index, y_index))
    }
}

#[derive(Debug, Clone)]
pub struct RegisteredInput {
    pub index: usize,
    pub path: Option<PathBuf>,
    pub header_presence: Option<bool>,
    pub format: Option<DataFormat>,
}

#[derive(Debug, Clone)]
pub struct SeriesStyle {
    pub raw: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SeriesSpec {
    pub axis_binding: SeriesAxisBinding,
    pub input_ref: usize,
    pub input_filter: String,
    pub output_filter: String,
    pub opseq: String,
    pub x_expr: String,
    pub y_expr: String,
    pub mark: SeriesMark,
    pub boxplot_group: Option<usize>,
    pub name: Option<String>,
    pub style: SeriesStyle,
}

#[derive(Debug, Clone)]
pub struct TickSpec {
    pub major: Option<StandardTickSpec>,
    pub custom: Vec<(f64, String)>,
}

#[derive(Debug, Clone)]
pub struct StandardTickSpec {
    pub range: Option<Range<f64>>,
    pub step: f64,
}

#[derive(Debug, Clone)]
pub struct AxisSpec {
    pub scale: AxisScale,
    pub number_format: AxisNumberFormat,
    pub range: Option<Range<f64>>,
    pub label: Option<String>,
    pub ticks: TickSpec,
}

impl Default for AxisSpec {
    fn default() -> Self {
        Self {
            scale: AxisScale::Linear,
            number_format: AxisNumberFormat::Plain,
            range: None,
            label: None,
            ticks: TickSpec {
                major: None,
                custom: Vec::new(),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlotAxes {
    axes: BTreeMap<AxisRef, AxisSpec>,
}

impl Default for PlotAxes {
    fn default() -> Self {
        let mut axes = BTreeMap::new();
        axes.insert(AxisRef::x(1), AxisSpec::default());
        axes.insert(AxisRef::x(2), AxisSpec::default());
        axes.insert(AxisRef::y(1), AxisSpec::default());
        axes.insert(AxisRef::y(2), AxisSpec::default());
        Self { axes }
    }
}

impl PlotAxes {
    pub fn insert(&mut self, axis: AxisRef, spec: AxisSpec) {
        self.axes.insert(axis, spec);
    }

    pub fn axis(&self, axis: AxisRef) -> Option<&AxisSpec> {
        self.axes.get(&axis)
    }

    pub fn get(&self, axis: AxisRef) -> AxisSpec {
        self.axis(axis).cloned().unwrap_or_default()
    }

    pub fn x1(&self) -> AxisSpec {
        self.get(AxisRef::x(1))
    }

    pub fn x2(&self) -> AxisSpec {
        self.get(AxisRef::x(2))
    }

    pub fn y1(&self) -> AxisSpec {
        self.get(AxisRef::y(1))
    }

    pub fn y2(&self) -> AxisSpec {
        self.get(AxisRef::y(2))
    }

    pub fn iter(
        &self,
    ) -> impl Iterator<Item = (&AxisRef, &AxisSpec)> + ExactSizeIterator {
        self.axes.iter()
    }
}

#[derive(Debug, Clone)]
pub struct LayoutSpec {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone)]
pub struct FontSpec {
    pub family: String,
    pub size: usize,
}

#[derive(Debug, Clone)]
pub struct ThemeSpec {
    pub font: Option<FontSpec>,
}

#[derive(Debug, Clone)]
pub struct LegendSpec {
    pub position: String,
    pub font: Option<FontSpec>,
}

#[derive(Debug, Clone)]
pub struct PlotSpec {
    pub title: Option<String>,
    pub layout: LayoutSpec,
    pub theme: ThemeSpec,
    pub legend: LegendSpec,
    pub axes: PlotAxes,
    pub grid: bool,
}

#[derive(Debug, Clone)]
pub struct RenderTarget {
    pub work_dir: PathBuf,
    pub out: Option<PathBuf>,
    pub format_hint: Option<String>,
    pub open: bool,
}

#[derive(Debug, Clone)]
pub struct DataPrepSpec {
    pub inputs: Vec<RegisteredInput>,
    pub series: Vec<SeriesSpec>,
}

#[derive(Debug, Clone, Default)]
pub struct GnuplotBackendOptions {
    pub pre_plot_snippet: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EchartsOutputMode {
    #[default]
    Page,
    Embed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EchartsRuntimeMode {
    #[default]
    Cdn,
    External,
}

#[derive(Debug, Clone, Default)]
pub struct EchartsBackendOptions {
    pub theme: Option<String>,
    pub max_points: Option<usize>,
    pub output_mode: EchartsOutputMode,
    pub runtime_mode: EchartsRuntimeMode,
}

#[derive(Debug, Clone)]
pub enum BackendOptions {
    Gnuplot(GnuplotBackendOptions),
    Echarts(EchartsBackendOptions),
}

#[derive(Debug, Clone)]
pub struct ResolvedMspRequest {
    pub mode: ExecutionMode,
    pub backend: BackendKind,
    pub data_prep: DataPrepSpec,
    pub plot: PlotSpec,
    pub render_target: RenderTarget,
    pub backend_options: BackendOptions,
}

#[derive(Debug, Clone)]
pub struct PreparedSeries {
    pub index: usize,
    pub spec: SeriesSpec,
    pub output_path: PathBuf,
    pub log_path: PathBuf,
}

impl ResolvedMspRequest {
    pub fn describe(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(&mut out, "mode: {:?}", self.mode);
        let _ = writeln!(&mut out, "backend: {:?}", self.backend);
        let _ = writeln!(
            &mut out,
            "work_dir: {}",
            self.render_target.work_dir.display()
        );
        if let Some(path) = &self.render_target.out {
            let _ = writeln!(&mut out, "out: {}", path.display());
        }
        if let Some(format) = &self.render_target.format_hint {
            let _ = writeln!(&mut out, "format_hint: {format}");
        }
        let _ = writeln!(&mut out, "open: {}", self.render_target.open);
        let _ = writeln!(&mut out, "inputs:");
        for input in &self.data_prep.inputs {
            let path = input
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<stdin>".to_string());
            let _ = writeln!(
                &mut out,
                "  - #{} path={} header={:?} format={:?}",
                input.index, path, input.header_presence, input.format
            );
        }
        let _ = writeln!(&mut out, "series:");
        for (idx, series) in self.data_prep.series.iter().enumerate() {
            let _ = writeln!(
                &mut out,
                "  - #{} file={} mark={:?} boxplot_group={:?} axis={} title={:?} x={} y={} if={} of={} opseq={} style={:?}",
                idx + 1,
                series.input_ref,
                series.mark,
                series.boxplot_group,
                series.axis_binding,
                series.name,
                series.x_expr,
                series.y_expr,
                series.input_filter,
                series.output_filter,
                series.opseq,
                series.style.raw
            );
        }
        let _ = writeln!(&mut out, "axes:");
        for (axis_id, axis) in self.plot.axes.iter() {
            let _ = writeln!(
                &mut out,
                "  - {} scale={:?} number_format={:?} range={:?} label={:?} major_tics={:?} custom_tics={:?}",
                axis_id,
                axis.scale,
                axis.number_format,
                axis.range,
                axis.label,
                axis.ticks.major,
                axis.ticks.custom
            );
        }
        out
    }
}
