use std::{fmt::Write, ops::Range, path::PathBuf};

use spreadsheet_plotter::DataFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Plot,
    Prepare,
    DryRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Gnuplot,
    Echarts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesMark {
    Points,
    Lines,
    LinesPoints,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxisId {
    X,
    Y,
    X2,
    Y2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisScale {
    Linear,
    Log10,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisBinding {
    X1Y1,
    X2Y1,
    X1Y2,
    X2Y2,
}

impl AxisBinding {
    pub fn use_x2(self) -> bool {
        matches!(self, Self::X2Y1 | Self::X2Y2)
    }

    pub fn use_y2(self) -> bool {
        matches!(self, Self::X1Y2 | Self::X2Y2)
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
    pub axis_binding: AxisBinding,
    pub input_ref: usize,
    pub input_filter: String,
    pub output_filter: String,
    pub opseq: String,
    pub x_expr: String,
    pub y_expr: String,
    pub mark: SeriesMark,
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
    pub range: Option<Range<f64>>,
    pub label: Option<String>,
    pub ticks: TickSpec,
}

impl Default for AxisSpec {
    fn default() -> Self {
        Self {
            scale: AxisScale::Linear,
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
    pub x: AxisSpec,
    pub y: AxisSpec,
    pub x2: AxisSpec,
    pub y2: AxisSpec,
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
    pub terminal: Option<String>,
    pub pre_plot_snippet: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct EchartsBackendOptions {
    pub theme: Option<String>,
    pub max_points: Option<usize>,
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
                "  - #{} file={} mark={:?} axis={:?} title={:?} x={} y={} if={} of={} opseq={} style={:?}",
                idx + 1,
                series.input_ref,
                series.mark,
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
        out
    }
}
