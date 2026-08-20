use std::fs;

use anyhow::{Context, bail};

use crate::{
    backend::{Backend, RenderPlan, maybe_open_in_browser, write_artifact},
    spec::{
        AxisScale, BackendOptions, EchartsBackendOptions, PreparedSeries,
        ResolvedMspRequest, SeriesMark,
    },
};

pub struct EchartsBackend;

const DEFAULT_MAX_POINTS_PER_SERIES: usize = 10_000;

#[derive(Debug)]
struct CsvSeriesData {
    x_label: String,
    y_label: String,
    x_numeric: bool,
    points: Vec<(String, f64)>,
    original_point_count: usize,
}

fn escape_js_string(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('"', "\\\"")
}

fn parse_csv_row(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    current.push('"');
                    let _ = chars.next();
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                out.push(current);
                current = String::new();
            }
            _ => current.push(ch),
        }
    }
    out.push(current);
    out
}

fn parse_series_csv(path: &std::path::Path) -> anyhow::Result<CsvSeriesData> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read prepared data '{}'", path.display()))?;
    let mut lines = content.lines().filter(|line| !line.trim().is_empty());
    let header = lines
        .next()
        .with_context(|| format!("Prepared data '{}' is empty", path.display()))?;
    let header = parse_csv_row(header);
    if header.len() < 2 {
        bail!("Prepared data '{}' does not contain two columns", path.display());
    }

    let mut points = Vec::new();
    let mut x_numeric = true;
    for line in lines {
        let row = parse_csv_row(line);
        if row.len() < 2 {
            continue;
        }
        let x_raw = row[0].clone();
        if x_raw.parse::<f64>().is_err() {
            x_numeric = false;
        }
        let y = row[1].parse::<f64>().with_context(|| {
            format!("Failed to parse y value '{}' from '{}'", row[1], path.display())
        })?;
        points.push((x_raw, y));
    }

    Ok(CsvSeriesData {
        x_label: header[0].clone(),
        y_label: header[1].clone(),
        x_numeric,
        original_point_count: points.len(),
        points,
    })
}

fn downsample_points(
    points: &[(String, f64)],
    max_points: usize,
) -> Vec<(String, f64)> {
    if max_points == 0 || points.len() <= max_points {
        return points.to_vec();
    }

    if max_points == 1 {
        return vec![points[0].clone()];
    }

    let last_index = points.len() - 1;
    let mut sampled = Vec::with_capacity(max_points);
    for i in 0..max_points {
        let index = i * last_index / (max_points - 1);
        if sampled
            .last()
            .is_none_or(|last: &(String, f64)| last != &points[index])
        {
            sampled.push(points[index].clone());
        }
    }
    sampled
}

#[allow(dead_code)]
fn axis_type(is_numeric: bool, scale: AxisScale) -> &'static str {
    if is_numeric {
        match scale {
            AxisScale::Linear => "value",
            AxisScale::Log10 => "log",
        }
    } else {
        "category"
    }
}

fn build_html(
    request: &ResolvedMspRequest,
    prepared: &[PreparedSeries],
    options: &EchartsBackendOptions,
) -> anyhow::Result<String> {
    let max_points = options.max_points.unwrap_or(DEFAULT_MAX_POINTS_PER_SERIES);
    let parsed = prepared
        .iter()
        .map(|series| parse_series_csv(&series.output_path))
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .map(|mut series| {
            series.points = downsample_points(&series.points, max_points);
            series
        })
        .collect::<Vec<_>>();
    let primary_x_numeric = parsed.first().map(|series| series.x_numeric).unwrap_or(true);

    let x_label = parsed.first().map(|series| series.x_label.clone()).unwrap_or_else(|| "x".to_string());
    let y_label = parsed.first().map(|series| series.y_label.clone()).unwrap_or_else(|| "y".to_string());

    // Extract unique labels for category X axes
    let labels: Vec<String> = if !primary_x_numeric {
        let mut seen = std::collections::HashSet::new();
        parsed.first().map(|series| {
            series.points.iter().filter_map(|(x, _)| {
                if seen.insert(x.clone()) { Some(x.clone()) } else { None }
            }).collect()
        }).unwrap_or_default()
    } else {
        Vec::new()
    };

    let labels_js = labels.iter().map(|l| format!("\"{}\"", escape_js_string(l))).collect::<Vec<_>>().join(",");

    const COLORS: [&str; 6] = ["#3b82f6", "#10b981", "#f59e0b", "#8b5cf6", "#ef4444", "#ec4899"];

    // Build series data JS
    let series_data_js = prepared.iter().zip(parsed.iter()).enumerate().map(|(idx, (prepared, parsed))| {
        let name = escape_js_string(prepared.spec.name.as_deref().unwrap_or(&format!("series {}", idx + 1)));
        let color = COLORS[idx % COLORS.len()];
        let chart_type = match prepared.spec.mark {
            SeriesMark::Points => "scatter",
            SeriesMark::Lines | SeriesMark::LinesPoints => "line",
        };
        let x_axis_key = if prepared.spec.axis_binding.use_x2() { "x-top" } else { "x-bottom" };
        let y_axis_key = if prepared.spec.axis_binding.use_y2() { "y-right-1" } else { "y-left" };
        let show_symbol = !matches!(prepared.spec.mark, SeriesMark::Lines);
        let symbol_size = if matches!(prepared.spec.mark, SeriesMark::Points) { 12 } else { 7 };

        let values = if primary_x_numeric {
            let points = parsed.points.iter().map(|(x, y)| format!("[{},{}]", x, y)).collect::<Vec<_>>().join(",");
            format!("[{}]", points)
        } else {
            match prepared.spec.mark {
                SeriesMark::Lines | SeriesMark::LinesPoints => {
                    let points = parsed.points.iter().map(|(_, y)| y.to_string()).collect::<Vec<_>>().join(",");
                    format!("[{}]", points)
                }
                SeriesMark::Points => {
                    let points = parsed.points.iter().map(|(x, y)| format!("[\"{}\",{}]", escape_js_string(x), y)).collect::<Vec<_>>().join(",");
                    format!("[{}]", points)
                }
            }
        };

        let mut parts = vec![
            format!("name:\"{name}\""),
            format!("type:\"{chart_type}\""),
            format!("values:{values}"),
            format!("color:\"{color}\""),
            format!("xAxisKey:\"{x_axis_key}\""),
            format!("yAxisKey:\"{y_axis_key}\""),
            format!("symbol:\"circle\""),
            format!("symbolSize:{symbol_size}"),
        ];
        if matches!(prepared.spec.mark, SeriesMark::Lines | SeriesMark::LinesPoints) {
            parts.push("lineWidth:3".to_string());
        }
        if !show_symbol {
            parts.push("showSymbol:false".to_string());
        }
        format!("{{{}}}", parts.join(","))
    }).collect::<Vec<_>>().join(",\n");

    // Build layout cells
    let mut cells: Vec<String> = Vec::new();
    let bfs = request.plot.theme.font.as_ref().map(|f| f.size as f64).unwrap_or(12.0);

    // Title cell
    let title_text = prepared.iter().filter_map(|s| s.spec.name.as_deref()).collect::<Vec<_>>().join(", ");
    let has_title = !title_text.is_empty();
    if has_title {
        cells.push(format!(
            r#"{{id:"title",kind:"title",side:"top",track:2,size:{},minorSpan:"stretch",align:"center",renderArea:{{zIndex:0,size:[{{type:"percent",value:0}},{{type:"percent",value:100}}]}},text:"{}"}}"#,
            bfs * 2.5, escape_js_string(&title_text),
        ));
    }

    // Legend cell
    if prepared.len() >= 2 {
        cells.push(format!(r#"{{id:"legend",kind:"legend",side:"top",track:1,size:{},minorSpan:"stretch"}}"#, bfs * 2.5));
    }

    // X axis cells
    let has_x_bottom = prepared.iter().any(|s| !s.spec.axis_binding.use_x2());
    let has_x_top = prepared.iter().any(|s| s.spec.axis_binding.use_x2());
    let x_axis_name = escape_js_string(request.plot.axes.x.label.as_deref().unwrap_or(&x_label));
    let x2_axis_name = escape_js_string(request.plot.axes.x2.label.as_deref().unwrap_or(request.plot.axes.x.label.as_deref().unwrap_or(&x_label)));

    if has_x_bottom {
        if primary_x_numeric {
            cells.push(format!(r#"{{id:"x-bottom",kind:"axis",side:"bottom",track:0,size:{},minorSpan:"stretch",axisDimension:"x",name:"{}",axisOffset:8,labelMargin:16,nameGap:38,visibilityPolicy:"if-any-bound-series-visible"}}"#, bfs * 4.5, x_axis_name));
        } else {
            cells.push(format!(r#"{{id:"x-bottom",kind:"axis",side:"bottom",track:0,size:{},minorSpan:"stretch",axisDimension:"x",name:"{}",data:[{}],axisOffset:8,labelMargin:16,nameGap:38,visibilityPolicy:"if-any-bound-series-visible"}}"#, bfs * 4.5, x_axis_name, labels_js));
        }
    }
    if has_x_top {
        if primary_x_numeric {
            cells.push(format!(r#"{{id:"x-top",kind:"axis",side:"top",track:0,size:{},minorSpan:"stretch",axisDimension:"x",name:"{}",axisOffset:8,labelMargin:16,nameGap:38,visibilityPolicy:"if-any-bound-series-visible"}}"#, bfs * 4.5, x2_axis_name));
        } else {
            cells.push(format!(r#"{{id:"x-top",kind:"axis",side:"top",track:0,size:{},minorSpan:"stretch",axisDimension:"x",name:"{}",data:[{}],axisOffset:8,labelMargin:16,nameGap:38,visibilityPolicy:"if-any-bound-series-visible"}}"#, bfs * 4.5, x2_axis_name, labels_js));
        }
    }

    // Data zoom cell
    if has_x_bottom || has_x_top {
        cells.push(format!(r#"{{id:"x-scale",kind:"data-zoom",side:"bottom",track:1,size:{},minorSpan:"stretch",align:"center"}}"#, bfs * 3.0));
    }

    // Y axis cells
    let has_y_left = prepared.iter().any(|s| !s.spec.axis_binding.use_y2());
    let has_y_right = prepared.iter().any(|s| s.spec.axis_binding.use_y2());
    let y_axis_name = escape_js_string(request.plot.axes.y.label.as_deref().unwrap_or(&y_label));
    let y2_axis_name = escape_js_string(request.plot.axes.y2.label.as_deref().unwrap_or(request.plot.axes.y.label.as_deref().unwrap_or(&y_label)));

    if has_y_left {
        cells.push(format!(r#"{{id:"y-left",kind:"axis",side:"left",track:0,size:{},minorSpan:"stretch",axisDimension:"y",name:"{}",labelMargin:10,visibilityPolicy:"if-any-bound-series-visible"}}"#, bfs * 5.2, y_axis_name));
    }
    if has_y_right {
        cells.push(format!(r#"{{id:"y-right-1",kind:"axis",side:"right",track:0,size:{},minorSpan:"stretch",axisDimension:"y",name:"{}",labelMargin:10,visibilityPolicy:"if-any-bound-series-visible"}}"#, bfs * 5.2, y2_axis_name));
    }

    let cells_js = cells.join(",\n");

    // Font
    let font_family = request.plot.theme.font.as_ref().map(|f| escape_js_string(&f.family)).unwrap_or_else(|| "sans-serif".to_string());
    let font_size = request.plot.theme.font.as_ref().map(|f| f.size).unwrap_or(12);

    // Axis scale mode initialization
    let y_log_init = request.plot.axes.y.scale == AxisScale::Log10;
    let y2_log_init = request.plot.axes.y2.scale == AxisScale::Log10;
    let mut scale_mode_overrides = Vec::new();
    if has_y_left && y_log_init { scale_mode_overrides.push("\"y-left\":true".to_string()); }
    if has_y_right && y2_log_init { scale_mode_overrides.push("\"y-right-1\":true".to_string()); }
    let scale_mode_overrides_js = if scale_mode_overrides.is_empty() {
        String::new()
    } else {
        format!("\nObject.assign(axisScaleMode, {{{}}});", scale_mode_overrides.join(","))
    };

    // Axis ranges
    let mut axis_range_entries = Vec::new();
    if has_y_left {
        let (min, max) = match &request.plot.axes.y.range {
            Some(r) => (format!("{}", r.start), format!("{}", r.end)),
            None => ("undefined".to_string(), "undefined".to_string()),
        };
        axis_range_entries.push(format!("\"y-left\":{{min:{},max:{}}}", min, max));
    }
    if has_y_right {
        let (min, max) = match &request.plot.axes.y2.range {
            Some(r) => (format!("{}", r.start), format!("{}", r.end)),
            None => ("undefined".to_string(), "undefined".to_string()),
        };
        axis_range_entries.push(format!("\"y-right-1\":{{min:{},max:{}}}", min, max));
    }
    if has_x_bottom && primary_x_numeric {
        let (min, max) = match &request.plot.axes.x.range {
            Some(r) => (format!("{}", r.start), format!("{}", r.end)),
            None => ("undefined".to_string(), "undefined".to_string()),
        };
        axis_range_entries.push(format!("\"x-bottom\":{{min:{},max:{}}}", min, max));
    }
    if has_x_top && primary_x_numeric {
        let (min, max) = match &request.plot.axes.x2.range {
            Some(r) => (format!("{}", r.start), format!("{}", r.end)),
            None => ("undefined".to_string(), "undefined".to_string()),
        };
        axis_range_entries.push(format!("\"x-top\":{{min:{},max:{}}}", min, max));
    }
    let axis_ranges_js = format!("{{{}}}", axis_range_entries.join(","));

    let total_original_points = parsed.iter().map(|s| s.original_point_count).sum::<usize>();
    let total_embedded_points = parsed.iter().map(|s| s.points.len()).sum::<usize>();
    let theme = escape_js_string(options.theme.as_deref().unwrap_or("default"));

    // Build the complete HTML
    Ok(format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>msp echarts</title>
<style>
:root {{
  --bg: #f5f7fb;
  --panel: #ffffff;
  --text: #172033;
  --muted: #5f6b85;
  --grid: #e6ebf5;
  --accent: #3b82f6;
  --shadow: 0 18px 40px rgba(23, 32, 51, 0.08);
}}
* {{ box-sizing: border-box; }}
body {{
  margin: 0;
  min-height: 100vh;
  padding: 32px;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  background: linear-gradient(180deg, #f8faff 0%, #f2f5fb 100%);
  color: var(--text);
}}
.page {{ max-width: 960px; margin: 0 auto; }}
.panel {{
  background: var(--panel);
  border: 1px solid rgba(95, 107, 133, 0.12);
  border-radius: 20px;
  box-shadow: var(--shadow);
  padding: 28px;
}}
#chart {{ width: 100%; height: 480px; }}
@media (max-width: 640px) {{
  body {{ padding: 16px; }}
  .panel {{ padding: 20px; border-radius: 16px; }}
  #chart {{ height: 360px; }}
}}
</style>
</head>
<body>
<main class="page">
<section class="panel">
<div id="chart" aria-label="msp echarts chart"></div>
</section>
</main>
<script src="https://cdn.jsdelivr.net/npm/echarts@5/dist/echarts.min.js"></script>
<script>
const labels = [{labels_js}];
const seriesData = [
{series_data_js}
];
const seriesVisibility = Object.fromEntries(
  seriesData.map(function(s) {{ return [s.name, true]; }}),
);
var hoveredSeriesName = null;
var hoverResetTimer = null;
const gridDebug = false;
const axisRanges = {axis_ranges_js};

const layoutSpec = {{
  padding: {{ top: 18, right: 18, bottom: 18, left: 18 }},
  gap: 10,
  cells: [
{cells_js}
  ],
}};

const chartNode = document.getElementById("chart");
const chart = echarts.init(chartNode, "{theme}");
const measureCanvas = document.createElement("canvas");
const measureContext = measureCanvas.getContext("2d");
const layoutTheme = {{
  titleFont: '600 20px "{font_family}", sans-serif',
  axisLabelFont: '{font_size}px "{font_family}", sans-serif',
  axisNameFont: '{font_size}px "{font_family}", sans-serif',
  axisLogFont: '600 10px "{font_family}", sans-serif',
  legendFont: '{font_size}px "{font_family}", sans-serif',
  titlePadding: 8,
  axisPadding: 10,
  axisTickLengthX: 8,
  axisTickLengthY: 6,
  legendItemWidth: 25,
  legendItemHeight: 14,
  legendInnerGap: 8,
  legendItemGap: 12,
  legendRowGap: 6,
  legendPadding: 6,
  legendWidthBuffer: 12,
}};
const numericAxisCells = layoutSpec.cells.filter(
  function(cell) {{ return cell.kind === "axis" && cell.axisDimension === "y"; }},
);
const titledAxisCells = layoutSpec.cells.filter(function(cell) {{ return cell.kind === "axis"; }});
const axisScaleMode = Object.fromEntries(
  numericAxisCells.map(function(cell) {{ return [cell.id, false]; }}),
);{scale_mode_overrides}

function getFontSize(font) {{
  const match = font.match(/(\d+(?:\.\d+)?)px/);
  return match ? Number.parseFloat(match[1]) : 12;
}}

function measureText(text, font) {{
  const value = text || "";
  measureContext.font = font;
  const metrics = measureContext.measureText(value);
  const fallbackHeight = getFontSize(font) * 1.2;
  const height = metrics.actualBoundingBoxAscent && metrics.actualBoundingBoxDescent
    ? metrics.actualBoundingBoxAscent + metrics.actualBoundingBoxDescent
    : fallbackHeight;
  return {{ width: metrics.width, height: height }};
}}

function getSeriesBoundToAxis(axisId, axisDimension) {{
  const axisKey = axisDimension === "x" ? "xAxisKey" : "yAxisKey";
  return seriesData.filter(function(s) {{ return s[axisKey] === axisId; }});
}}

function usesBandPositioning(seriesItem) {{
  return seriesItem.type === "bar" || seriesItem.type === "boxplot";
}}

function shouldUseCategoryBoundaryGap(axisId) {{
  return getSeriesBoundToAxis(axisId, "x").some(usesBandPositioning);
}}

function collectSeriesNumericValues(seriesItem) {{
  if (!Array.isArray(seriesItem.values)) {{ return []; }}
  return seriesItem.values.flatMap(function(value) {{
    if (typeof value === "number") {{ return [value]; }}
    if (!Array.isArray(value)) {{ return []; }}
    if (seriesItem.type === "scatter") {{ return typeof value[1] === "number" ? [value[1]] : []; }}
    return value.filter(function(item) {{ return typeof item === "number"; }});
  }});
}}

function getPositiveAxisValues(axisId) {{
  return getSeriesBoundToAxis(axisId, "y")
    .flatMap(collectSeriesNumericValues)
    .filter(function(value) {{ return value > 0; }});
}}

function getLogAxisMin(axisId) {{
  const positiveValues = getPositiveAxisValues(axisId);
  if (positiveValues.length === 0) {{ return 1; }}
  return Math.min.apply(null, positiveValues);
}}

function getValueAxisNameGap(cell, bandSize) {{
  return cell.nameGap || Math.ceil(getFontSize(layoutTheme.axisLabelFont) * 1.5);
}}

function getCategoryAxisNameGap(cell, bandSize) {{
  return cell.nameGap || Math.max(28, bandSize - 18);
}}

function toggleAxisLogScale(axisId) {{
  if (!(axisId in axisScaleMode)) {{ return; }}
  axisScaleMode[axisId] = !axisScaleMode[axisId];
  renderChart();
}}

function normalizeMajorPadding(majorPadding) {{
  return {{ before: (majorPadding && majorPadding.before) || 0, after: (majorPadding && majorPadding.after) || 0 }};
}}

function getMajorPadding(cell) {{
  const padding = normalizeMajorPadding(cell.majorPadding);
  return padding.before + padding.after;
}}

function measureLegendLayout(availableWidth) {{
  const safeWidth = Math.max(120, availableWidth);
  const itemWidths = seriesData.map(function(s) {{
    const labelMetrics = measureText(s.name, layoutTheme.legendFont);
    return layoutTheme.legendItemWidth + layoutTheme.legendInnerGap + labelMetrics.width;
  }});
  var rows = 1;
  var rowWidth = 0;
  var maxRowWidth = 0;
  itemWidths.forEach(function(itemWidth, index) {{
    const nextWidth = rowWidth + (index === 0 || rowWidth === 0 ? 0 : layoutTheme.legendItemGap) + itemWidth;
    if (nextWidth > safeWidth && rowWidth > 0) {{
      maxRowWidth = Math.max(maxRowWidth, rowWidth);
      rows += 1;
      rowWidth = itemWidth;
      return;
    }}
    rowWidth = nextWidth;
  }});
  maxRowWidth = Math.max(maxRowWidth, rowWidth);
  const rowHeight = measureText("Series A", layoutTheme.legendFont).height + layoutTheme.legendPadding;
  return {{
    width: Math.min(safeWidth, maxRowWidth + layoutTheme.legendWidthBuffer),
    height: rows * rowHeight + (rows - 1) * layoutTheme.legendRowGap,
  }};
}}

function getAxisTitleLayout(cell) {{
  const titleMetrics = measureText(cell.name, layoutTheme.axisNameFont);
  const showsLogToggle = cell.axisDimension === "y";
  const logMetrics = showsLogToggle ? measureText("Log", layoutTheme.axisLogFont) : {{ width: 0, height: 0 }};
  const gap = showsLogToggle ? 6 : 0;
  const totalWidth = titleMetrics.width + (showsLogToggle ? gap + logMetrics.width : 0);
  const thickness = Math.max(titleMetrics.height, logMetrics.height);
  return {{
    titleMetrics: titleMetrics,
    logMetrics: logMetrics,
    gap: gap,
    totalWidth: totalWidth,
    thickness: thickness,
  }};
}}

function getYAxisTitleGap(cell) {{
  return cell.nameGap || Math.ceil(getFontSize(layoutTheme.axisLabelFont) * 1.5);
}}

function measureAxisSize(cell) {{
  if (cell.axisDimension === "x") {{
    const sampleLabels = (cell.data && cell.data.length > 0) ? cell.data : ["0", "10", "100", "1000"];
    const labelHeight = Math.max.apply(null, sampleLabels.map(function(label) {{
      return measureText(String(label), layoutTheme.axisLabelFont).height;
    }}));
    const nameHeight = measureText(cell.name, layoutTheme.axisNameFont).height;
    const labelBlock = (cell.axisOffset || 0) + layoutTheme.axisTickLengthX + (cell.labelMargin || 10) + labelHeight;
    const nameBlock = (cell.nameGap || 28) + nameHeight;
    return Math.ceil(Math.max(labelBlock, nameBlock) + layoutTheme.axisPadding);
  }}
  const boundSeries = getSeriesBoundToAxis(cell.id, "y");
  const candidateLabels = new Set(["0"]);
  boundSeries.forEach(function(s) {{
    collectSeriesNumericValues(s).forEach(function(value) {{ candidateLabels.add(String(value)); }});
  }});
  const widestLabel = Math.max.apply(null, Array.from(candidateLabels).map(function(label) {{
    return measureText(label, layoutTheme.axisLabelFont).width;
  }}));
  const titleLayout = getAxisTitleLayout(cell);
  const labelBlock = layoutTheme.axisTickLengthY + (cell.labelMargin || 10) + widestLabel;
  const titleBlock = cell.name ? getYAxisTitleGap(cell) + titleLayout.thickness : 0;
  return Math.ceil(labelBlock + titleBlock + layoutTheme.axisPadding);
}}

function measureCellMajorSize(cell, plotWidthEstimate) {{
  if (cell.kind === "axis") {{ return measureAxisSize(cell); }}
  if (cell.kind === "title") {{
    const textMetrics = measureText(cell.text, layoutTheme.titleFont);
    return Math.ceil(textMetrics.height + layoutTheme.titlePadding);
  }}
  if (cell.kind === "legend") {{
    const availableWidth = resolveMinorSpan(cell.minorSpan, plotWidthEstimate);
    return Math.ceil(measureLegendLayout(availableWidth).height);
  }}
  if (cell.kind === "data-zoom") {{ return cell.size || 36; }}
  return cell.size || 0;
}}

function resolveMeasuredSpec(width, height, spec) {{
  const measuredCells = spec.cells.map(function(cell) {{ return Object.assign({{}}, cell); }});
  measuredCells.forEach(function(cell) {{
    if (cell.side === "left" || cell.side === "right") {{
      cell.resolvedSize = Math.max(cell.size || 0, measureCellMajorSize(cell, width)) + getMajorPadding(cell);
    }}
  }});
  const sideOnlySpec = Object.assign({{}}, spec, {{ cells: measuredCells }});
  const sideTracks = buildTrackMap(sideOnlySpec.cells);
  const plotWidthEstimate = width - spec.padding.left - spec.padding.right
    - getReservedSpace(sideTracks.left, spec.gap) - getReservedSpace(sideTracks.right, spec.gap);
  measuredCells.forEach(function(cell) {{
    if (cell.side === "top" || cell.side === "bottom") {{
      cell.resolvedSize = Math.max(cell.size || 0, measureCellMajorSize(cell, plotWidthEstimate)) + getMajorPadding(cell);
    }}
  }});
  return Object.assign({{}}, spec, {{ cells: measuredCells }});
}}

function resolveMinorSpan(span, availableSize) {{
  if (span === "stretch" || span === undefined) {{ return availableSize; }}
  if (typeof span === "number") {{ return Math.min(span, availableSize); }}
  if (typeof span === "string" && span.endsWith("%")) {{ return (availableSize * Number.parseFloat(span)) / 100; }}
  return availableSize;
}}

function isHorizontalCell(cell) {{ return cell.side === "top" || cell.side === "bottom"; }}

function isOrthogonalCell(baseCell, targetCell) {{
  if (!targetCell) {{ return false; }}
  return isHorizontalCell(baseCell)
    ? targetCell.side === "left" || targetCell.side === "right"
    : targetCell.side === "top" || targetCell.side === "bottom";
}}

function getEdgeFromCellRect(targetCell, edge) {{
  if (targetCell.side === "left" || targetCell.side === "right") {{
    return edge === "end" ? targetCell.rect.x + targetCell.rect.width : targetCell.rect.x;
  }}
  return edge === "end" ? targetCell.rect.y + targetCell.rect.height : targetCell.rect.y;
}}

function resolveRenderEdgeValue(edgeValue, cell, layoutRect, cellsById, fallback) {{
  if (edgeValue === undefined || edgeValue === null) {{ return fallback; }}
  if (typeof edgeValue === "number") {{ return fallback; }}
  if (typeof edgeValue === "string" && edgeValue.endsWith("%")) {{
    const percent = Number.parseFloat(edgeValue) / 100;
    return isHorizontalCell(cell) ? layoutRect.x + layoutRect.width * percent : layoutRect.y + layoutRect.height * percent;
  }}
  if (edgeValue.type === "percent") {{
    const percent = edgeValue.value / 100;
    return isHorizontalCell(cell) ? layoutRect.x + layoutRect.width * percent : layoutRect.y + layoutRect.height * percent;
  }}
  if (edgeValue.type === "align-cell") {{
    const targetCell = cellsById[edgeValue.cellId];
    if (!isOrthogonalCell(cell, targetCell)) {{ return fallback; }}
    return getEdgeFromCellRect(targetCell, edgeValue.edge || "start");
  }}
  return fallback;
}}

function buildRenderRect(cell, plotRect, layoutRect, cellsById) {{
  const baseRect = cell.contentRect;
  const renderArea = cell.renderArea;
  if (!renderArea || !Array.isArray(renderArea.size) || renderArea.size.length !== 2) {{
    if (cell.kind === "axis") {{
      if (isHorizontalCell(cell)) {{
        return {{ x: plotRect.x, y: baseRect.y, width: plotRect.width, height: baseRect.height, zIndex: 0 }};
      }}
      return {{ x: baseRect.x, y: plotRect.y, width: baseRect.width, height: plotRect.height, zIndex: 0 }};
    }}
    return Object.assign({{}}, baseRect, {{ zIndex: 0 }});
  }}
  const startFallback = isHorizontalCell(cell) ? baseRect.x : baseRect.y;
  const endFallback = isHorizontalCell(cell) ? baseRect.x + baseRect.width : baseRect.y + baseRect.height;
  const resolvedStart = resolveRenderEdgeValue(renderArea.size[0], cell, layoutRect, cellsById, startFallback);
  const resolvedEnd = resolveRenderEdgeValue(renderArea.size[1], cell, layoutRect, cellsById, endFallback);
  const start = Math.min(resolvedStart, resolvedEnd);
  const end = Math.max(resolvedStart, resolvedEnd);
  if (isHorizontalCell(cell)) {{
    return {{ x: start, y: baseRect.y, width: Math.max(0, end - start), height: baseRect.height, zIndex: renderArea.zIndex || 0 }};
  }}
  return {{ x: baseRect.x, y: start, width: baseRect.width, height: Math.max(0, end - start), zIndex: renderArea.zIndex || 0 }};
}}

function alignAlong(start, availableSize, itemSize, align) {{
  if (align === "end") {{ return start + availableSize - itemSize; }}
  if (align === "center") {{ return start + (availableSize - itemSize) / 2; }}
  return start;
}}

function buildTrackMap(cells) {{
  const sides = {{ top: new Map(), right: new Map(), bottom: new Map(), left: new Map() }};
  cells.forEach(function(cell) {{
    const sideTracks = sides[cell.side];
    const track = sideTracks.get(cell.track) || {{ track: cell.track, size: 0, cells: [] }};
    const majorSize = cell.resolvedSize || cell.size;
    track.size = Math.max(track.size, majorSize);
    track.cells.push(cell);
    sideTracks.set(cell.track, track);
  }});
  return Object.fromEntries(
    Object.entries(sides).map(function(entry) {{
      return [entry[0], Array.from(entry[1].values()).sort(function(a, b) {{ return a.track - b.track; }})];
    }}),
  );
}}

function getReservedSpace(tracks, gap) {{
  if (tracks.length === 0) {{ return 0; }}
  return tracks.reduce(function(sum, track) {{ return sum + track.size; }}, 0) + gap * (tracks.length - 1);
}}

function computeCrossLayout(width, height, spec) {{
  const layoutRect = {{
    x: spec.padding.left, y: spec.padding.top,
    width: width - spec.padding.left - spec.padding.right,
    height: height - spec.padding.top - spec.padding.bottom,
  }};
  const tracksBySide = buildTrackMap(spec.cells);
  const reserved = {{
    top: getReservedSpace(tracksBySide.top, spec.gap),
    right: getReservedSpace(tracksBySide.right, spec.gap),
    bottom: getReservedSpace(tracksBySide.bottom, spec.gap),
    left: getReservedSpace(tracksBySide.left, spec.gap),
  }};
  const plotRect = {{
    x: spec.padding.left + reserved.left,
    y: spec.padding.top + reserved.top,
    width: width - spec.padding.left - spec.padding.right - reserved.left - reserved.right,
    height: height - spec.padding.top - spec.padding.bottom - reserved.top - reserved.bottom,
  }};
  const cellsById = {{}};
  const trackOffsets = {{}};
  function placeTrack(side, track, bandOffset) {{
    var bandRect;
    if (side === "top") {{
      bandRect = {{ x: plotRect.x, y: plotRect.y - bandOffset - track.size, width: plotRect.width, height: track.size }};
    }} else if (side === "bottom") {{
      bandRect = {{ x: plotRect.x, y: plotRect.y + plotRect.height + bandOffset, width: plotRect.width, height: track.size }};
    }} else if (side === "left") {{
      bandRect = {{ x: plotRect.x - bandOffset - track.size, y: plotRect.y, width: track.size, height: plotRect.height }};
    }} else {{
      bandRect = {{ x: plotRect.x + plotRect.width + bandOffset, y: plotRect.y, width: track.size, height: plotRect.height }};
    }}
    track.cells.forEach(function(cell) {{
      const align = cell.align || "start";
      const majorSize = cell.resolvedSize || cell.size;
      const majorPadding = normalizeMajorPadding(cell.majorPadding);
      if (side === "top" || side === "bottom") {{
        const cellWidth = resolveMinorSpan(cell.minorSpan, plotRect.width);
        const cellX = alignAlong(plotRect.x, plotRect.width, cellWidth, align);
        const cellY = bandRect.y + (bandRect.height - majorSize) / 2;
        cellsById[cell.id] = Object.assign({{}}, cell, {{
          rect: {{ x: cellX, y: cellY, width: cellWidth, height: majorSize }},
          contentRect: {{
            x: cellX, y: cellY + majorPadding.before,
            width: cellWidth, height: Math.max(0, majorSize - majorPadding.before - majorPadding.after),
          }},
          bandRect: bandRect,
        }});
      }} else {{
        const cellHeight = resolveMinorSpan(cell.minorSpan, plotRect.height);
        const cellX = bandRect.x + (bandRect.width - majorSize) / 2;
        const cellY = alignAlong(plotRect.y, plotRect.height, cellHeight, align);
        cellsById[cell.id] = Object.assign({{}}, cell, {{
          rect: {{ x: cellX, y: cellY, width: majorSize, height: cellHeight }},
          contentRect: {{
            x: cellX + majorPadding.before, y: cellY,
            width: Math.max(0, majorSize - majorPadding.before - majorPadding.after), height: cellHeight,
          }},
          bandRect: bandRect,
        }});
      }}
      trackOffsets[cell.id] = bandOffset;
    }});
  }}
  ["top", "right", "bottom", "left"].forEach(function(side) {{
    var bandOffset = 0;
    tracksBySide[side].forEach(function(track, index) {{
      placeTrack(side, track, bandOffset);
      bandOffset += track.size;
      if (index < tracksBySide[side].length - 1) {{ bandOffset += spec.gap; }}
    }});
  }});
  Object.values(cellsById).forEach(function(cell) {{
    cell.renderRect = buildRenderRect(cell, plotRect, layoutRect, cellsById);
  }});
  return {{ layoutRect: layoutRect, plotRect: plotRect, cellsById: cellsById, trackOffsets: trackOffsets }};
}}

function toPercent(value, total) {{
  return ((value / total) * 100) + "%";
}}

function buildAxisOption(cell, trackOffset, isVisible) {{
  const isX = cell.axisDimension === "x";
  const isLogScale = !isX && Boolean(axisScaleMode[cell.id]);
  const bandSize = cell.bandRect ? (isX ? cell.bandRect.height : cell.bandRect.width) : cell.size;
  const useBoundaryGap = isX ? shouldUseCategoryBoundaryGap(cell.id) : false;
  const isNumericX = isX && !cell.data;
  const range = axisRanges[cell.id] || {{}};
  if (isX) {{
    return {{
      id: cell.id,
      show: isVisible,
      type: isNumericX ? "value" : "category",
      position: cell.side,
      offset: trackOffset + (cell.axisOffset || 0),
      name: "",
      nameLocation: "middle",
      nameGap: getCategoryAxisNameGap(cell, bandSize),
      boundaryGap: isNumericX ? false : useBoundaryGap,
      data: isNumericX ? undefined : cell.data,
      min: isNumericX ? (range.min !== undefined ? range.min : undefined) : undefined,
      max: isNumericX ? (range.max !== undefined ? range.max : undefined) : undefined,
      axisLine: {{ show: isVisible, lineStyle: {{ color: cell.side === "top" ? "#d7deeb" : "#c7d2e5" }} }},
      axisTick: {{ show: isVisible, length: 8, alignWithLabel: isNumericX ? undefined : useBoundaryGap }},
      axisLabel: {{ show: isVisible, color: cell.side === "top" ? "#7b879c" : "#5f6b85", margin: cell.labelMargin || 10 }},
      splitLine: {{ show: false }},
    }};
  }}
  return {{
    id: cell.id,
    show: isVisible,
    type: isLogScale ? "log" : "value",
    position: cell.side,
    offset: trackOffset,
    name: "",
    nameLocation: "middle",
    nameGap: getValueAxisNameGap(cell, bandSize),
    min: isLogScale ? getLogAxisMin(cell.id) : (range.min !== undefined ? range.min : undefined),
    max: isLogScale ? undefined : (range.max !== undefined ? range.max : undefined),
    logBase: isLogScale ? 10 : undefined,
    axisLine: {{ show: isVisible, lineStyle: {{ color: cell.side === "left" ? "#c7d2e5" : "#d7deeb" }} }},
    axisTick: {{ show: isVisible, length: 6 }},
    axisLabel: {{ show: isVisible, color: cell.side === "left" ? "#5f6b85" : "#7b879c", margin: cell.labelMargin || 10 }},
    splitLine: {{ show: false, lineStyle: {{ color: "#e6ebf5" }} }},
  }};
}}

function buildAxisTitleGraphic(cell) {{
  const titleLayout = getAxisTitleLayout(cell);
  const titleMetrics = titleLayout.titleMetrics;
  const showsLogToggle = cell.axisDimension === "y";
  const logMetrics = titleLayout.logMetrics;
  const gap = titleLayout.gap;
  const isEnabled = showsLogToggle && Boolean(axisScaleMode[cell.id]);
  const totalWidth = titleLayout.totalWidth;
  const inwardOffset = titleLayout.thickness / 2;
  const titleX = -totalWidth / 2;
  const logX = titleX + titleMetrics.width + gap;
  var anchorX = cell.renderRect.x + cell.renderRect.width / 2;
  var anchorY = cell.renderRect.y + cell.renderRect.height / 2;
  var rotation = 0;
  if (cell.axisDimension === "y") {{
    anchorX = cell.side === "left"
      ? cell.renderRect.x + inwardOffset
      : cell.renderRect.x + cell.renderRect.width - inwardOffset;
    rotation = cell.side === "left" ? Math.PI / 2 : -Math.PI / 2;
  }} else {{
    anchorY = cell.side === "top" ? cell.renderRect.y + inwardOffset : cell.renderRect.y + cell.renderRect.height - inwardOffset;
  }}
  return [
    {{
      type: "group",
      id: "axis-title-" + cell.id,
      x: anchorX, y: anchorY, rotation: rotation, z: 30,
      children: [
        {{
          type: "text", silent: true, x: titleX, y: 0,
          style: {{ text: cell.name, fill: "#172033", font: layoutTheme.axisNameFont, textAlign: "left", textVerticalAlign: "middle" }},
        }}
      ].concat(showsLogToggle ? [
        {{
          type: "text", x: logX, y: 0, cursor: "pointer",
          onclick: function() {{ toggleAxisLogScale(cell.id); }},
          style: {{ text: "Log", fill: isEnabled ? "#172033" : "#9ca3af", font: layoutTheme.axisLogFont, textAlign: "left", textVerticalAlign: "middle" }},
        }}
      ] : []),
    }},
  ];
}}

function buildAxisTitleGraphics(layout, visibleAxisIds) {{
  return titledAxisCells.flatMap(function(axisCell) {{
    if (!visibleAxisIds.has(axisCell.id)) {{ return []; }}
    return buildAxisTitleGraphic(layout.cellsById[axisCell.id]);
  }});
}}

function buildGridDebugGraphic(layout) {{
  const debugItems = [
    {{ id: "plot-grid", rect: layout.plotRect, stroke: "#ef4444" }}
  ].concat(Object.values(layout.cellsById).map(function(cell, index) {{
    return {{ id: cell.id, rect: cell.renderRect, stroke: ["#2563eb", "#10b981", "#f59e0b", "#8b5cf6"][index % 4] }};
  }}));
  return debugItems.map(function(item) {{
    return {{
      type: "rect", id: "grid-debug-" + item.id, silent: true, z: 1000,
      shape: {{ x: item.rect.x, y: item.rect.y, width: item.rect.width, height: item.rect.height }},
      style: {{ fill: "rgba(0, 0, 0, 0)", stroke: item.stroke, lineWidth: 1, lineDash: [6, 4] }},
    }};
  }});
}}

function getVisibleSeries() {{
  return seriesData.filter(function(s) {{ return seriesVisibility[s.name]; }});
}}

function getAxisIdsForSeries(seriesItem) {{
  return new Set([seriesItem.xAxisKey, seriesItem.yAxisKey]);
}}

function clearHoverResetTimer() {{
  if (hoverResetTimer !== null) {{ window.clearTimeout(hoverResetTimer); hoverResetTimer = null; }}
}}

function getVisibleAxisIds(visibleSeries, activeSeriesName) {{
  const activeSeries = activeSeriesName ? visibleSeries.find(function(s) {{ return s.name === activeSeriesName; }}) : null;
  const activeAxisIds = activeSeries ? getAxisIdsForSeries(activeSeries) : null;
  return new Set(
    layoutSpec.cells.filter(function(cell) {{
      if (cell.kind !== "axis") {{ return false; }}
      if (cell.visibilityPolicy === "always" || !cell.visibilityPolicy) {{ return true; }}
      if (cell.visibilityPolicy === "if-any-bound-series-visible") {{
        if (activeAxisIds) {{ return activeAxisIds.has(cell.id); }}
        return visibleSeries.some(function(s) {{ return s.xAxisKey === cell.id || s.yAxisKey === cell.id; }});
      }}
      return true;
    }}).map(function(cell) {{ return cell.id; }}),
  );
}}

function buildOption(width, height, activeSeriesName) {{
  activeSeriesName = activeSeriesName || null;
  const visibleSeries = getVisibleSeries();
  const visibleAxisIds = getVisibleAxisIds(visibleSeries, activeSeriesName);
  const resolvedSpec = resolveMeasuredSpec(width, height, layoutSpec);
  const layout = computeCrossLayout(width, height, resolvedSpec);
  const titleCell = layout.cellsById.title;
  const legendCell = layout.cellsById.legend;
  const scaleCell = layout.cellsById["x-scale"];
  const legendLayout = legendCell ? measureLegendLayout(legendCell.renderRect.width) : {{ width: 0, height: 0 }};
  const axisCells = resolvedSpec.cells.filter(function(cell) {{ return cell.kind === "axis"; }});
  const xAxes = [];
  const yAxes = [];
  const xAxisIndexById = {{}};
  const yAxisIndexById = {{}};
  axisCells.forEach(function(cell) {{
    const isActive = visibleAxisIds.has(cell.id);
    const positionedCell = layout.cellsById[cell.id];
    const axisOption = buildAxisOption(positionedCell, layout.trackOffsets[cell.id] || 0, isActive);
    if (cell.axisDimension === "x") {{
      xAxisIndexById[cell.id] = xAxes.length;
      xAxes.push(axisOption);
    }} else {{
      yAxisIndexById[cell.id] = yAxes.length;
      yAxes.push(axisOption);
    }}
  }});
  const option = {{
    animation: false,
    tooltip: {{ trigger: "axis", axisPointer: {{ type: "shadow" }} }},
    grid: {{
      top: toPercent(layout.plotRect.y, height),
      right: toPercent(width - layout.plotRect.x - layout.plotRect.width, width),
      bottom: toPercent(height - layout.plotRect.y - layout.plotRect.height, height),
      left: toPercent(layout.plotRect.x, width),
    }},
    dataZoom: scaleCell ? [
      {{
        id: "x-scale-slider", type: "slider",
        xAxisIndex: Array.from(new Set(Object.values(xAxisIndexById))),
        filterMode: "filter", showDetail: false, realtime: true, brushSelect: false, moveHandleSize: 0,
        height: scaleCell.renderRect.height, left: scaleCell.renderRect.x, top: scaleCell.renderRect.y, width: scaleCell.renderRect.width,
        borderColor: "#d7deeb", fillerColor: "rgba(59, 130, 246, 0.14)", backgroundColor: "rgba(230, 235, 245, 0.65)",
        dataBackground: {{ lineStyle: {{ color: "#94a3b8", opacity: 0.9 }}, areaStyle: {{ color: "rgba(203, 213, 225, 0.6)" }} }},
        selectedDataBackground: {{ lineStyle: {{ color: "#3b82f6" }}, areaStyle: {{ color: "rgba(59, 130, 246, 0.2)" }} }},
        handleStyle: {{ color: "#ffffff", borderColor: "#94a3b8", shadowBlur: 0 }},
        textStyle: {{ color: "#5f6b85" }},
      }}
    ] : [],
    graphic: buildAxisTitleGraphics(layout, visibleAxisIds).concat(gridDebug ? buildGridDebugGraphic(layout) : []),
    xAxis: xAxes,
    yAxis: yAxes,
    series: seriesData.map(function(seriesItem) {{
      const isHovered = activeSeriesName === seriesItem.name;
      const isDimmed = activeSeriesName && !isHovered;
      const opacity = isDimmed ? 0.14 : 1;
      return {{
        name: seriesItem.name, type: seriesItem.type, data: seriesItem.values,
        xAxisIndex: xAxisIndexById[seriesItem.xAxisKey], yAxisIndex: yAxisIndexById[seriesItem.yAxisKey],
        clip: true, smooth: Boolean(seriesItem.smooth),
        symbol: seriesItem.symbol || (seriesItem.showSymbol === false ? "none" : "circle"),
        symbolSize: seriesItem.symbolSize || 7,
        triggerLineEvent: seriesItem.type === "line",
        barMaxWidth: seriesItem.barMaxWidth,
        z: isHovered ? 4 : 2,
        lineStyle: seriesItem.type === "line" ? {{ width: seriesItem.lineWidth || 3, color: seriesItem.color, opacity: opacity }} : undefined,
        areaStyle: seriesItem.areaStyle ? Object.assign({{}}, seriesItem.areaStyle, {{ opacity: opacity }}) : undefined,
        itemStyle: {{ color: seriesItem.color, borderColor: seriesItem.color, opacity: opacity }},
        emphasis: {{ disabled: true }},
      }};
    }}),
  }};
  if (titleCell) {{
    option.title = {{
      text: titleCell.text, left: titleCell.renderRect.x + titleCell.renderRect.width / 2, top: titleCell.renderRect.y,
      z: titleCell.renderRect.zIndex, textAlign: "center",
      textStyle: {{ color: "#172033", fontSize: 20, fontWeight: 600 }},
    }};
  }}
  if (legendCell) {{
    option.legend = {{
      data: seriesData.map(function(s) {{ return s.name; }}), selected: seriesVisibility,
      top: legendCell.renderRect.y + Math.max(0, (legendCell.renderRect.height - legendLayout.height) / 2),
      left: legendCell.renderRect.x + Math.max(0, (legendCell.renderRect.width - legendLayout.width) / 2),
      width: legendLayout.width, z: legendCell.renderRect.zIndex,
      itemWidth: layoutTheme.legendItemWidth, itemHeight: layoutTheme.legendItemHeight, itemGap: 12,
      textStyle: {{ color: "#5f6b85" }},
    }};
  }}
  return option;
}}

function renderChart() {{
  const width = chartNode.clientWidth;
  const height = chartNode.clientHeight;
  chart.setOption(buildOption(width, height, hoveredSeriesName), true);
}}

function setHoveredSeries(seriesName) {{
  clearHoverResetTimer();
  if (!seriesName || !seriesVisibility[seriesName]) {{
    if (!hoveredSeriesName) {{ return; }}
    hoveredSeriesName = null;
    renderChart();
    return;
  }}
  if (hoveredSeriesName === seriesName) {{ return; }}
  hoveredSeriesName = seriesName;
  renderChart();
}}

function scheduleHoveredSeriesReset() {{
  clearHoverResetTimer();
  hoverResetTimer = window.setTimeout(function() {{
    hoverResetTimer = null;
    setHoveredSeries(null);
  }}, 0);
}}

renderChart();

chart.on("legendselectchanged", function(event) {{
  Object.entries(event.selected).forEach(function(entry) {{
    seriesVisibility[entry[0]] = entry[1];
  }});
  if (hoveredSeriesName && !seriesVisibility[hoveredSeriesName]) {{ hoveredSeriesName = null; }}
  renderChart();
}});

chart.on("mouseover", {{ componentType: "series" }}, function(event) {{
  setHoveredSeries(event.seriesName || null);
}});

chart.on("mouseout", {{ componentType: "series" }}, function() {{
  scheduleHoveredSeriesReset();
}});

chart.on("globalout", function() {{
  scheduleHoveredSeriesReset();
}});

chart.getZr().on("mousemove", function(event) {{
  if (event.target || !hoveredSeriesName) {{ return; }}
  setHoveredSeries(null);
}});

window.addEventListener("resize", function() {{
  chart.resize();
  renderChart();
}});

console.info("msp echarts points:", {{ original: {total_original_points}, embedded: {total_embedded_points}, maxPerSeries: {max_points} }});
</script>
</body>
</html>"##,
        labels_js = labels_js,
        series_data_js = series_data_js,
        cells_js = cells_js,
        axis_ranges_js = axis_ranges_js,
        theme = theme,
        font_family = font_family,
        font_size = font_size,
        scale_mode_overrides = scale_mode_overrides_js,
        total_original_points = total_original_points,
        total_embedded_points = total_embedded_points,
        max_points = max_points,
    ))
}

impl Backend for EchartsBackend {
    fn build_render_plan(
        &self,
        request: &ResolvedMspRequest,
        prepared: &[PreparedSeries],
    ) -> anyhow::Result<RenderPlan> {
        let options = match &request.backend_options {
            BackendOptions::Echarts(options) => options,
            _ => bail!("Expected echarts backend options"),
        };
        let html = build_html(request, prepared, options)?;
        let out_path = request.render_target.out.clone().unwrap_or_else(|| {
            request
                .render_target
                .work_dir
                .join("msp-echarts.html")
        });
        Ok(RenderPlan {
            description: format!("ECharts HTML -> {}", out_path.display()),
            payload: html,
            artifact_path: Some(out_path),
        })
    }

    fn execute(&self, plan: &RenderPlan, request: &ResolvedMspRequest) -> anyhow::Result<()> {
        write_artifact(plan)?;
        if request.render_target.open {
            if let Some(path) = &plan.artifact_path {
                maybe_open_in_browser(path)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::{
        backend::Backend,
        spec::{
            AxisBinding, AxisScale, AxisSpec, BackendKind, BackendOptions,
            DataPrepSpec, EchartsBackendOptions, ExecutionMode, LayoutSpec,
            LegendSpec, PlotAxes, PlotSpec, PreparedSeries, RenderTarget,
            ResolvedMspRequest, SeriesMark, SeriesSpec, SeriesStyle, ThemeSpec,
            TickSpec,
        },
    };

    use super::EchartsBackend;

    fn unique_test_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "spreadsheet-plotter-echarts-test-{}-{}",
            std::process::id(),
            nanos
        ))
    }

    fn write_csv(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn test_request(work_dir: &Path, out: &Path) -> ResolvedMspRequest {
        ResolvedMspRequest {
            mode: ExecutionMode::Plot,
            backend: BackendKind::Echarts,
            data_prep: DataPrepSpec {
                inputs: Vec::new(),
                series: Vec::new(),
            },
            plot: PlotSpec {
                layout: LayoutSpec {
                    width: 1.0,
                    height: 1.0,
                },
                theme: ThemeSpec { font: None },
                legend: LegendSpec {
                    position: "top right".to_string(),
                    font: None,
                },
                axes: PlotAxes {
                    x: AxisSpec {
                        scale: AxisScale::Linear,
                        range: None,
                        label: Some("Sample X".to_string()),
                        ticks: TickSpec {
                            major: None,
                            custom: Vec::new(),
                        },
                    },
                    y: AxisSpec {
                        scale: AxisScale::Linear,
                        range: Some(0.0..100.0),
                        label: Some("Primary Y".to_string()),
                        ticks: TickSpec {
                            major: None,
                            custom: Vec::new(),
                        },
                    },
                    x2: AxisSpec::default(),
                    y2: AxisSpec {
                        scale: AxisScale::Log10,
                        range: Some(1.0..1000.0),
                        label: Some("Secondary Y".to_string()),
                        ticks: TickSpec {
                            major: None,
                            custom: Vec::new(),
                        },
                    },
                },
                grid: true,
            },
            render_target: RenderTarget {
                work_dir: work_dir.to_path_buf(),
                out: Some(out.to_path_buf()),
                format_hint: Some("html".to_string()),
                open: false,
            },
            backend_options: BackendOptions::Echarts(EchartsBackendOptions {
                theme: Some("light".to_string()),
                max_points: None,
            }),
        }
    }

    #[test]
    fn build_render_plan_embeds_generated_csv_data() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(
            &csv_path,
            "x,y\n1,10\n2,20\n3,30\n",
        );

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: AxisBinding::X1Y1,
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "y".to_string(),
                mark: SeriesMark::Lines,
                name: Some("Throughput".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path.clone(),
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let request = test_request(&work_dir, &out_path);

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert_eq!(plan.artifact_path.as_deref(), Some(out_path.as_path()));
        assert!(plan.payload.contains("echarts.min.js"));
        assert!(plan.payload.contains("Throughput"));
        assert!(plan.payload.contains("[1,10]"));
        assert!(plan.payload.contains("[2,20]"));
        assert!(plan.payload.contains("Primary Y"));
        assert!(plan.payload.contains("xAxisKey:\"x-bottom\""));
        assert!(plan.payload.contains("yAxisKey:\"y-left\""));
        assert!(plan.payload.contains("labelMargin:10,visibilityPolicy"));
        assert!(plan.payload.contains("nameLocation: \"middle\""));
        assert!(plan.payload.contains("function computeCrossLayout"));
        assert!(plan.payload.contains("function buildAxisTitleGraphic"));
        assert!(plan.payload.contains("function getAxisTitleLayout"));
        assert!(plan.payload.contains("function getYAxisTitleGap"));
        assert!(plan.payload.contains("function buildTrackMap"));
        assert!(plan.payload.contains("function resolveMeasuredSpec"));
        assert!(plan.payload.contains("Math.ceil(getFontSize(layoutTheme.axisLabelFont) * 1.5)"));
        assert!(plan.payload.contains("labelBlock + titleBlock + layoutTheme.axisPadding"));
        assert!(plan.payload.contains("anchorX = cell.side === \"left\""));
        assert!(plan.payload.contains("layoutSpec"));
        assert!(plan.payload.contains("seriesData"));
        assert!(plan.payload.contains("\"x-bottom\""));
        assert!(plan.payload.contains("\"y-left\""));
    }

    #[test]
    fn build_render_plan_supports_category_x_and_secondary_axis() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(
            &csv_path,
            "label,value\nalpha,5\nbeta,8\n",
        );

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: AxisBinding::X1Y2,
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "label".to_string(),
                y_expr: "value".to_string(),
                mark: SeriesMark::Points,
                name: Some("Latency".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path.clone(),
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let request = test_request(&work_dir, &out_path);

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains("\"category\""));
        assert!(plan.payload.contains("yAxisKey:\"y-right-1\""));
        assert!(plan.payload.contains("[\"alpha\",5]"));
        assert!(plan.payload.contains("type:\"scatter\""));
        assert!(plan.payload.contains("Secondary Y"));
    }

    #[test]
    fn build_render_plan_downsamples_large_series_when_max_points_is_set() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        let mut csv = String::from("x,y\n");
        for i in 0..20 {
            csv.push_str(&format!("{i},{}\n", i * 10));
        }
        write_csv(&csv_path, &csv);

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: AxisBinding::X1Y1,
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "y".to_string(),
                mark: SeriesMark::Lines,
                name: Some("Downsampled".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path,
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.backend_options = BackendOptions::Echarts(EchartsBackendOptions {
            theme: Some("light".to_string()),
            max_points: Some(5),
        });

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains("original: 20"));
        assert!(plan.payload.contains("embedded: 5"));
        assert!(plan.payload.contains("[0,0]"));
        assert!(plan.payload.contains("[19,190]"));
    }
}
