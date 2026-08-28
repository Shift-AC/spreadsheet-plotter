use std::fs;

use anyhow::{Context, bail};

use crate::{
    backend::{Backend, RenderPlan, maybe_open_in_browser, write_artifact},
    spec::{
        AxisDimension, AxisRef, AxisScale, AxisValueFormat, BackendOptions,
        EchartsBackendOptions, EchartsOutputMode, EchartsRuntimeMode,
        PreparedSeries, ResolvedMspRequest, SeriesAxisBinding, SeriesMark,
        TimestampUnit,
    },
};

pub struct EchartsBackend;

const DEFAULT_MAX_POINTS_PER_SERIES: usize = 200;
const DEFAULT_CHART_WIDTH_PX: f32 = 800.0;
const DEFAULT_CHART_HEIGHT_PX: f32 = 800.0;
const ECHARTS_CDN_URL: &str =
    "https://cdn.jsdelivr.net/npm/echarts@6/dist/echarts.min.js";

#[derive(Debug)]
struct CsvSeriesData {
    x_label: String,
    y_label: String,
    x_numeric: bool,
    points: Vec<(String, f64)>,
    original_point_count: usize,
}

#[derive(Debug)]
struct BoxplotSeriesData {
    x_label: String,
    y_label: String,
    labels: Vec<String>,
    groups: Vec<Vec<f64>>,
    medians: Vec<(String, f64)>,
    original_point_count: usize,
}

#[derive(Debug)]
enum ParsedSeriesData {
    Standard(CsvSeriesData),
    Boxplot(BoxplotSeriesData),
}

#[derive(Debug)]
struct RenderSeries {
    name: String,
    axis_binding: SeriesAxisBinding,
    mark: SeriesMark,
    data: ParsedSeriesData,
}

#[derive(Debug, Clone)]
struct AxisMeta {
    axis_ref: AxisRef,
    key: String,
    side: &'static str,
    track: usize,
    name: String,
    number_format: String,
}

#[derive(Debug, Clone, Copy)]
struct LegendPlacement {
    side: &'static str,
    align: &'static str,
    minor_span: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum XAxisRenderModel {
    Numeric,
    Category(Vec<String>),
}

fn escape_js_string(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('"', "\\\"")
}

fn axis_value_format_js_value(format: &AxisValueFormat) -> String {
    match format {
        AxisValueFormat::Plain { decimals } => {
            format!(
                "{{mode:\"plain\",decimals:{}}}",
                format_optional_js_usize(*decimals)
            )
        }
        AxisValueFormat::Suffix { decimals } => {
            format!(
                "{{mode:\"suffix\",decimals:{}}}",
                format_optional_js_usize(*decimals)
            )
        }
        AxisValueFormat::Scientific { decimals } => {
            format!(
                "{{mode:\"scientific\",decimals:{}}}",
                format_optional_js_usize(*decimals)
            )
        }
        AxisValueFormat::Percentage { decimals } => {
            format!(
                "{{mode:\"percentage\",decimals:{}}}",
                format_optional_js_usize(*decimals)
            )
        }
        AxisValueFormat::Timestamp { unit, timezone } => {
            let unit = match unit {
                TimestampUnit::Seconds => "s",
                TimestampUnit::Milliseconds => "ms",
            };
            let timezone = timezone
                .as_ref()
                .map(|value| format!("\"{}\"", escape_js_string(value)))
                .unwrap_or_else(|| "undefined".to_string());
            format!(
                "{{mode:\"timestamp\",unit:\"{unit}\",timezone:{timezone}}}"
            )
        }
    }
}

fn format_optional_js_usize(value: Option<usize>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "undefined".to_string())
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
    let content = fs::read_to_string(path).with_context(|| {
        format!("Failed to read prepared data '{}'", path.display())
    })?;
    let mut lines = content.lines().filter(|line| !line.trim().is_empty());
    let header = lines.next().with_context(|| {
        format!("Prepared data '{}' is empty", path.display())
    })?;
    let header = parse_csv_row(header);
    if header.len() < 2 {
        bail!(
            "Prepared data '{}' does not contain two columns",
            path.display()
        );
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
            format!(
                "Failed to parse y value '{}' from '{}'",
                row[1],
                path.display()
            )
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

fn median(values: &[f64]) -> f64 {
    let len = values.len();
    if len == 0 {
        return 0.0;
    }
    let mid = len / 2;
    if len % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

fn parse_boxplot_series_csv(
    path: &std::path::Path,
) -> anyhow::Result<BoxplotSeriesData> {
    let content = fs::read_to_string(path).with_context(|| {
        format!("Failed to read prepared data '{}'", path.display())
    })?;
    let mut lines = content.lines().filter(|line| !line.trim().is_empty());
    let header = lines.next().with_context(|| {
        format!("Prepared data '{}' is empty", path.display())
    })?;
    let header = parse_csv_row(header);
    if header.len() < 2 {
        bail!(
            "Prepared data '{}' does not contain two columns",
            path.display()
        );
    }

    let mut labels = Vec::new();
    let mut label_indexes = std::collections::HashMap::new();
    let mut groups: Vec<Vec<f64>> = Vec::new();
    let mut original_point_count = 0usize;

    for line in lines {
        let row = parse_csv_row(line);
        if row.len() < 2 {
            continue;
        }
        let label = row[0].clone();
        let value = row[1].parse::<f64>().with_context(|| {
            format!(
                "Failed to parse y value '{}' from '{}'",
                row[1],
                path.display()
            )
        })?;
        let group_index = if let Some(index) = label_indexes.get(&label) {
            *index
        } else {
            let index = labels.len();
            labels.push(label.clone());
            label_indexes.insert(label, index);
            groups.push(Vec::new());
            index
        };
        groups[group_index].push(value);
        original_point_count += 1;
    }

    let medians = labels
        .iter()
        .zip(groups.iter())
        .map(|(label, group)| {
            let mut sorted = group.clone();
            sorted.sort_by(|left, right| left.total_cmp(right));
            (label.clone(), median(&sorted))
        })
        .collect();

    Ok(BoxplotSeriesData {
        x_label: header[0].clone(),
        y_label: header[1].clone(),
        labels,
        groups,
        medians,
        original_point_count,
    })
}

fn merge_boxplot_series_data(
    base: &mut BoxplotSeriesData,
    incoming: BoxplotSeriesData,
) {
    let mut label_indexes = base
        .labels
        .iter()
        .enumerate()
        .map(|(index, label)| (label.clone(), index))
        .collect::<std::collections::HashMap<_, _>>();

    for (label, group) in incoming.labels.into_iter().zip(incoming.groups) {
        let group_index = if let Some(index) = label_indexes.get(&label) {
            *index
        } else {
            let index = base.labels.len();
            base.labels.push(label.clone());
            base.groups.push(Vec::new());
            label_indexes.insert(label, index);
            index
        };
        base.groups[group_index].extend(group);
    }

    base.original_point_count += incoming.original_point_count;
    refresh_boxplot_medians(base);
}

fn refresh_boxplot_medians(series: &mut BoxplotSeriesData) {
    series.medians = series
        .labels
        .iter()
        .zip(series.groups.iter())
        .map(|(label, group)| {
            let mut sorted = group.clone();
            sorted.sort_by(|left, right| left.total_cmp(right));
            (label.clone(), median(&sorted))
        })
        .collect();
}

fn downsample_boxplot_series(
    series: &mut BoxplotSeriesData,
    max_points: usize,
) {
    let total_points = series.groups.iter().map(Vec::len).sum::<usize>();
    if max_points == 0 || total_points <= max_points {
        return;
    }

    let mut flattened = Vec::with_capacity(total_points);
    for (group_index, group) in series.groups.iter().enumerate() {
        flattened.extend(group.iter().map(|value| (group_index, *value)));
    }
    let sampled = downsample_points(&flattened, max_points);

    let old_labels = std::mem::take(&mut series.labels);
    let group_count = series.groups.len();
    series.groups = vec![Vec::new(); group_count];
    for (group_index, value) in sampled {
        if let Some(group) = series.groups.get_mut(group_index) {
            group.push(value);
        }
    }

    let mut labels = Vec::new();
    let mut groups = Vec::new();
    for (label, group) in old_labels.into_iter().zip(series.groups.drain(..)) {
        if !group.is_empty() {
            labels.push(label);
            groups.push(group);
        }
    }
    series.labels = labels;
    series.groups = groups;
    refresh_boxplot_medians(series);
}

fn build_render_series(
    prepared: &[PreparedSeries],
    max_points: usize,
) -> anyhow::Result<Vec<RenderSeries>> {
    let mut rendered = Vec::<RenderSeries>::new();
    let mut boxplot_group_indexes =
        std::collections::HashMap::<usize, usize>::new();

    for (index, series) in prepared.iter().enumerate() {
        let default_name = format!("series {}", index + 1);
        let name = series
            .spec
            .name
            .clone()
            .unwrap_or_else(|| default_name.clone());
        match series.spec.mark {
            SeriesMark::Boxplot => {
                let parsed = parse_boxplot_series_csv(&series.output_path)?;
                if let Some(group) = series.spec.boxplot_group {
                    if let Some(render_index) =
                        boxplot_group_indexes.get(&group)
                    {
                        let existing = rendered
                            .get_mut(*render_index)
                            .expect("stored boxplot render index must exist");
                        match &mut existing.data {
                            ParsedSeriesData::Boxplot(existing_boxplot) => {
                                merge_boxplot_series_data(
                                    existing_boxplot,
                                    parsed,
                                );
                            }
                            ParsedSeriesData::Standard(_) => {
                                unreachable!(
                                    "boxplot group points to non-boxplot render series"
                                )
                            }
                        }
                    } else {
                        let render_index = rendered.len();
                        boxplot_group_indexes.insert(group, render_index);
                        rendered.push(RenderSeries {
                            name: if series.spec.name.is_some() {
                                name
                            } else {
                                format!("boxplot {}", group)
                            },
                            axis_binding: series.spec.axis_binding,
                            mark: SeriesMark::Boxplot,
                            data: ParsedSeriesData::Boxplot(parsed),
                        });
                    }
                } else {
                    rendered.push(RenderSeries {
                        name,
                        axis_binding: series.spec.axis_binding,
                        mark: SeriesMark::Boxplot,
                        data: ParsedSeriesData::Boxplot(parsed),
                    });
                }
            }
            _ => {
                let mut parsed = parse_series_csv(&series.output_path)?;
                parsed.points = downsample_points(&parsed.points, max_points);
                rendered.push(RenderSeries {
                    name,
                    axis_binding: series.spec.axis_binding,
                    mark: series.spec.mark,
                    data: ParsedSeriesData::Standard(parsed),
                });
            }
        }
    }

    for series in &mut rendered {
        if let ParsedSeriesData::Boxplot(parsed) = &mut series.data {
            downsample_boxplot_series(parsed, max_points);
        }
    }

    Ok(rendered)
}

fn downsample_points<T: Clone + PartialEq>(
    points: &[T],
    max_points: usize,
) -> Vec<T> {
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
        if sampled.last().is_none_or(|last: &T| last != &points[index]) {
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

fn x_axis_key(index: usize) -> &'static str {
    match index {
        1 => "x-bottom",
        2 => "x-top",
        _ => unreachable!("only x1 and x2 are supported"),
    }
}

fn y_axis_key(index: usize) -> String {
    if index == 1 {
        "y-left-1".to_string()
    } else {
        format!("y-right-{}", index - 1)
    }
}

fn format_js_number(value: f64) -> String {
    value.to_string()
}

fn format_optional_js_number(value: Option<f64>) -> String {
    value
        .map(format_js_number)
        .unwrap_or_else(|| "undefined".to_string())
}

// ECharts computes box geometry natively, but its transform emits outliers as a
// separate dataset result. We always keep the native boxplot and the aligned
// outlier overlay together as one emitted structure so later changes do not
// accidentally regress grouped-category boxplots back to centered outliers.
fn append_boxplot_series_data_parts(
    parts: &mut Vec<String>,
    dataset_entries_js: &mut Vec<String>,
    parsed: &BoxplotSeriesData,
    x_axis_key: &str,
) {
    let grouped_values_js = parsed
        .groups
        .iter()
        .map(|group| {
            format!(
                "[{}]",
                group
                    .iter()
                    .map(|value| format_js_number(*value))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let raw_dataset_index = dataset_entries_js.len();
    dataset_entries_js.push(format!("{{source:[{grouped_values_js}]}}"));
    let transform_dataset_index = dataset_entries_js.len();
    dataset_entries_js.push(format!(
        "{{fromDatasetIndex:{raw_dataset_index},transform:{{type:\"boxplot\",config:{{itemNameFormatter:function(params){{var axisLabels = axisCategoryLabels[\"{x_axis_key}\"] || []; return axisLabels[params.value] == null ? String(params.value) : axisLabels[params.value];}}}}}}}}"
    ));
    let outlier_dataset_index = dataset_entries_js.len();
    dataset_entries_js.push(format!(
        "{{fromDatasetIndex:{transform_dataset_index},fromTransformResult:1}}"
    ));
    parts.push("type:\"boxplot\"".to_string());
    parts.push(format!(
        "values:[{}]",
        parsed
            .medians
            .iter()
            .map(|(label, value)| {
                format!(
                    "[\"{}\",{}]",
                    escape_js_string(label),
                    format_js_number(*value)
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    ));
    parts.push(format!("groupedValues:[{grouped_values_js}]"));
    parts.push(format!("rawDatasetIndex:{raw_dataset_index}"));
    parts.push(format!("datasetIndex:{transform_dataset_index}"));
    parts.push(format!("outlierDatasetIndex:{outlier_dataset_index}"));
}

fn parse_legend_placement(position: &str) -> anyhow::Result<LegendPlacement> {
    match position.trim().to_ascii_lowercase().as_str() {
        "top right" => Ok(LegendPlacement {
            side: "top",
            align: "end",
            minor_span: "50%",
        }),
        "top left" => Ok(LegendPlacement {
            side: "top",
            align: "start",
            minor_span: "50%",
        }),
        "top center" => Ok(LegendPlacement {
            side: "top",
            align: "center",
            minor_span: "60%",
        }),
        "bottom right" => Ok(LegendPlacement {
            side: "bottom",
            align: "end",
            minor_span: "50%",
        }),
        "bottom left" => Ok(LegendPlacement {
            side: "bottom",
            align: "start",
            minor_span: "50%",
        }),
        "bottom center" => Ok(LegendPlacement {
            side: "bottom",
            align: "center",
            minor_span: "60%",
        }),
        "right" => Ok(LegendPlacement {
            side: "right",
            align: "center",
            minor_span: "60%",
        }),
        "left" => Ok(LegendPlacement {
            side: "left",
            align: "center",
            minor_span: "60%",
        }),
        _ => bail!("Unknown echarts legend position '{position}'"),
    }
}

fn categorical_labels_for_series(series: &RenderSeries) -> Option<Vec<String>> {
    match &series.data {
        ParsedSeriesData::Standard(parsed)
            if !parsed.x_numeric || matches!(series.mark, SeriesMark::Bar) =>
        {
            let mut seen = std::collections::HashSet::new();
            Some(
                parsed
                    .points
                    .iter()
                    .filter_map(|(label, _)| {
                        seen.insert(label.clone()).then_some(label.clone())
                    })
                    .collect(),
            )
        }
        ParsedSeriesData::Boxplot(parsed) => Some(parsed.labels.clone()),
        _ => None,
    }
}

fn validate_x_axis_models(
    rendered: &[RenderSeries],
) -> anyhow::Result<std::collections::BTreeMap<AxisRef, XAxisRenderModel>> {
    let mut models = std::collections::BTreeMap::new();

    for axis_index in 1..=2 {
        let axis_ref = AxisRef::x(axis_index);
        let axis_series = rendered
            .iter()
            .filter(|series| series.axis_binding.x_axis() == axis_ref)
            .collect::<Vec<_>>();
        if axis_series.is_empty() {
            continue;
        }

        let category_series = axis_series
            .iter()
            .filter_map(|series| {
                categorical_labels_for_series(series).map(|labels| {
                    (
                        series.name.as_str(),
                        matches!(series.data, ParsedSeriesData::Boxplot(_)),
                        labels,
                    )
                })
            })
            .collect::<Vec<_>>();

        let numeric_series = axis_series
            .iter()
            .filter_map(|series| match &series.data {
                ParsedSeriesData::Standard(parsed) if parsed.x_numeric => {
                    Some(series.name.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let supports_category_overlay = axis_series.iter().any(|series| {
            matches!(series.mark, SeriesMark::Bar)
                || matches!(series.data, ParsedSeriesData::Boxplot(_))
        });

        if !supports_category_overlay
            && !numeric_series.is_empty()
            && !category_series.is_empty()
        {
            bail!(
                "x axis {} mixes numeric and category series: numeric [{}], category [{}]",
                axis_ref,
                numeric_series.join(", "),
                category_series
                    .iter()
                    .map(|(name, _, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }

        if !category_series.is_empty() {
            let category_axis_series = if supports_category_overlay {
                axis_series
                    .iter()
                    .map(|series| {
                        let labels = match &series.data {
                            ParsedSeriesData::Standard(parsed) => {
                                let mut seen = std::collections::HashSet::new();
                                parsed
                                    .points
                                    .iter()
                                    .filter_map(|(label, _)| {
                                        seen.insert(label.clone())
                                            .then_some(label.clone())
                                    })
                                    .collect::<Vec<_>>()
                            }
                            ParsedSeriesData::Boxplot(parsed) => {
                                parsed.labels.clone()
                            }
                        };
                        (
                            series.name.as_str(),
                            matches!(series.data, ParsedSeriesData::Boxplot(_)),
                            labels,
                        )
                    })
                    .collect::<Vec<_>>()
            } else {
                category_series.clone()
            };
            let canonical_labels = category_series
                .iter()
                .find(|(_, is_boxplot, _)| *is_boxplot)
                .or_else(|| category_series.first())
                .map(|(_, _, labels)| labels.clone())
                .expect("category series must provide labels");
            let canonical_set = canonical_labels
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();

            for (name, _, labels) in &category_axis_series {
                let label_set = labels
                    .iter()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>();
                if label_set != canonical_set {
                    bail!(
                        "x axis {} has incompatible category sets between series '{}' and the shared axis categories: expected [{}], got [{}]",
                        axis_ref,
                        name,
                        canonical_labels.join(", "),
                        labels.join(", "),
                    );
                }
            }

            models
                .insert(axis_ref, XAxisRenderModel::Category(canonical_labels));
        } else {
            models.insert(axis_ref, XAxisRenderModel::Numeric);
        }
    }

    Ok(models)
}

fn build_x_axes(
    request: &ResolvedMspRequest,
    prepared: &[PreparedSeries],
    fallback_label: &str,
) -> Vec<AxisMeta> {
    let primary_label = request
        .plot
        .axes
        .axis(AxisRef::x(1))
        .and_then(|axis| axis.label.clone())
        .unwrap_or_else(|| fallback_label.to_string());
    let secondary_label = request
        .plot
        .axes
        .axis(AxisRef::x(2))
        .and_then(|axis| axis.label.clone())
        .unwrap_or_else(|| primary_label.clone());

    let mut axes = Vec::new();
    if prepared.iter().any(|s| s.spec.axis_binding.x_index == 1) {
        axes.push(AxisMeta {
            axis_ref: AxisRef::x(1),
            key: x_axis_key(1).to_string(),
            side: "bottom",
            track: 0,
            name: escape_js_string(&primary_label),
            number_format: axis_value_format_js_value(
                &request.plot.axes.get(AxisRef::x(1)).number_format,
            ),
        });
    }
    if prepared.iter().any(|s| s.spec.axis_binding.x_index == 2) {
        axes.push(AxisMeta {
            axis_ref: AxisRef::x(2),
            key: x_axis_key(2).to_string(),
            side: "top",
            track: 0,
            name: escape_js_string(&secondary_label),
            number_format: axis_value_format_js_value(
                &request.plot.axes.get(AxisRef::x(2)).number_format,
            ),
        });
    }
    axes
}

fn build_y_axes(
    request: &ResolvedMspRequest,
    prepared: &[PreparedSeries],
    fallback_label: &str,
) -> Vec<AxisMeta> {
    let primary_label = request
        .plot
        .axes
        .axis(AxisRef::y(1))
        .and_then(|axis| axis.label.clone())
        .unwrap_or_else(|| fallback_label.to_string());
    let mut indexes = prepared
        .iter()
        .map(|series| series.spec.axis_binding.y_index)
        .collect::<std::collections::BTreeSet<_>>();
    indexes.extend(request.plot.axes.iter().filter_map(|(axis, _)| {
        (matches!(axis.dimension, AxisDimension::Y) && axis.index > 2)
            .then_some(axis.index)
    }));
    indexes
        .into_iter()
        .map(|index| {
            let label = request
                .plot
                .axes
                .axis(AxisRef::y(index))
                .and_then(|axis| axis.label.clone())
                .unwrap_or_else(|| primary_label.clone());
            AxisMeta {
                axis_ref: AxisRef::y(index),
                key: y_axis_key(index),
                side: if index == 1 { "left" } else { "right" },
                track: if index == 1 { 0 } else { index - 2 },
                name: escape_js_string(&label),
                number_format: axis_value_format_js_value(
                    &request.plot.axes.get(AxisRef::y(index)).number_format,
                ),
            }
        })
        .collect()
}

fn build_chart_snippet(
    request: &ResolvedMspRequest,
    prepared: &[PreparedSeries],
    options: &EchartsBackendOptions,
) -> anyhow::Result<String> {
    let max_points =
        options.max_points.unwrap_or(DEFAULT_MAX_POINTS_PER_SERIES);
    let rendered = build_render_series(prepared, max_points)?;
    let x_axis_models = validate_x_axis_models(&rendered)?;

    let x_label = rendered
        .first()
        .map(|series| match series {
            RenderSeries {
                data: ParsedSeriesData::Standard(series),
                ..
            } => series.x_label.clone(),
            RenderSeries {
                data: ParsedSeriesData::Boxplot(series),
                ..
            } => series.x_label.clone(),
        })
        .unwrap_or_else(|| "x".to_string());
    let y_label = rendered
        .first()
        .map(|series| match series {
            RenderSeries {
                data: ParsedSeriesData::Standard(series),
                ..
            } => series.y_label.clone(),
            RenderSeries {
                data: ParsedSeriesData::Boxplot(series),
                ..
            } => series.y_label.clone(),
        })
        .unwrap_or_else(|| "y".to_string());
    let labels_by_axis = x_axis_models
        .iter()
        .filter_map(|(axis_ref, model)| match model {
            XAxisRenderModel::Category(labels) => {
                Some((x_axis_key(axis_ref.index).to_string(), labels.clone()))
            }
            XAxisRenderModel::Numeric => None,
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let primary_labels = labels_by_axis
        .get(x_axis_key(1))
        .cloned()
        .or_else(|| labels_by_axis.values().next().cloned())
        .unwrap_or_default();
    let labels_js = primary_labels
        .iter()
        .map(|label| format!("\"{}\"", escape_js_string(label)))
        .collect::<Vec<_>>()
        .join(",");
    let axis_labels_js = format!(
        "{{{}}}",
        labels_by_axis
            .iter()
            .map(|(axis_key, labels)| {
                format!(
                    "\"{}\":[{}]",
                    axis_key,
                    labels
                        .iter()
                        .map(|label| format!("\"{}\"", escape_js_string(label)))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    );

    const COLORS: [&str; 6] = [
        "#3b82f6", "#10b981", "#f59e0b", "#8b5cf6", "#ef4444", "#ec4899",
    ];

    let mut dataset_entries_js: Vec<String> = Vec::new();
    let mut series_data_items_js = Vec::new();
    let mut option_series_index = 0usize;

    for (idx, series) in rendered.iter().enumerate() {
        let name = escape_js_string(&series.name);
        let color = COLORS[idx % COLORS.len()];
        let x_axis_key = x_axis_key(series.axis_binding.x_index);
        let y_axis_key = y_axis_key(series.axis_binding.y_index);
        let mut parts = vec![
            format!("name:\"{name}\""),
            format!("color:\"{color}\""),
            format!("xAxisKey:\"{x_axis_key}\""),
            format!("yAxisKey:\"{y_axis_key}\""),
            format!("optionSeriesIndex:{option_series_index}"),
        ];
        let x_model = x_axis_models
            .get(&series.axis_binding.x_axis())
            .cloned()
            .unwrap_or(XAxisRenderModel::Numeric);

        match &series.data {
            ParsedSeriesData::Standard(parsed) => {
                let chart_type = match series.mark {
                    SeriesMark::Points => "scatter",
                    SeriesMark::Lines | SeriesMark::LinesPoints => "line",
                    SeriesMark::Bar => "bar",
                    SeriesMark::Boxplot => {
                        unreachable!("boxplot series are parsed separately")
                    }
                };
                let values = match &x_model {
                    XAxisRenderModel::Numeric => {
                        let points = parsed
                            .points
                            .iter()
                            .map(|(x, y)| format!("[{},{}]", x, y))
                            .collect::<Vec<_>>()
                            .join(",");
                        format!("[{}]", points)
                    }
                    XAxisRenderModel::Category(labels) => {
                        let label_index_by_name = labels
                            .iter()
                            .enumerate()
                            .map(|(index, label)| (label.clone(), index))
                            .collect::<std::collections::HashMap<_, _>>();
                        let mut category_points = parsed.points.clone();
                        category_points.sort_by_key(|(x, _)| {
                            label_index_by_name
                                .get(x)
                                .copied()
                                .unwrap_or(usize::MAX)
                        });
                        let dataset_index = dataset_entries_js.len();
                        let source_rows = category_points
                            .iter()
                            .map(|(x, y)| {
                                format!(
                                    "[\"{}\",{}]",
                                    escape_js_string(x),
                                    format_js_number(*y)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(",");
                        dataset_entries_js.push(format!(
                            "{{source:[[\"x\",\"y\"],{source_rows}]}}"
                        ));
                        parts.push(format!("datasetIndex:{dataset_index}"));
                        parts.push(
                            "encode:{x:\"x\",y:\"y\",itemName:\"x\",tooltip:[\"y\"]}"
                                .to_string(),
                        );
                        format!(
                            "[{}]",
                            category_points
                                .iter()
                                .map(|(x, y)| {
                                    format!(
                                        "[\"{}\",{}]",
                                        escape_js_string(x),
                                        format_js_number(*y)
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(",")
                        )
                    }
                };
                parts.push(format!("type:\"{chart_type}\""));
                parts.push(format!("values:{values}"));
                match series.mark {
                    SeriesMark::Points => {
                        parts.push("symbol:\"circle\"".to_string());
                        parts.push("symbolSize:12".to_string());
                    }
                    SeriesMark::Lines => {
                        parts.push("lineWidth:3".to_string());
                        parts.push("showSymbol:false".to_string());
                    }
                    SeriesMark::LinesPoints => {
                        parts.push("symbol:\"circle\"".to_string());
                        parts.push("symbolSize:7".to_string());
                        parts.push("lineWidth:3".to_string());
                    }
                    SeriesMark::Bar => {
                        parts.push("barMaxWidth:48".to_string());
                    }
                    SeriesMark::Boxplot => {
                        unreachable!("boxplot series are parsed separately")
                    }
                }
                option_series_index += 1;
            }
            ParsedSeriesData::Boxplot(parsed) => {
                append_boxplot_series_data_parts(
                    &mut parts,
                    &mut dataset_entries_js,
                    parsed,
                    &x_axis_key,
                );
                option_series_index += 2;
            }
        }
        series_data_items_js.push(format!("{{{}}}", parts.join(",")));
    }
    let series_data_js = series_data_items_js.join(",\n");
    let dataset_js = if dataset_entries_js.is_empty() {
        "[]".to_string()
    } else {
        format!("[{}]", dataset_entries_js.join(","))
    };

    let x_axes = build_x_axes(request, prepared, &x_label);
    let y_axes = build_y_axes(request, prepared, &y_label);
    let legend_placement =
        parse_legend_placement(&request.plot.legend.position)?;

    // Build layout cells
    let mut cells: Vec<String> = Vec::new();
    let bfs = request
        .plot
        .theme
        .font
        .as_ref()
        .map(|f| f.size as f64)
        .unwrap_or(12.0);

    // Title cell
    if let Some(title_text) = request
        .plot
        .title
        .as_deref()
        .filter(|title| !title.is_empty())
    {
        cells.push(format!(
            r#"{{id:"title",kind:"title",side:"top",track:2,size:{},minorSpan:"stretch",align:"center",renderArea:{{zIndex:0,size:[{{type:"percent",value:0}},{{type:"percent",value:100}}]}},text:"{}"}}"#,
            bfs * 2.5,
            escape_js_string(title_text),
        ));
    }

    // Legend cell
    if rendered.len() >= 2 {
        let legend_track = match legend_placement.side {
            "top" => 1,
            "bottom" => 2,
            "left" => 1,
            "right" => {
                y_axes
                    .iter()
                    .filter(|axis| axis.side == "right")
                    .map(|axis| axis.track)
                    .max()
                    .unwrap_or(0)
                    + 1
            }
            _ => unreachable!("legend side is validated"),
        };
        cells.push(format!(
            r#"{{id:"legend",kind:"legend",side:"{}",track:{},size:{},minorSpan:"{}",align:"{}"}}"#,
            legend_placement.side,
            legend_track,
            bfs * 2.5,
            legend_placement.minor_span,
            legend_placement.align
        ));
    }

    for axis in &x_axes {
        let x_model = x_axis_models
            .get(&axis.axis_ref)
            .cloned()
            .unwrap_or(XAxisRenderModel::Numeric);
        match x_model {
            XAxisRenderModel::Numeric => {
                let scale_type = if request.plot.axes.get(axis.axis_ref).scale
                    == AxisScale::Log10
                {
                    "log"
                } else {
                    "value"
                };
                cells.push(format!(r#"{{id:"{}",kind:"axis",side:"{}",track:{},size:{},minorSpan:"stretch",axisDimension:"x",scaleType:"{}",numberFormat:{},name:"{}",axisOffset:8,labelMargin:16,nameGap:38,visibilityPolicy:"if-any-bound-series-visible"}}"#, axis.key, axis.side, axis.track, bfs * 4.5, scale_type, axis.number_format, axis.name));
            }
            XAxisRenderModel::Category(labels) => {
                let labels_js = labels
                    .iter()
                    .map(|label| format!("\"{}\"", escape_js_string(label)))
                    .collect::<Vec<_>>()
                    .join(",");
                cells.push(format!(r#"{{id:"{}",kind:"axis",side:"{}",track:{},size:{},minorSpan:"stretch",axisDimension:"x",numberFormat:{},name:"{}",data:[{}],axisOffset:8,labelMargin:16,nameGap:38,visibilityPolicy:"if-any-bound-series-visible"}}"#, axis.key, axis.side, axis.track, bfs * 4.5, axis.number_format, axis.name, labels_js));
            }
        }
    }

    // Data zoom cell
    if !x_axes.is_empty() {
        cells.push(format!(r#"{{id:"x-scale",kind:"data-zoom",side:"bottom",track:1,size:{},minorSpan:"stretch",align:"center"}}"#, bfs * 3.0));
    }

    for axis in &y_axes {
        let scale_type =
            if request.plot.axes.get(axis.axis_ref).scale == AxisScale::Log10 {
                "log"
            } else {
                "value"
            };
        cells.push(format!(r#"{{id:"{}",kind:"axis",side:"{}",track:{},size:0,minorSpan:"stretch",axisDimension:"y",scaleType:"{}",numberFormat:{},name:"{}",labelMargin:10,visibilityPolicy:"if-any-bound-series-visible"}}"#, axis.key, axis.side, axis.track, scale_type, axis.number_format, axis.name));
    }

    let cells_js = cells.join(",\n");

    // Font
    let font_family = request
        .plot
        .theme
        .font
        .as_ref()
        .map(|f| escape_js_string(&f.family))
        .unwrap_or_else(|| "sans-serif".to_string());
    let font_size = request
        .plot
        .theme
        .font
        .as_ref()
        .map(|f| f.size)
        .unwrap_or(12);
    let legend_font_family = request
        .plot
        .legend
        .font
        .as_ref()
        .map(|f| escape_js_string(&f.family))
        .unwrap_or_else(|| font_family.clone());
    let legend_font_size = request
        .plot
        .legend
        .font
        .as_ref()
        .map(|f| f.size)
        .unwrap_or(font_size);

    // Axis ranges
    let mut axis_range_entries = Vec::new();
    let mut axis_tick_entries = Vec::new();
    for axis in &y_axes {
        let axis_spec = request.plot.axes.get(axis.axis_ref);
        let (min, max) = match &axis_spec.range {
            Some(r) => (format!("{}", r.start), format!("{}", r.end)),
            None => ("undefined".to_string(), "undefined".to_string()),
        };
        axis_range_entries
            .push(format!("\"{}\":{{min:{},max:{}}}", axis.key, min, max));
        let tick_min = axis_spec
            .range
            .as_ref()
            .map(|range| range.start)
            .or_else(|| {
                axis_spec.ticks.major.as_ref().and_then(|tick| {
                    tick.range.as_ref().map(|range| range.start)
                })
            });
        let tick_max =
            axis_spec.range.as_ref().map(|range| range.end).or_else(|| {
                axis_spec
                    .ticks
                    .major
                    .as_ref()
                    .and_then(|tick| tick.range.as_ref().map(|range| range.end))
            });
        let tick_interval =
            axis_spec.ticks.major.as_ref().map(|tick| tick.step);
        let custom_ticks = axis_spec
            .ticks
            .custom
            .iter()
            .map(|(value, label)| {
                format!(
                    "[{},\"{}\"]",
                    format_js_number(*value),
                    escape_js_string(label)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        axis_tick_entries.push(format!(
            "\"{}\":{{interval:{},min:{},max:{},custom:[{}]}}",
            axis.key,
            format_optional_js_number(tick_interval),
            format_optional_js_number(tick_min),
            format_optional_js_number(tick_max),
            custom_ticks
        ));
    }
    for axis in &x_axes {
        if !matches!(
            x_axis_models.get(&axis.axis_ref),
            Some(XAxisRenderModel::Numeric) | None
        ) {
            continue;
        }
        let axis_spec = request.plot.axes.get(axis.axis_ref);
        let (min, max) = match &axis_spec.range {
            Some(r) => (format!("{}", r.start), format!("{}", r.end)),
            None => ("undefined".to_string(), "undefined".to_string()),
        };
        axis_range_entries
            .push(format!("\"{}\":{{min:{},max:{}}}", axis.key, min, max));
        let tick_min = axis_spec
            .range
            .as_ref()
            .map(|range| range.start)
            .or_else(|| {
                axis_spec.ticks.major.as_ref().and_then(|tick| {
                    tick.range.as_ref().map(|range| range.start)
                })
            });
        let tick_max =
            axis_spec.range.as_ref().map(|range| range.end).or_else(|| {
                axis_spec
                    .ticks
                    .major
                    .as_ref()
                    .and_then(|tick| tick.range.as_ref().map(|range| range.end))
            });
        let tick_interval =
            axis_spec.ticks.major.as_ref().map(|tick| tick.step);
        let custom_ticks = axis_spec
            .ticks
            .custom
            .iter()
            .map(|(value, label)| {
                format!(
                    "[{},\"{}\"]",
                    format_js_number(*value),
                    escape_js_string(label)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        axis_tick_entries.push(format!(
            "\"{}\":{{interval:{},min:{},max:{},custom:[{}]}}",
            axis.key,
            format_optional_js_number(tick_interval),
            format_optional_js_number(tick_min),
            format_optional_js_number(tick_max),
            custom_ticks
        ));
    }
    let axis_ranges_js = format!("{{{}}}", axis_range_entries.join(","));
    let axis_ticks_js = format!("{{{}}}", axis_tick_entries.join(","));

    let total_original_points = rendered
        .iter()
        .map(|series| match series {
            RenderSeries {
                data: ParsedSeriesData::Standard(series),
                ..
            } => series.original_point_count,
            RenderSeries {
                data: ParsedSeriesData::Boxplot(series),
                ..
            } => series.original_point_count,
        })
        .sum::<usize>();
    let total_embedded_points = rendered
        .iter()
        .map(|series| match series {
            RenderSeries {
                data: ParsedSeriesData::Standard(series),
                ..
            } => series.points.len(),
            RenderSeries {
                data: ParsedSeriesData::Boxplot(series),
                ..
            } => series.groups.iter().map(Vec::len).sum::<usize>(),
        })
        .sum::<usize>();
    let theme = escape_js_string(options.theme.as_deref().unwrap_or("default"));
    let has_plot_title = request
        .plot
        .title
        .as_deref()
        .is_some_and(|title| !title.is_empty());
    let chart_width_px = (request.plot.layout.width * DEFAULT_CHART_WIDTH_PX)
        .round()
        .max(1.0) as usize;
    let chart_height_px = (request.plot.layout.height * DEFAULT_CHART_HEIGHT_PX)
        .round()
        .max(1.0) as usize;
    let title_cell_js = if has_plot_title {
        "const titleCell = layout.cellsById.title;"
    } else {
        "const titleCell = null;"
    };
    let title_option_js = if has_plot_title {
        r##"  if (titleCell) {
    option.title = {
      text: titleCell.text, left: titleCell.renderRect.x + titleCell.renderRect.width / 2, top: titleCell.renderRect.y,
      z: titleCell.renderRect.zIndex, textAlign: "center",
      textStyle: { color: "#172033", fontSize: 20, fontWeight: 600 },
    };
  }
"##
    } else {
        ""
    };

    // Build the shared embeddable chart snippet.
    Ok(format!(
        r##"<div class="msp-echarts-root" style="width:{chart_width_px}px;max-width:100%;">
<style>
.msp-echarts-root,
.msp-echarts-root * {{
  box-sizing: border-box;
}}
.msp-echarts-root {{
  position: relative;
  display: block;
  color: #172033;
}}
.msp-echarts-canvas {{
  width: 100%;
  height: {chart_height_px}px;
}}
</style>
<div class="msp-echarts-canvas" aria-label="msp echarts chart"></div>
</div>
<script>
(function() {{
const script = document.currentScript;
const root = script.previousElementSibling;
if (!root || !root.classList.contains("msp-echarts-root")) {{
  throw new Error("msp echarts root not found for embedded chart");
}}
const chartNode = root.querySelector(".msp-echarts-canvas");
const echarts = window.echarts;
if (!echarts) {{
  throw new Error("msp echarts runtime missing: load ECharts before this fragment or use --backend-opt runtime=cdn");
}}
const labels = [{labels_js}];
const axisCategoryLabels = {axis_labels_js};
const datasets = {dataset_js};
const seriesData = [
{series_data_js}
];
const seriesVisibility = Object.fromEntries(
  seriesData.map(function(s) {{ return [s.name, true]; }}),
);
var hoveredSeriesName = null;
var hoveredTooltipKey = null;
var hoverResetTimer = null;
var legendClickTimer = null;
const gridDebug = false;
const axisSpacingDebug = false;
const showGrid = {show_grid};
const axisRanges = {axis_ranges_js};
const axisTickConfigs = {axis_ticks_js};

const layoutSpec = {{
  padding: {{ top: 18, right: 18, bottom: 18, left: 18 }},
  gap: 10,
  cells: [
{cells_js}
  ],
}};

const chart = echarts.init(chartNode, "{theme}");
const measureCanvas = document.createElement("canvas");
const measureContext = measureCanvas.getContext("2d");
const layoutTheme = {{
  titleFont: '600 20px "{font_family}", sans-serif',
  axisLabelFont: '{font_size}px "{font_family}", sans-serif',
  axisNameFont: '{font_size}px "{font_family}", sans-serif',
  legendFont: '{legend_font_size}px "{legend_font_family}", sans-serif',
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
const toggleableAxisCells = layoutSpec.cells.filter(function(cell) {{
  return cell.kind === "axis" && cell.scaleType;
}});
const axisScaleMode = Object.fromEntries(
  toggleableAxisCells.map(function(cell) {{ return [cell.id, cell.scaleType === "log"]; }}),
);
const OUTLIER_SERIES_ID_SUFFIX = "__outliers";
var axisHitAreas = [];
var legendHitAreas = [];
var lastLayout = null;
var lastVisibleAxisIds = new Set();
var lastBaseGraphics = [];
var hoveredRenderedSeriesName = null;
var lastAxisToggleEventTime = 0;

function getBaseSeriesName(seriesName) {{
  return seriesName;
}}

function isOutlierSeriesEvent(params) {{
  return Boolean(
    params
    && typeof params.seriesId === "string"
    && params.seriesId.endsWith(OUTLIER_SERIES_ID_SUFFIX)
  );
}}

function shouldUseDirectItemTooltip(seriesName) {{
  const seriesIndex = getSeriesIndexByName(seriesName);
  if (seriesIndex < 0) {{ return false; }}
  return seriesData[seriesIndex].type === "boxplot";
}}

function getCategoryBandWidth(seriesItem) {{
  const axisIndexes = getAxisIndexMaps();
  const xAxisIndex = axisIndexes.x[seriesItem.xAxisKey];
  if (xAxisIndex === undefined) {{ return 0; }}
  const axisLabels = axisCategoryLabels[seriesItem.xAxisKey] || [];
  if (axisLabels.length >= 2) {{
    const firstPixel = chart.convertToPixel(
      {{ xAxisIndex: xAxisIndex }},
      axisLabels[0],
    );
    const secondPixel = chart.convertToPixel(
      {{ xAxisIndex: xAxisIndex }},
      axisLabels[1],
    );
    if (Number.isFinite(firstPixel) && Number.isFinite(secondPixel)) {{
      return Math.abs(secondPixel - firstPixel);
    }}
  }}
  if (axisLabels.length === 1 && lastLayout) {{
    return lastLayout.plotRect.width;
  }}
  return 0;
}}

function getBoxplotOffsetPixels(seriesItem) {{
  const siblingBoxplots = getVisibleSeries().filter(function(candidate) {{
    return candidate.type === "boxplot"
      && candidate.xAxisKey === seriesItem.xAxisKey
      && candidate.yAxisKey === seriesItem.yAxisKey;
  }});
  if (siblingBoxplots.length <= 1) {{ return 0; }}
  const seriesIndex = siblingBoxplots.findIndex(function(candidate) {{
    return candidate === seriesItem;
  }});
  if (seriesIndex < 0) {{ return 0; }}
  const bandWidth = getCategoryBandWidth(seriesItem);
  if (!Number.isFinite(bandWidth) || bandWidth <= 0) {{ return 0; }}
  const availableWidth = Math.max(bandWidth * 0.8 - 2, 0);
  const boxGap = availableWidth / siblingBoxplots.length * 0.3;
  const boxWidth = (availableWidth - boxGap * (siblingBoxplots.length - 1))
    / siblingBoxplots.length;
  const baseOffset = boxWidth / 2 - availableWidth / 2;
  return baseOffset + seriesIndex * (boxGap + boxWidth);
}}

// Keep every boxplot rendered as a deliberate pair: native ECharts boxplot for
// quartiles/whiskers plus a custom outlier overlay that reuses the same
// grouped-category offset. ECharts exposes outliers as a separate dataset
// result, and a plain scatter series would fall back to the category center.
function buildBoxplotOutlierSeries(seriesItem, axisIndexes, isHovered) {{
  return {{
    id: seriesItem.name + OUTLIER_SERIES_ID_SUFFIX,
    name: seriesItem.name,
    type: "custom",
    data: seriesItem.outlierValues !== undefined ? seriesItem.outlierValues : undefined,
    datasetIndex: seriesItem.outlierDatasetIndex,
    encode: seriesItem.outlierEncode,
    xAxisIndex: axisIndexes.x[seriesItem.xAxisKey],
    yAxisIndex: axisIndexes.y[seriesItem.yAxisKey],
    clip: true,
    z: isHovered ? 4 : 2,
    renderItem: function(params, api) {{
      const xValue = api.value(0);
      const yValue = api.value(1);
      const point = api.coord([xValue, yValue]);
      if (!Array.isArray(point) || point.length < 2) {{ return null; }}
      if (!Number.isFinite(point[0]) || !Number.isFinite(point[1])) {{ return null; }}
      const radius = 3;
      return {{
        type: "circle",
        shape: {{
          cx: point[0] + getBoxplotOffsetPixels(seriesItem),
          cy: point[1],
          r: radius,
        }},
        style: {{
          fill: seriesItem.color,
          stroke: seriesItem.color,
        }},
      }};
    }},
    tooltip: buildItemTooltipConfig(seriesItem),
    emphasis: {{ disabled: true }},
  }};
}}

function buildItemTooltipConfig(seriesItem) {{
  return {{
    trigger: "item",
    borderColor: seriesItem.color,
    borderWidth: 1,
    formatter: function(params) {{ return formatHoveredSeriesTooltip(params); }},
  }};
}}

function buildRenderableSeries(seriesItem, axisIndexes, activeSeriesName) {{
  const isHovered = activeSeriesName === seriesItem.name;
  const primarySeries = {{
    name: seriesItem.name, type: seriesItem.type,
    data: seriesItem.renderData !== undefined
      ? seriesItem.renderData
      : (seriesItem.datasetIndex === undefined ? seriesItem.values : undefined),
    xAxisIndex: axisIndexes.x[seriesItem.xAxisKey],
    yAxisIndex: axisIndexes.y[seriesItem.yAxisKey],
    datasetIndex: seriesItem.datasetIndex,
    encode: seriesItem.encode,
    clip: true, smooth: Boolean(seriesItem.smooth),
    symbol: seriesItem.symbol || (seriesItem.showSymbol === false ? "none" : "circle"),
    symbolSize: seriesItem.symbolSize || 7,
    triggerLineEvent: seriesItem.type === "line",
    barMaxWidth: seriesItem.barMaxWidth,
    z: isHovered ? 4 : 2,
    lineStyle: seriesItem.type === "line" ? {{ width: seriesItem.lineWidth || 3, color: seriesItem.color }} : undefined,
    areaStyle: seriesItem.areaStyle,
    itemStyle: {{ color: seriesItem.color, borderColor: seriesItem.color }},
    tooltip: seriesItem.type === "boxplot"
      ? buildItemTooltipConfig(seriesItem)
      : undefined,
    emphasis: {{ disabled: true }},
  }};
  if (seriesItem.type !== "boxplot") {{
    return [primarySeries];
  }}
  return [primarySeries, buildBoxplotOutlierSeries(seriesItem, axisIndexes, isHovered)];
}}

function isAxisLogScale(cell) {{
  return Boolean(axisScaleMode[cell.id]);
}}

function toggleAxisLogScale(axisId) {{
  if (!(axisId in axisScaleMode)) {{ return; }}
  axisScaleMode[axisId] = !axisScaleMode[axisId];
  renderChart();
}}

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

function visitSeriesAxisValues(seriesItem, axisDimension, visitor) {{
  if (seriesItem.type === "boxplot" && axisDimension === "y" && Array.isArray(seriesItem.groupedValues)) {{
    for (const group of seriesItem.groupedValues) {{
      if (!Array.isArray(group)) {{ continue; }}
      for (const item of group) {{
        if (typeof item === "number") {{
          visitor(item);
        }}
      }}
    }}
    return;
  }}
  if (!Array.isArray(seriesItem.values)) {{ return; }}
  for (const value of seriesItem.values) {{
    if (typeof value === "number") {{
      visitor(value);
      continue;
    }}
    if (!Array.isArray(value)) {{ continue; }}
    if (axisDimension === "x") {{
      if (typeof value[0] === "number") {{
        visitor(value[0]);
      }}
      continue;
    }}
    if (typeof value[1] === "number") {{
      visitor(value[1]);
    }} else if (typeof value[0] === "number") {{
      visitor(value[0]);
    }}
  }}
}}

function usesBandPositioning(seriesItem) {{
  return seriesItem.type === "bar" || seriesItem.type === "boxplot";
}}

function shouldUseCategoryBoundaryGap(axisId) {{
  return getSeriesBoundToAxis(axisId, "x").some(usesBandPositioning);
}}

function collectSeriesAxisValues(seriesItem, axisDimension) {{
  const values = [];
  visitSeriesAxisValues(seriesItem, axisDimension, function(value) {{
    values.push(value);
  }});
  return values;
}}

function getLogAxisMin(axisId, axisDimension) {{
  var minimum = undefined;
  getSeriesBoundToAxis(axisId, axisDimension).forEach(function(seriesItem) {{
    visitSeriesAxisValues(seriesItem, axisDimension, function(value) {{
      if (value > 0 && (minimum === undefined || value < minimum)) {{
        minimum = value;
      }}
    }});
  }});
  return minimum === undefined ? 1 : minimum;
}}

function nearlyEqual(left, right) {{
  return Math.abs(left - right) <= Math.max(1e-9, Math.max(Math.abs(left), Math.abs(right)) * 1e-9);
}}

function findCustomTickLabel(axisId, value) {{
  const tickConfig = axisTickConfigs[axisId];
  if (!tickConfig || !Array.isArray(tickConfig.custom)) {{ return null; }}
  const matched = tickConfig.custom.find(function(entry) {{
    return Array.isArray(entry)
      && entry.length >= 2
      && typeof entry[0] === "number"
      && nearlyEqual(entry[0], value);
  }});
  return matched ? matched[1] : null;
}}

function hasCustomTicks(axisId) {{
  const tickConfig = axisTickConfigs[axisId];
  return Boolean(
    tickConfig
      && Array.isArray(tickConfig.custom)
      && tickConfig.custom.length > 0
  );
}}

function getSortedCustomTickValues(axisId) {{
  const tickConfig = axisTickConfigs[axisId];
  if (!tickConfig || !Array.isArray(tickConfig.custom)) {{ return []; }}
  return tickConfig.custom
    .filter(function(entry) {{
      return Array.isArray(entry)
        && entry.length >= 1
        && typeof entry[0] === "number"
        && Number.isFinite(entry[0]);
    }})
    .map(function(entry) {{ return entry[0]; }})
    .sort(function(left, right) {{ return left - right; }});
}}

function getAxisTickInterval(tickConfig, axisId, isLogScale) {{
  if (tickConfig.interval !== undefined || isLogScale) {{
    return tickConfig.interval;
  }}
  const customTickValues = getSortedCustomTickValues(axisId);
  if (customTickValues.length < 2) {{
    return tickConfig.interval;
  }}
  const firstStep = customTickValues[1] - customTickValues[0];
  if (!Number.isFinite(firstStep) || firstStep <= 0) {{
    return tickConfig.interval;
  }}
  for (var index = 2; index < customTickValues.length; index += 1) {{
    if (!nearlyEqual(customTickValues[index] - customTickValues[index - 1], firstStep)) {{
      return tickConfig.interval;
    }}
  }}
  return firstStep;
}}

function getPositiveCustomTickMin(axisId) {{
  const customTickValues = getSortedCustomTickValues(axisId).filter(function(value) {{
    return value > 0;
  }});
  return customTickValues.length > 0 ? customTickValues[0] : undefined;
}}

function getPositiveAxisMax(axisId, axisDimension) {{
  var maximum = undefined;
  getSeriesBoundToAxis(axisId, axisDimension).forEach(function(seriesItem) {{
    visitSeriesAxisValues(seriesItem, axisDimension, function(value) {{
      if (value > 0 && (maximum === undefined || value > maximum)) {{
        maximum = value;
      }}
    }});
  }});
  return maximum;
}}

function shouldHideNativeAxisTicks(cell, isNumericX) {{
  return hasCustomTicks(cell.id) && (cell.axisDimension === "y" || isNumericX);
}}

function resolveAxisMin(range, tickConfig, axisId, axisDimension, isLogScale) {{
  const explicitMin = range.min !== undefined
    ? range.min
    : (tickConfig.min !== undefined ? tickConfig.min : undefined);
  if (!isLogScale) {{
    return explicitMin;
  }}
  if (explicitMin !== undefined && explicitMin > 0) {{
    return explicitMin;
  }}
  const customTickMin = getPositiveCustomTickMin(axisId);
  if (customTickMin !== undefined) {{
    return customTickMin;
  }}
  return getLogAxisMin(axisId, axisDimension);
}}

function resolveAxisMax(range, tickConfig, axisId, axisDimension, isLogScale, minValue) {{
  const explicitMax = range.max !== undefined
    ? range.max
    : (tickConfig.max !== undefined ? tickConfig.max : undefined);
  if (!isLogScale) {{
    return explicitMax;
  }}
  if (explicitMax !== undefined && explicitMax > 0 && (minValue === undefined || explicitMax > minValue)) {{
    return explicitMax;
  }}
  const dataMax = getPositiveAxisMax(axisId, axisDimension);
  if (dataMax !== undefined && (minValue === undefined || dataMax > minValue)) {{
    return dataMax;
  }}
  return explicitMax !== undefined && explicitMax > 0 ? explicitMax : undefined;
}}

function trimTrailingZeroes(valueText) {{
  return valueText.replace(/(\.\d*?[1-9])0+$/u, "$1").replace(/\.0+$/u, "");
}}

function normalizeDecimals(decimals, fallback) {{
  return Number.isInteger(decimals) && decimals >= 0 ? decimals : fallback;
}}

function resolveFormatSpec(formatSpec) {{
  return formatSpec && typeof formatSpec === "object"
    ? formatSpec
    : {{ mode: "plain", decimals: undefined }};
}}

function formatPlainValue(value, decimals) {{
  const normalizedDecimals = normalizeDecimals(decimals, undefined);
  if (normalizedDecimals === undefined) {{
    return echarts.format.addCommas(value);
  }}
  return value.toLocaleString(undefined, {{
    minimumFractionDigits: normalizedDecimals,
    maximumFractionDigits: normalizedDecimals,
  }});
}}

function formatScientificValue(value, decimals) {{
  const precision = normalizeDecimals(decimals, 3);
  const parts = value.toExponential(precision).split("e");
  return trimTrailingZeroes(parts[0]) + "e" + parts[1];
}}

function formatSuffixValue(value, decimals) {{
  const suffixes = ["K", "M", "G", "T", "P", "E"];
  const absValue = Math.abs(value);
  if (absValue < 1000) {{
    return formatPlainValue(value, decimals);
  }}
  var scaled = absValue;
  var suffixIndex = -1;
  while (scaled >= 1000 && suffixIndex < suffixes.length - 1) {{
    scaled /= 1000;
    suffixIndex += 1;
  }}
  const precision = normalizeDecimals(
    decimals,
    scaled >= 100 ? 0 : (scaled >= 10 ? 1 : 2),
  );
  const mantissa = trimTrailingZeroes(scaled.toFixed(precision));
  return (value < 0 ? "-" : "") + mantissa + suffixes[suffixIndex];
}}

function formatPercentageValue(value, decimals) {{
  const precision = normalizeDecimals(decimals, 2);
  return trimTrailingZeroes((value * 100).toFixed(precision)) + "%";
}}

function timestampToMillis(value, unit) {{
  if (unit === "s") {{
    return value * 1000;
  }}
  return value;
}}

function formatTimestampValue(value, formatSpec) {{
  const formatterOptions = {{
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }};
  if (formatSpec.timezone) {{
    formatterOptions.timeZone = formatSpec.timezone;
  }}
  return new Intl.DateTimeFormat(
    undefined,
    formatterOptions,
  ).format(new Date(timestampToMillis(value, formatSpec.unit || "ms")));
}}

function formatAxisValue(value, isLogScale, formatSpec) {{
  if (!Number.isFinite(value)) {{ return ""; }}
  if (isLogScale && value <= 0) {{ return ""; }}
  const normalizedFormat = resolveFormatSpec(formatSpec);
  if (normalizedFormat.mode === "scientific") {{
    return formatScientificValue(value, normalizedFormat.decimals);
  }}
  if (normalizedFormat.mode === "suffix") {{
    return formatSuffixValue(value, normalizedFormat.decimals);
  }}
  if (normalizedFormat.mode === "percentage") {{
    return formatPercentageValue(value, normalizedFormat.decimals);
  }}
  if (normalizedFormat.mode === "timestamp") {{
    return formatTimestampValue(value, normalizedFormat);
  }}
  return formatPlainValue(value, normalizedFormat.decimals);
}}

function formatAxisLabel(axisId, value, isLogScale, numberFormat) {{
  const customLabel = findCustomTickLabel(axisId, value);
  if (customLabel !== null) {{
    return customLabel;
  }}
  if (hasCustomTicks(axisId)) {{
    return "";
  }}
  return formatAxisValue(value, isLogScale, numberFormat);
}}

function escapeTooltipHtml(value) {{
  return echarts.format.encodeHTML(value == null ? "" : String(value));
}}

function getValueAxisNameGap(cell, bandSize) {{
  return cell.nameGap;
}}

function getCategoryAxisNameGap(cell, bandSize) {{
  return cell.nameGap || Math.max(28, bandSize - 18);
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
  const itemLabels = seriesData.map(function(s) {{ return s.name; }});
  const rowHeight = measureText("Series A", layoutTheme.legendFont).height + layoutTheme.legendPadding;
  var rows = 1;
  var rowWidth = 0;
  var maxRowWidth = 0;
  var rowIndex = 0;
  const items = [];
  itemWidths.forEach(function(itemWidth, index) {{
    const nextWidth = rowWidth + (index === 0 || rowWidth === 0 ? 0 : layoutTheme.legendItemGap) + itemWidth;
    if (nextWidth > safeWidth && rowWidth > 0) {{
      maxRowWidth = Math.max(maxRowWidth, rowWidth);
      rows += 1;
      rowIndex += 1;
      rowWidth = itemWidth;
      items.push({{
        name: itemLabels[index],
        x: 0,
        y: rowIndex * (rowHeight + layoutTheme.legendRowGap),
        width: itemWidth,
        height: rowHeight,
      }});
      return;
    }}
    const itemX = rowWidth === 0 ? 0 : rowWidth + layoutTheme.legendItemGap;
    items.push({{
      name: itemLabels[index],
      x: itemX,
      y: rowIndex * (rowHeight + layoutTheme.legendRowGap),
      width: itemWidth,
      height: rowHeight,
    }});
    rowWidth = nextWidth;
  }});
  maxRowWidth = Math.max(maxRowWidth, rowWidth);
  return {{
    width: Math.min(safeWidth, maxRowWidth + layoutTheme.legendWidthBuffer),
    height: rows * rowHeight + (rows - 1) * layoutTheme.legendRowGap,
    items: items,
  }};
}}

function getYAxisLabelBlock(cell) {{
  const isLogScale = isAxisLogScale(cell);
  const boundSeries = getSeriesBoundToAxis(cell.id, "y");
  var widestLabel = measureText(
    formatAxisValue(0, isLogScale, cell.numberFormat),
    layoutTheme.axisLabelFont,
  ).width;
  boundSeries.forEach(function(s) {{
    visitSeriesAxisValues(s, "y", function(value) {{
      const labelWidth = measureText(formatAxisValue(
        value,
        isLogScale,
        cell.numberFormat,
      ), layoutTheme.axisLabelFont).width;
      if (labelWidth > widestLabel) {{
        widestLabel = labelWidth;
      }}
    }});
  }});
  return layoutTheme.axisTickLengthY + (cell.labelMargin || 10) + widestLabel;
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
  const labelBlock = getYAxisLabelBlock(cell);
  if (!cell.name) {{ return Math.ceil(labelBlock + layoutTheme.axisPadding); }}
  const nameHeight = measureText(cell.name, layoutTheme.axisNameFont).height;
  const nameGap = cell.nameGap !== undefined ? cell.nameGap : 15;
  return Math.ceil(labelBlock + nameGap + nameHeight + layoutTheme.axisPadding);
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
  const isLogScale = isAxisLogScale(cell);
  const bandSize = cell.bandRect ? (isX ? cell.bandRect.height : cell.bandRect.width) : cell.size;
  const useBoundaryGap = isX ? shouldUseCategoryBoundaryGap(cell.id) : false;
  const isNumericX = isX && !cell.data;
  const hideNativeTicks = shouldHideNativeAxisTicks(cell, isNumericX);
  const range = axisRanges[cell.id] || {{}};
  const tickConfig = axisTickConfigs[cell.id] || {{}};
  const axisDimension = isX ? "x" : "y";
  const minValue = resolveAxisMin(range, tickConfig, cell.id, axisDimension, isLogScale);
  const maxValue = resolveAxisMax(range, tickConfig, cell.id, axisDimension, isLogScale, minValue);
  const tickInterval = getAxisTickInterval(tickConfig, cell.id, isLogScale);
  if (isX) {{
    return {{
      id: cell.id,
      show: isVisible,
      type: isNumericX ? (isLogScale ? "log" : "value") : "category",
      position: cell.side,
      offset: trackOffset + (cell.axisOffset || 0),
      name: cell.name || "",
      nameLocation: "middle",
      nameGap: getCategoryAxisNameGap(cell, bandSize),
      nameTextStyle: {{ color: "#172033", fontSize: getFontSize(layoutTheme.axisNameFont) }},
      boundaryGap: isNumericX ? false : useBoundaryGap,
      data: isNumericX ? undefined : cell.data,
      min: isNumericX ? minValue : undefined,
      max: isNumericX ? maxValue : undefined,
      interval: isNumericX ? tickInterval : undefined,
      logBase: isNumericX && isLogScale ? 10 : undefined,
      axisLine: {{ show: isVisible, lineStyle: {{ color: cell.side === "top" ? "#d7deeb" : "#c7d2e5" }} }},
      axisTick: {{ show: isVisible && !hideNativeTicks, length: 8, alignWithLabel: isNumericX ? undefined : useBoundaryGap }},
      axisLabel: {{
        show: isVisible && !hideNativeTicks,
        color: cell.side === "top" ? "#7b879c" : "#5f6b85",
        margin: cell.labelMargin || 10,
        formatter: isNumericX
          ? function(value) {{ return formatAxisLabel(cell.id, value, isLogScale, cell.numberFormat); }}
          : undefined,
      }},
      splitLine: {{ show: showGrid && !hideNativeTicks, lineStyle: {{ color: "#eef2fb" }} }},
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
    nameGap: 0,
    min: minValue,
    max: maxValue,
    interval: tickInterval,
    logBase: isLogScale ? 10 : undefined,
    axisLine: {{ show: isVisible, lineStyle: {{ color: cell.side === "left" ? "#c7d2e5" : "#d7deeb" }} }},
    axisTick: {{ show: isVisible && !hideNativeTicks, length: 6 }},
    axisLabel: {{
      show: isVisible && !hideNativeTicks,
      color: cell.side === "left" ? "#5f6b85" : "#7b879c",
      margin: cell.labelMargin || 10,
      formatter: function(value) {{ return formatAxisLabel(cell.id, value, isLogScale, cell.numberFormat); }},
    }},
    splitLine: {{ show: showGrid && !hideNativeTicks, lineStyle: {{ color: "#e6ebf5" }} }},
  }};
}}

function buildAxisTitleGraphic(cell) {{
  if (!cell.name) {{ return []; }}
  const titleMetrics = measureText(cell.name, layoutTheme.axisNameFont);
  const inwardOffset = titleMetrics.height / 2;
  var anchorX = cell.renderRect.x + cell.renderRect.width / 2;
  var anchorY = cell.renderRect.y + cell.renderRect.height / 2;
  var rotation = 0;
  if (cell.axisDimension === "y") {{
    const labelBlock = getYAxisLabelBlock(cell);
    const titleBorderX = cell.side === "left"
      ? cell.renderRect.x + cell.renderRect.width - labelBlock
      : cell.renderRect.x + labelBlock;
    const titleGap = cell.nameGap !== undefined ? cell.nameGap : 15;
    anchorX = cell.side === "left"
      ? titleBorderX - titleGap - inwardOffset
      : titleBorderX + titleGap + inwardOffset;
    rotation = cell.side === "left" ? Math.PI / 2 : -Math.PI / 2;
  }} else {{
    anchorY = cell.side === "top" ? cell.renderRect.y + inwardOffset : cell.renderRect.y + cell.renderRect.height - inwardOffset;
  }}
  return [{{
    type: "text",
    id: "axis-title-" + cell.id,
    silent: true,
    x: anchorX,
    y: anchorY,
    rotation: rotation,
    z: 30,
    style: {{
      text: cell.name,
      fill: "#172033",
      font: layoutTheme.axisNameFont,
      textAlign: "center",
      textVerticalAlign: "middle",
    }},
  }}];
}}

function buildAxisTitleGraphics(layout, visibleAxisIds) {{
  return layoutSpec.cells.flatMap(function(axisCell) {{
    if (axisCell.kind !== "axis" || axisCell.axisDimension !== "y" || !visibleAxisIds.has(axisCell.id)) {{ return []; }}
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

function getAxisLinePixel(cell, layout) {{
  const baseCell = layoutSpec.cells.find(function(item) {{ return item.id === cell.id; }}) || {{}};
  const trackOffset = (layout.trackOffsets && layout.trackOffsets[cell.id]) || 0;
  const axisOffset = baseCell.axisOffset || 0;
  if (cell.axisDimension === "x") {{
    if (cell.side === "top") {{
      return layout.plotRect.y - trackOffset - axisOffset;
    }}
    return layout.plotRect.y + layout.plotRect.height + trackOffset + axisOffset;
  }}
  if (cell.side === "left") {{
    return layout.plotRect.x - trackOffset;
  }}
  return layout.plotRect.x + layout.plotRect.width + trackOffset;
}}

function buildSingleCustomTickGraphics(cell, layout, axisIndexes, tickValue, tickLabel, tickIndex) {{
  const axisIndex = axisIndexes[cell.axisDimension][cell.id];
  if (axisIndex === undefined) {{ return []; }}
  const valuePixel = chart.convertToPixel(
    cell.axisDimension === "x" ? {{ xAxisIndex: axisIndex }} : {{ yAxisIndex: axisIndex }},
    tickValue,
  );
  const axisCoordinate = Array.isArray(valuePixel)
    ? (cell.axisDimension === "x" ? valuePixel[0] : valuePixel[1])
    : valuePixel;
  if (!Number.isFinite(axisCoordinate)) {{ return []; }}
  const probePixel = cell.axisDimension === "x"
    ? [axisCoordinate, layout.plotRect.y + layout.plotRect.height / 2]
    : [layout.plotRect.x + layout.plotRect.width / 2, axisCoordinate];
  if (!chart.containPixel({{ gridIndex: 0 }}, probePixel)) {{ return []; }}
  const axisLinePixel = getAxisLinePixel(cell, layout);
  const labelMargin = cell.labelMargin || 10;
  const tickLength = cell.axisDimension === "x" ? layoutTheme.axisTickLengthX : layoutTheme.axisTickLengthY;
  const graphicItems = [];
  if (showGrid) {{
    graphicItems.push({{
      type: "line",
      id: "custom-grid-" + cell.id + "-" + tickIndex,
      silent: true,
      z: 5,
      shape: cell.axisDimension === "x"
        ? {{ x1: axisCoordinate, y1: layout.plotRect.y, x2: axisCoordinate, y2: layout.plotRect.y + layout.plotRect.height }}
        : {{ x1: layout.plotRect.x, y1: axisCoordinate, x2: layout.plotRect.x + layout.plotRect.width, y2: axisCoordinate }},
      style: {{ stroke: cell.axisDimension === "x" ? "#eef2fb" : "#e6ebf5", lineWidth: 1 }},
    }});
  }}
  if (cell.axisDimension === "x") {{
    const tickDirection = cell.side === "top" ? -1 : 1;
    graphicItems.push(
      {{
        type: "line",
        id: "custom-tick-line-" + cell.id + "-" + tickIndex,
        silent: true,
        z: 20,
        shape: {{
          x1: axisCoordinate,
          y1: axisLinePixel,
          x2: axisCoordinate,
          y2: axisLinePixel + tickDirection * tickLength,
        }},
        style: {{ stroke: cell.side === "top" ? "#d7deeb" : "#c7d2e5", lineWidth: 1 }},
      }},
      {{
        type: "text",
        id: "custom-tick-label-" + cell.id + "-" + tickIndex,
        silent: true,
        z: 21,
        x: axisCoordinate,
        y: axisLinePixel + tickDirection * (tickLength + labelMargin),
        style: {{
          text: tickLabel,
          fill: cell.side === "top" ? "#7b879c" : "#5f6b85",
          font: layoutTheme.axisLabelFont,
          textAlign: "center",
          textVerticalAlign: cell.side === "top" ? "bottom" : "top",
        }},
      }},
    );
    return graphicItems;
  }}
  const tickDirection = cell.side === "left" ? -1 : 1;
  graphicItems.push(
    {{
      type: "line",
      id: "custom-tick-line-" + cell.id + "-" + tickIndex,
      silent: true,
      z: 20,
      shape: {{
        x1: axisLinePixel,
        y1: axisCoordinate,
        x2: axisLinePixel + tickDirection * tickLength,
        y2: axisCoordinate,
      }},
      style: {{ stroke: cell.side === "left" ? "#c7d2e5" : "#d7deeb", lineWidth: 1 }},
    }},
    {{
      type: "text",
      id: "custom-tick-label-" + cell.id + "-" + tickIndex,
      silent: true,
      z: 21,
      x: axisLinePixel + tickDirection * (tickLength + labelMargin),
      y: axisCoordinate,
      style: {{
        text: tickLabel,
        fill: cell.side === "left" ? "#5f6b85" : "#7b879c",
        font: layoutTheme.axisLabelFont,
        textAlign: cell.side === "left" ? "right" : "left",
        textVerticalAlign: "middle",
      }},
    }},
  );
  return graphicItems;
}}

function buildCustomTickGraphics() {{
  if (!lastLayout) {{ return []; }}
  const axisIndexes = getAxisIndexMaps();
  return layoutSpec.cells.flatMap(function(axisCell) {{
    if (axisCell.kind !== "axis" || !lastVisibleAxisIds.has(axisCell.id) || !hasCustomTicks(axisCell.id)) {{
      return [];
    }}
    const positionedCell = lastLayout.cellsById[axisCell.id];
    if (!positionedCell) {{ return []; }}
    const tickConfig = axisTickConfigs[axisCell.id];
    if (!tickConfig || !Array.isArray(tickConfig.custom)) {{ return []; }}
    return tickConfig.custom.flatMap(function(entry, tickIndex) {{
      if (!Array.isArray(entry) || entry.length < 2 || typeof entry[0] !== "number") {{
        return [];
      }}
      return buildSingleCustomTickGraphics(
        positionedCell,
        lastLayout,
        axisIndexes,
        entry[0],
        String(entry[1]),
        tickIndex,
      );
    }});
  }});
}}

function updateCustomTickGraphics() {{
  const combinedGraphics = lastBaseGraphics.concat(buildCustomTickGraphics());
  chart.setOption({{
    graphic: combinedGraphics,
  }}, {{
    replaceMerge: ["graphic"],
    lazyUpdate: true,
    silent: true,
  }});
}}

function updateAxisHitAreas(layout, visibleAxisIds) {{
  axisHitAreas = layoutSpec.cells
    .filter(function(axisCell) {{
      return axisCell.kind === "axis" && axisCell.scaleType && visibleAxisIds.has(axisCell.id);
    }})
    .map(function(axisCell) {{
      const cell = layout.cellsById[axisCell.id];
      return {{ id: axisCell.id, rect: cell.renderRect }};
    }});
}}

function updateLegendHitAreas(legendCell, legendLayout) {{
  if (!legendCell || !legendLayout || !Array.isArray(legendLayout.items)) {{
    legendHitAreas = [];
    return;
  }}
  const originX = legendCell.renderRect.x + Math.max(0, (legendCell.renderRect.width - legendLayout.width) / 2);
  const originY = legendCell.renderRect.y + Math.max(0, (legendCell.renderRect.height - legendLayout.height) / 2);
  legendHitAreas = legendLayout.items.map(function(item) {{
    return {{
      name: item.name,
      rect: {{
        x: originX + item.x,
        y: originY + item.y,
        width: item.width,
        height: item.height,
      }},
    }};
  }});
}}

function isPixelWithinRect(pixel, rect) {{
  return Array.isArray(pixel)
    && pixel.length >= 2
    && pixel[0] >= rect.x
    && pixel[0] <= rect.x + rect.width
    && pixel[1] >= rect.y
    && pixel[1] <= rect.y + rect.height;
}}

function findAxisHitArea(pixel) {{
  return axisHitAreas.find(function(area) {{
    return isPixelWithinRect(pixel, area.rect);
  }}) || null;
}}

function findLegendHitArea(pixel) {{
  return legendHitAreas.find(function(area) {{
    return isPixelWithinRect(pixel, area.rect);
  }}) || null;
}}

function setInteractiveCursor(cursorStyle) {{
  const nextCursor = cursorStyle || "default";
  chartNode.style.cursor = nextCursor;
  const zr = chart.getZr();
  if (zr && typeof zr.setCursorStyle === "function") {{
    zr.setCursorStyle(nextCursor);
  }}
}}

function getAxisIdsForSeries(seriesItem) {{
  return new Set([seriesItem.xAxisKey, seriesItem.yAxisKey]);
}}

function clearHoverResetTimer() {{
  if (hoverResetTimer !== null) {{ window.clearTimeout(hoverResetTimer); hoverResetTimer = null; }}
}}

function clearLegendClickTimer() {{
  if (legendClickTimer !== null) {{ window.clearTimeout(legendClickTimer); legendClickTimer = null; }}
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
  {title_cell_js}
  const legendCell = layout.cellsById.legend;
  const scaleCell = layout.cellsById["x-scale"];
  const legendLayout = legendCell ? measureLegendLayout(legendCell.renderRect.width) : {{ width: 0, height: 0 }};
  const axisCells = resolvedSpec.cells.filter(function(cell) {{ return cell.kind === "axis"; }});
  const xAxes = [];
  const yAxes = [];
  const xAxisIndexById = {{}};
  const yAxisIndexById = {{}};
  var primaryXAxisIndex = null;
  axisCells.forEach(function(cell) {{
    const isActive = visibleAxisIds.has(cell.id);
    const positionedCell = layout.cellsById[cell.id];
    const axisOption = buildAxisOption(positionedCell, layout.trackOffsets[cell.id] || 0, isActive);
    if (cell.axisDimension === "x") {{
      xAxisIndexById[cell.id] = xAxes.length;
      if (primaryXAxisIndex === null) {{ primaryXAxisIndex = xAxes.length; }}
      xAxes.push(axisOption);
    }} else {{
      yAxisIndexById[cell.id] = yAxes.length;
      yAxes.push(axisOption);
    }}
  }});
  updateAxisHitAreas(layout, visibleAxisIds);
  updateLegendHitAreas(legendCell, legendLayout);
  lastLayout = layout;
  lastVisibleAxisIds = visibleAxisIds;
  lastBaseGraphics = buildAxisTitleGraphics(layout, visibleAxisIds)
    .concat(gridDebug ? buildGridDebugGraphic(layout) : []);
  const option = {{
    animation: false,
    tooltip: activeSeriesName
      ? {{
        trigger: "item", triggerOn: "none",
        formatter: function(params) {{ return formatHoveredSeriesTooltip(params); }},
      }}
      : {{ trigger: "axis" }},
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
      }},
      {{
        id: "x1-wheel-zoom", type: "inside",
        xAxisIndex: primaryXAxisIndex !== null ? [primaryXAxisIndex] : [],
        filterMode: "filter", realtime: true,
        zoomOnMouseWheel: true, moveOnMouseWheel: false, moveOnMouseMove: true,
      }}
    ] : [],
    graphic: lastBaseGraphics,
    dataset: datasets,
    xAxis: xAxes,
    yAxis: yAxes,
    series: seriesData.flatMap(function(seriesItem) {{
      return buildRenderableSeries(
        seriesItem,
        {{ x: xAxisIndexById, y: yAxisIndexById }},
        activeSeriesName,
      );
    }}),
  }};
{title_option_js}
  if (legendCell) {{
    option.legend = {{
      data: seriesData.map(function(s) {{ return s.name; }}), selected: seriesVisibility,
      selectedMode: false,
      top: legendCell.renderRect.y + Math.max(0, (legendCell.renderRect.height - legendLayout.height) / 2),
      left: legendCell.renderRect.x + Math.max(0, (legendCell.renderRect.width - legendLayout.width) / 2),
      width: legendLayout.width, z: legendCell.renderRect.zIndex,
      itemWidth: layoutTheme.legendItemWidth, itemHeight: layoutTheme.legendItemHeight, itemGap: 12,
      textStyle: {{ color: "#5f6b85", fontFamily: "{legend_font_family}", fontSize: {legend_font_size} }},
    }};
  }}
  return option;
}}

function readCurrentDataZoomState() {{
  const currentOption = chart.getOption();
  if (!currentOption || !Array.isArray(currentOption.dataZoom)) {{ return null; }}
  const zoomState = {{}};
  currentOption.dataZoom.forEach(function(item) {{
    if (!item || !item.id) {{ return; }}
    const itemState = {{}};
    ["start", "end", "startValue", "endValue"].forEach(function(key) {{
      if (item[key] !== undefined) {{
        itemState[key] = item[key];
      }}
    }});
    if (Object.keys(itemState).length > 0) {{
      zoomState[item.id] = itemState;
    }}
  }});
  return Object.keys(zoomState).length > 0 ? zoomState : null;
}}

function applyDataZoomState(option, zoomState) {{
  if (!zoomState || !Array.isArray(option.dataZoom)) {{ return option; }}
  option.dataZoom = option.dataZoom.map(function(item) {{
    if (!item || !item.id || !zoomState[item.id]) {{ return item; }}
    return Object.assign({{}}, item, zoomState[item.id]);
  }});
  return option;
}}

function resetDataZoom() {{
  const currentOption = chart.getOption();
  if (!currentOption || !Array.isArray(currentOption.dataZoom)) {{ return; }}
  currentOption.dataZoom.forEach(function(_, dataZoomIndex) {{
    chart.dispatchAction({{
      type: "dataZoom",
      dataZoomIndex: dataZoomIndex,
      start: 0,
      end: 100,
    }});
  }});
}}

function clearHoveredTooltip() {{
  hoveredTooltipKey = null;
  chart.dispatchAction({{ type: "hideTip" }});
}}

function getAxisIndexMaps() {{
  const currentOption = chart.getOption() || {{}};
  const xAxisIndexById = Object.fromEntries(
    (Array.isArray(currentOption.xAxis) ? currentOption.xAxis : []).map(function(axis, index) {{
      return [axis.id, index];
    }}),
  );
  const yAxisIndexById = Object.fromEntries(
    (Array.isArray(currentOption.yAxis) ? currentOption.yAxis : []).map(function(axis, index) {{
      return [axis.id, index];
    }}),
  );
  return {{ x: xAxisIndexById, y: yAxisIndexById }};
}}

function getSeriesIndexByName(seriesName) {{
  const baseSeriesName = getBaseSeriesName(seriesName);
  return seriesData.findIndex(function(seriesItem) {{ return seriesItem.name === baseSeriesName; }});
}}

function getSeriesPointValue(seriesItem, dataIndex) {{
  const value = seriesItem.values[dataIndex];
  if (Array.isArray(value)) {{ return value; }}
  const axisLabels = axisCategoryLabels[seriesItem.xAxisKey] || labels;
  return [axisLabels[dataIndex], value];
}}

function getAxisCellById(axisId) {{
  return layoutSpec.cells.find(function(cell) {{ return cell.id === axisId; }}) || null;
}}

function formatTooltipAxisValue(axisId, value) {{
  if (typeof value === "number") {{
    const axisCell = getAxisCellById(axisId);
    return formatAxisValue(
      value,
      axisCell ? isAxisLogScale(axisCell) : false,
      axisCell ? axisCell.numberFormat : undefined,
    );
  }}
  return escapeTooltipHtml(value);
}}

function renderTooltipMarker(seriesItem) {{
  const color = escapeTooltipHtml(seriesItem && seriesItem.color ? seriesItem.color : "#5f6b85");
  return "<span style=\"display:inline-block;margin-right:6px;border-radius:50%;width:10px;height:10px;background:" + color + ";border:1px solid " + color + ";\"></span>";
}}

function getBoxplotTooltipStats(boxValues) {{
  if (!Array.isArray(boxValues)) {{ return null; }}
  if (boxValues.length >= 6) {{
    return {{
      min: boxValues[1],
      q1: boxValues[2],
      median: boxValues[3],
      q3: boxValues[4],
      max: boxValues[5],
    }};
  }}
  if (boxValues.length >= 5) {{
    return {{
      min: boxValues[0],
      q1: boxValues[1],
      median: boxValues[2],
      q3: boxValues[3],
      max: boxValues[4],
    }};
  }}
  return null;
}}

function formatHoveredSeriesTooltip(params) {{
  if (!params || params.seriesName == null) {{ return ""; }}
  const seriesIndex = getSeriesIndexByName(params.seriesName);
  if (seriesIndex < 0) {{ return escapeTooltipHtml(params.seriesName); }}
  const seriesItem = seriesData[seriesIndex];
  if (isOutlierSeriesEvent(params)) {{
    const outlierPoint = Array.isArray(params.data)
      ? params.data
      : (Array.isArray(params.value) ? params.value : null);
    const xValue = Array.isArray(outlierPoint) ? outlierPoint[0] : params.name;
    const header = params.axisValueLabel != null && params.axisValueLabel !== ""
      ? escapeTooltipHtml(params.axisValueLabel)
      : formatTooltipAxisValue(seriesItem.xAxisKey, xValue);
    const yValue = Array.isArray(outlierPoint) ? outlierPoint[outlierPoint.length - 1] : outlierPoint;
    return [
      "<div>" + header + "</div>",
      "<div style=\"display:flex;align-items:center;justify-content:space-between;gap:16px;\">"
        + "<span>" + renderTooltipMarker(seriesItem) + escapeTooltipHtml(seriesItem.name) + " outlier</span>"
        + "<span style=\"margin-left:16px;font-weight:600;\">"
        + formatTooltipAxisValue(seriesItem.yAxisKey, yValue)
        + "</span>"
        + "</div>",
    ].join("");
  }}
  const pointValue = getSeriesPointValue(seriesItem, params.dataIndex);
  const xValue = Array.isArray(pointValue) ? pointValue[0] : pointValue;
  const header = params.axisValueLabel != null && params.axisValueLabel !== ""
    ? escapeTooltipHtml(params.axisValueLabel)
    : formatTooltipAxisValue(seriesItem.xAxisKey, xValue);
  if (seriesItem.type === "boxplot") {{
    const boxValues = Array.isArray(params.data)
      ? params.data
      : (Array.isArray(params.value) ? params.value : null);
    const stats = getBoxplotTooltipStats(boxValues);
    if (stats) {{
      return [
        "<div>" + header + "</div>",
        "<div>" + renderTooltipMarker(seriesItem) + escapeTooltipHtml(seriesItem.name) + "</div>",
        "<div>Min: <span style=\"font-weight:600;\">" + formatTooltipAxisValue(seriesItem.yAxisKey, stats.min) + "</span></div>",
        "<div>Q1: <span style=\"font-weight:600;\">" + formatTooltipAxisValue(seriesItem.yAxisKey, stats.q1) + "</span></div>",
        "<div>Median: <span style=\"font-weight:600;\">" + formatTooltipAxisValue(seriesItem.yAxisKey, stats.median) + "</span></div>",
        "<div>Q3: <span style=\"font-weight:600;\">" + formatTooltipAxisValue(seriesItem.yAxisKey, stats.q3) + "</span></div>",
        "<div>Max: <span style=\"font-weight:600;\">" + formatTooltipAxisValue(seriesItem.yAxisKey, stats.max) + "</span></div>",
      ].join("");
    }}
  }}
  const yValue = Array.isArray(pointValue) ? pointValue[pointValue.length - 1] : pointValue;
  const value = formatTooltipAxisValue(seriesItem.yAxisKey, yValue);
  return [
    "<div>" + header + "</div>",
    "<div style=\"display:flex;align-items:center;justify-content:space-between;gap:16px;\">"
      + "<span>" + renderTooltipMarker(seriesItem) + escapeTooltipHtml(seriesItem.name) + "</span>"
      + "<span style=\"margin-left:16px;font-weight:600;\">" + value + "</span>"
      + "</div>",
  ].join("");
}}

function findNearestVisibleDataPoint(seriesName, pixel) {{
  const seriesIndex = getSeriesIndexByName(seriesName);
  if (seriesIndex < 0) {{ return null; }}
  const seriesItem = seriesData[seriesIndex];
  const axisIndexes = getAxisIndexMaps();
  const xAxisIndex = axisIndexes.x[seriesItem.xAxisKey];
  const yAxisIndex = axisIndexes.y[seriesItem.yAxisKey];
  if (xAxisIndex === undefined || yAxisIndex === undefined) {{ return null; }}
  var nearestPoint = null;
  seriesItem.values.forEach(function(_, dataIndex) {{
    const pointValue = getSeriesPointValue(seriesItem, dataIndex);
    const pointPixel = chart.convertToPixel(
      {{ xAxisIndex: xAxisIndex, yAxisIndex: yAxisIndex }},
      pointValue,
    );
    if (!Array.isArray(pointPixel) || pointPixel.length < 2) {{ return; }}
    if (!Number.isFinite(pointPixel[0]) || !Number.isFinite(pointPixel[1])) {{ return; }}
    if (!chart.containPixel({{ gridIndex: 0 }}, pointPixel)) {{ return; }}
    const dx = pointPixel[0] - pixel[0];
    const dy = pointPixel[1] - pixel[1];
    const distanceSquared = dx * dx + dy * dy;
    if (!nearestPoint || distanceSquared < nearestPoint.distanceSquared) {{
      nearestPoint = {{
        seriesIndex: seriesItem.optionSeriesIndex,
        dataIndex: dataIndex,
        distanceSquared: distanceSquared,
      }};
    }}
  }});
  return nearestPoint;
}}

function syncHoveredSeriesTooltip(pixel) {{
  if (!hoveredSeriesName || !Array.isArray(pixel) || pixel.length < 2) {{
    clearHoveredTooltip();
    return;
  }}
  if (!chart.containPixel({{ gridIndex: 0 }}, pixel)) {{
    clearHoveredTooltip();
    return;
  }}
  const nearestPoint = findNearestVisibleDataPoint(hoveredSeriesName, pixel);
  if (!nearestPoint) {{
    clearHoveredTooltip();
    return;
  }}
  const nextTooltipKey = nearestPoint.seriesIndex + ":" + nearestPoint.dataIndex;
  if (hoveredTooltipKey === nextTooltipKey) {{ return; }}
  hoveredTooltipKey = nextTooltipKey;
  chart.dispatchAction({{
    type: "showTip",
    seriesIndex: nearestPoint.seriesIndex,
    dataIndex: nearestPoint.dataIndex,
  }});
}}

function showTooltipForSeriesEvent(event) {{
  if (!event || event.seriesIndex == null || event.dataIndex == null) {{
    return false;
  }}
  const nextTooltipKey = event.seriesIndex + ":" + event.dataIndex;
  if (hoveredTooltipKey === nextTooltipKey) {{
    return true;
  }}
  hoveredTooltipKey = nextTooltipKey;
  chart.dispatchAction({{
    type: "showTip",
    seriesIndex: event.seriesIndex,
    dataIndex: event.dataIndex,
  }});
  return true;
}}

function getEventPixel(event) {{
  if (!event) {{ return null; }}
  const candidates = [
    [event.offsetX, event.offsetY],
    [event.zrX, event.zrY],
    event.event ? [event.event.offsetX, event.event.offsetY] : null,
    event.event ? [event.event.zrX, event.event.zrY] : null,
  ];
  for (const candidate of candidates) {{
    if (
      Array.isArray(candidate)
      && Number.isFinite(candidate[0])
      && Number.isFinite(candidate[1])
    ) {{
      return candidate;
    }}
  }}
  const sourceEvent = event.event || event;
  if (Number.isFinite(sourceEvent.clientX) && Number.isFinite(sourceEvent.clientY)) {{
    const rect = chartNode.getBoundingClientRect();
    return [sourceEvent.clientX - rect.left, sourceEvent.clientY - rect.top];
  }}
  return null;
}}

function handleAxisToggleEvent(event) {{
  const pixel = getEventPixel(event);
  const axisHitArea = findAxisHitArea(pixel);
  if (!axisHitArea) {{ return false; }}
  const eventTime = event && Number.isFinite(event.timeStamp) ? event.timeStamp : 0;
  if (eventTime && lastAxisToggleEventTime && eventTime === lastAxisToggleEventTime) {{
    return true;
  }}
  lastAxisToggleEventTime = eventTime;
  toggleAxisLogScale(axisHitArea.id);
  if (event && typeof event.preventDefault === "function") {{
    event.preventDefault();
  }}
  if (event && typeof event.stopPropagation === "function") {{
    event.stopPropagation();
  }}
  return true;
}}

function renderChart() {{
  const width = chartNode.clientWidth;
  const height = chartNode.clientHeight;
  const zoomState = readCurrentDataZoomState();
  const option = buildOption(width, height, hoveredSeriesName);
  setInteractiveCursor("default");
  chart.setOption(applyDataZoomState(option, zoomState), true);
  updateCustomTickGraphics();
}}

function setHoveredSeries(seriesName) {{
  clearHoverResetTimer();
  if (!seriesName || !seriesVisibility[seriesName]) {{
    if (!hoveredSeriesName) {{ return; }}
    hoveredSeriesName = null;
    clearHoveredTooltip();
    renderChart();
    return;
  }}
  if (hoveredSeriesName === seriesName) {{ return; }}
  hoveredSeriesName = seriesName;
  clearHoveredTooltip();
  renderChart();
}}

function scheduleHoveredSeriesReset() {{
  clearHoverResetTimer();
  hoverResetTimer = window.setTimeout(function() {{
    hoverResetTimer = null;
    setHoveredSeries(null);
  }}, 0);
}}

function syncLegendHover(pixel) {{
  if (findLegendHitArea(pixel)) {{
    setInteractiveCursor("pointer");
    return true;
  }}
  return false;
}}

function applySeriesVisibilityChange() {{
  if (hoveredSeriesName && !seriesVisibility[hoveredSeriesName]) {{
    hoveredSeriesName = null;
    clearHoveredTooltip();
  }}
  renderChart();
}}

function toggleLegendSeries(seriesName) {{
  if (!(seriesName in seriesVisibility)) {{ return; }}
  seriesVisibility[seriesName] = !seriesVisibility[seriesName];
  applySeriesVisibilityChange();
}}

function toggleOtherLegendSeries(seriesName) {{
  const otherSeriesNames = seriesData
    .map(function(seriesItem) {{ return seriesItem.name; }})
    .filter(function(name) {{ return name !== seriesName; }});
  if (otherSeriesNames.length === 0) {{ return; }}
  const shouldShowOthers = otherSeriesNames.every(function(name) {{ return !seriesVisibility[name]; }});
  otherSeriesNames.forEach(function(name) {{
    seriesVisibility[name] = shouldShowOthers;
  }});
  applySeriesVisibilityChange();
}}

renderChart();

chart.on("mouseover", {{ componentType: "series" }}, function(event) {{
  hoveredRenderedSeriesName = event.seriesName || null;
  if (shouldUseDirectItemTooltip(event.seriesName || null)) {{
    clearHoverResetTimer();
    return;
  }}
  setHoveredSeries(getBaseSeriesName(event.seriesName || null));
  if (!showTooltipForSeriesEvent(event)) {{
    syncHoveredSeriesTooltip(getEventPixel(event));
  }}
}});

chart.on("mousemove", {{ componentType: "series" }}, function(event) {{
  hoveredRenderedSeriesName = event.seriesName || null;
  if (shouldUseDirectItemTooltip(event.seriesName || null)) {{
    clearHoverResetTimer();
    return;
  }}
  setHoveredSeries(getBaseSeriesName(event.seriesName || null));
  if (!showTooltipForSeriesEvent(event)) {{
    syncHoveredSeriesTooltip(getEventPixel(event));
  }}
}});

chart.on("mouseout", {{ componentType: "series" }}, function() {{
  hoveredRenderedSeriesName = null;
  scheduleHoveredSeriesReset();
}});

chart.on("globalout", function() {{
  hoveredRenderedSeriesName = null;
  setInteractiveCursor("default");
  scheduleHoveredSeriesReset();
}});

chart.on("dataZoom", function() {{
  updateCustomTickGraphics();
}});

chart.getZr().on("mousemove", function(event) {{
  const pixel = getEventPixel(event);
  const axisHitArea = findAxisHitArea(pixel);
  if (axisHitArea) {{
    setInteractiveCursor("pointer");
    if (hoveredSeriesName) {{
      setHoveredSeries(null);
    }}
    return;
  }}
  if (syncLegendHover(pixel)) {{ return; }}
  setInteractiveCursor("default");
  if (!hoveredSeriesName) {{ return; }}
  if (!event.target) {{
    hoveredRenderedSeriesName = null;
    setHoveredSeries(null);
    return;
  }}
  if (shouldUseDirectItemTooltip(hoveredRenderedSeriesName)) {{
    return;
  }}
  syncHoveredSeriesTooltip(pixel);
}});

chart.getZr().on("click", function(event) {{
  const legendHitArea = findLegendHitArea(getEventPixel(event));
  if (legendHitArea) {{
    clearLegendClickTimer();
    legendClickTimer = window.setTimeout(function() {{
      legendClickTimer = null;
      toggleLegendSeries(legendHitArea.name);
    }}, 250);
    return;
  }}
  handleAxisToggleEvent(event);
}});

chart.getZr().on("dblclick", function(event) {{
  const pixel = getEventPixel(event);
  const legendHitArea = findLegendHitArea(pixel);
  if (legendHitArea) {{
    clearLegendClickTimer();
    toggleOtherLegendSeries(legendHitArea.name);
    return;
  }}
  if (!chart.containPixel({{ gridIndex: 0 }}, pixel)) {{ return; }}
  resetDataZoom();
}});

function handleResize() {{
  chart.resize();
  setInteractiveCursor("default");
  clearLegendClickTimer();
  renderChart();
}}

var resizeObserver = null;
var usingWindowResizeFallback = false;
if (typeof ResizeObserver === "function") {{
  resizeObserver = new ResizeObserver(function() {{
    handleResize();
  }});
  resizeObserver.observe(chartNode);
}} else {{
  usingWindowResizeFallback = true;
  window.addEventListener("resize", handleResize);
}}

console.info("msp echarts points:", {{ original: {total_original_points}, embedded: {total_embedded_points}, maxPerSeries: {max_points} }});
window.addEventListener("pagehide", function() {{
  setInteractiveCursor("default");
  clearHoverResetTimer();
  clearLegendClickTimer();
  if (resizeObserver) {{
    resizeObserver.disconnect();
  }}
  if (usingWindowResizeFallback) {{
    window.removeEventListener("resize", handleResize);
  }}
  chart.dispose();
}}, {{ once: true }});
}})();
</script>
"##,
        labels_js = labels_js,
        axis_labels_js = axis_labels_js,
        series_data_js = series_data_js,
        cells_js = cells_js,
        axis_ranges_js = axis_ranges_js,
        axis_ticks_js = axis_ticks_js,
        theme = theme,
        font_family = font_family,
        font_size = font_size,
        legend_font_family = legend_font_family,
        legend_font_size = legend_font_size,
        total_original_points = total_original_points,
        total_embedded_points = total_embedded_points,
        max_points = max_points,
        show_grid = if request.plot.grid { "true" } else { "false" },
        chart_width_px = chart_width_px,
        chart_height_px = chart_height_px,
    ))
}

fn build_runtime_script_tag() -> String {
    format!(r#"<script src="{ECHARTS_CDN_URL}"></script>"#)
}

fn build_page_html(snippet: &str, options: &EchartsBackendOptions) -> String {
    let runtime_tag = if options.runtime_mode == EchartsRuntimeMode::Cdn {
        build_runtime_script_tag()
    } else {
        String::new()
    };
    format!(
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
  --shadow: 0 18px 40px rgba(23, 32, 51, 0.08);
}}
body {{
  margin: 0;
  padding: 32px;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  background: linear-gradient(180deg, #f8faff 0%, #f2f5fb 100%);
  color: var(--text);
}}
.page {{
  width: fit-content;
  max-width: 100%;
  margin: 0 auto;
}}
.panel {{
  position: relative;
  background: var(--panel);
  border: 1px solid rgba(95, 107, 133, 0.12);
  border-radius: 20px;
  box-shadow: var(--shadow);
  padding: 28px;
}}
@media (max-width: 640px) {{
  body {{ padding: 16px; }}
  .panel {{ padding: 20px; border-radius: 16px; }}
}}
</style>
</head>
<body>
<main class="page">
<section class="panel">
{runtime_tag}
{snippet}
</section>
</main>
</body>
</html>"##
    )
}

fn build_embed_fragment(
    snippet: &str,
    options: &EchartsBackendOptions,
) -> String {
    if options.runtime_mode == EchartsRuntimeMode::Cdn {
        format!("{}\n{}", build_runtime_script_tag(), snippet)
    } else {
        snippet.to_string()
    }
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
        let snippet = build_chart_snippet(request, prepared, options)?;
        let (description, payload) = match options.output_mode {
            EchartsOutputMode::Page => {
                ("ECharts HTML page", build_page_html(&snippet, options))
            }
            EchartsOutputMode::Embed => (
                "ECharts HTML fragment",
                build_embed_fragment(&snippet, options),
            ),
        };
        let out_path = request.render_target.out.clone().unwrap_or_else(|| {
            request.render_target.work_dir.join("msp-echarts.html")
        });
        Ok(RenderPlan {
            description: format!("{description} -> {}", out_path.display()),
            payload,
            artifact_path: Some(out_path),
        })
    }

    fn execute(
        &self,
        plan: &RenderPlan,
        request: &ResolvedMspRequest,
    ) -> anyhow::Result<()> {
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
            AxisRef, AxisScale, AxisSpec, AxisValueFormat, BackendKind,
            BackendOptions, DataPrepSpec, EchartsBackendOptions,
            EchartsOutputMode, EchartsRuntimeMode, ExecutionMode, LayoutSpec,
            LegendSpec, PlotAxes, PlotSpec, PreparedSeries, RenderTarget,
            ResolvedMspRequest, SeriesAxisBinding, SeriesMark, SeriesSpec,
            SeriesStyle, StandardTickSpec, ThemeSpec, TickSpec, TimestampUnit,
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
        let mut axes = PlotAxes::default();
        axes.insert(
            AxisRef::x(1),
            AxisSpec {
                scale: AxisScale::Linear,
                number_format: AxisValueFormat::Scientific { decimals: None },
                range: None,
                label: Some("Sample X".to_string()),
                ticks: TickSpec {
                    major: None,
                    custom: Vec::new(),
                },
            },
        );
        axes.insert(
            AxisRef::y(1),
            AxisSpec {
                scale: AxisScale::Linear,
                number_format: AxisValueFormat::Suffix { decimals: None },
                range: Some(0.0..100.0),
                label: Some("Primary Y".to_string()),
                ticks: TickSpec {
                    major: None,
                    custom: Vec::new(),
                },
            },
        );
        axes.insert(AxisRef::x(2), AxisSpec::default());
        axes.insert(
            AxisRef::y(2),
            AxisSpec {
                scale: AxisScale::Log10,
                number_format: AxisValueFormat::Plain { decimals: None },
                range: Some(1.0..1000.0),
                label: Some("Secondary Y".to_string()),
                ticks: TickSpec {
                    major: None,
                    custom: Vec::new(),
                },
            },
        );
        ResolvedMspRequest {
            mode: ExecutionMode::Plot,
            backend: BackendKind::Echarts,
            data_prep: DataPrepSpec {
                inputs: Vec::new(),
                series: Vec::new(),
            },
            plot: PlotSpec {
                title: None,
                layout: LayoutSpec {
                    width: 1.0,
                    height: 1.0,
                },
                theme: ThemeSpec { font: None },
                legend: LegendSpec {
                    position: "top right".to_string(),
                    font: None,
                },
                axes,
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
                output_mode: EchartsOutputMode::Page,
                runtime_mode: EchartsRuntimeMode::Cdn,
            }),
        }
    }

    #[test]
    fn build_render_plan_embeds_generated_csv_data() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,y\n1,10\n2,20\n3,30\n");

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 1),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "y".to_string(),
                mark: SeriesMark::Lines,
                boxplot_group: None,
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
        assert!(plan.description.contains("ECharts HTML page"));
        assert!(plan.payload.contains("<!DOCTYPE html>"));
        assert!(plan.payload.contains("echarts.min.js"));
        assert!(plan.payload.contains("body {"));
        assert!(plan.payload.contains("Throughput"));
        assert!(plan.payload.contains("[1,10]"));
        assert!(plan.payload.contains("[2,20]"));
        assert!(plan.payload.contains(
            "class=\"msp-echarts-root\" style=\"width:800px;max-width:100%;\""
        ));
        assert!(plan.payload.contains(
            ".msp-echarts-canvas {\n  width: 100%;\n  height: 800px;"
        ));
        assert!(plan.payload.contains("Primary Y"));
        assert!(plan.payload.contains("xAxisKey:\"x-bottom\""));
        assert!(plan.payload.contains("yAxisKey:\"y-left-1\""));
        assert!(plan.payload.contains("labelMargin:10,visibilityPolicy"));
        assert!(plan.payload.contains("nameLocation: \"middle\""));
        assert!(plan.payload.contains("function computeCrossLayout"));
        assert!(plan.payload.contains("function collectSeriesAxisValues"));
        assert!(plan.payload.contains("function formatAxisValue"));
        assert!(plan.payload.contains("function buildAxisTitleGraphic"));
        assert!(plan.payload.contains("function updateAxisHitAreas"));
        assert!(!plan.payload.contains("}};\n\\\n  if (legendCell)"));
        assert!(plan.payload.contains("function updateLegendHitAreas"));
        assert!(plan.payload.contains("function toggleAxisLogScale"));
        assert!(plan.payload.contains("function readCurrentDataZoomState"));
        assert!(plan.payload.contains("function resetDataZoom"));
        assert!(plan.payload.contains("function toggleLegendSeries"));
        assert!(plan.payload.contains("function toggleOtherLegendSeries"));
        assert!(
            plan.payload
                .contains("function findNearestVisibleDataPoint")
        );
        assert!(
            plan.payload
                .contains("function formatHoveredSeriesTooltip(params)")
        );
        assert!(plan.payload.contains("function buildTrackMap"));
        assert!(plan.payload.contains("function resolveMeasuredSpec"));
        assert!(plan.payload.contains("scaleType:\"value\""));
        assert!(
            plan.payload.contains(
                "numberFormat:{mode:\"scientific\",decimals:undefined}"
            )
        );
        assert!(
            plan.payload
                .contains("numberFormat:{mode:\"suffix\",decimals:undefined}")
        );
        assert!(plan.payload.contains("value <= 0"));
        assert!(plan.payload.contains("function formatScientificValue"));
        assert!(plan.payload.contains("function formatSuffixValue"));
        assert!(plan.payload.contains("function formatPercentageValue"));
        assert!(plan.payload.contains("function formatTimestampValue"));
        assert!(plan.payload.contains(
            "formatter: function(value) { return formatAxisLabel(cell.id, value, isLogScale, cell.numberFormat); }"
        ));
        assert!(plan.payload.contains("return formatAxisValue("));
        assert!(plan.payload.contains("nameTextStyle: { color: \"#172033\""));
        assert!(
            plan.payload
                .contains("const titleBorderX = cell.side === \"left\"")
        );
        assert!(plan.payload.contains("chart.getZr().on(\"click\""));
        assert!(plan.payload.contains("chart.getZr().on(\"dblclick\""));
        assert!(
            !plan
                .payload
                .contains("chartNode.addEventListener(\"click\"")
        );
        assert!(
            plan.payload
                .contains("id: \"x1-wheel-zoom\", type: \"inside\"")
        );
        assert!(plan.payload.contains("selectedMode: false"));
        assert!(plan.payload.contains("name: cell.name || \"\""));
        assert!(plan.payload.contains("name: \"\""));
        assert!(plan.payload.contains("layoutSpec"));
        assert!(plan.payload.contains("seriesData"));
        assert!(plan.payload.contains(
            "formatter: function(params) { return formatHoveredSeriesTooltip(params); }"
        ));
        assert!(
            plan.payload
                .contains("const script = document.currentScript;")
        );
        assert!(
            plan.payload
                .contains("const root = script.previousElementSibling;")
        );
        assert!(plan.payload.contains("new ResizeObserver"));
        assert!(!plan.payload.contains("id:\"title\",kind:\"title\""));
        assert!(!plan.payload.contains("option.title = {"));
        assert!(
            !plan
                .payload
                .contains("const isDimmed = activeSeriesName && !isHovered;")
        );
        assert!(
            !plan
                .payload
                .contains("const opacity = isDimmed ? 0.14 : 1;")
        );
        assert!(plan.payload.contains("\"x-bottom\""));
        assert!(plan.payload.contains("\"y-left-1\""));
    }

    #[test]
    fn build_render_plan_supports_embed_mode_with_external_runtime() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,y\n1,10\n2,20\n");

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 1),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "y".to_string(),
                mark: SeriesMark::Lines,
                boxplot_group: None,
                name: Some("Throughput".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path,
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.embed.html");
        let mut request = test_request(&work_dir, &out_path);
        request.backend_options =
            BackendOptions::Echarts(EchartsBackendOptions {
                theme: Some("light".to_string()),
                max_points: None,
                output_mode: EchartsOutputMode::Embed,
                runtime_mode: EchartsRuntimeMode::External,
            });

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.description.contains("ECharts HTML fragment"));
        assert!(!plan.payload.contains("<!DOCTYPE html>"));
        assert!(!plan.payload.contains("<body>"));
        assert!(!plan.payload.contains("body {"));
        assert!(!plan.payload.contains("id=\"chart\""));
        assert!(!plan.payload.contains("id=\"axis-toggle-tooltip\""));
        assert!(!plan.payload.contains("echarts.min.js"));
        assert!(!plan.payload.contains("msp-echarts-axis-toggle-tooltip"));
        assert!(plan.payload.contains("class=\"msp-echarts-root\""));
        assert!(plan.payload.contains("class=\"msp-echarts-canvas\""));
        assert!(plan.payload.contains("document.currentScript"));
        assert!(plan.payload.contains("script.previousElementSibling"));
        assert!(plan.payload.contains("load ECharts before this fragment"));
        assert!(
            plan.payload
                .contains("const axisHitArea = findAxisHitArea(pixel);")
        );
        assert!(plan.payload.contains("toggleAxisLogScale(axisHitArea.id);"));
    }

    #[test]
    fn build_render_plan_supports_category_x_and_secondary_axis() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "label,value\nalpha,5\nbeta,8\n");

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 2),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "label".to_string(),
                y_expr: "value".to_string(),
                mark: SeriesMark::Points,
                boxplot_group: None,
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
    fn build_render_plan_supports_bar_series_with_category_overlay() {
        let work_dir = unique_test_dir();
        let bar_csv_path = work_dir.join("bar.csv");
        let line_csv_path = work_dir.join("line.csv");
        write_csv(&bar_csv_path, "year,value\n2023,10\n2024,20\n");
        write_csv(&line_csv_path, "year,value\n2023,12\n2024,18\n");

        let prepared = vec![
            PreparedSeries {
                index: 0,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "year".to_string(),
                    y_expr: "value".to_string(),
                    mark: SeriesMark::Bar,
                    boxplot_group: None,
                    name: Some("Revenue".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: bar_csv_path,
                log_path: work_dir.join("bar.log"),
            },
            PreparedSeries {
                index: 1,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "year".to_string(),
                    y_expr: "value".to_string(),
                    mark: SeriesMark::LinesPoints,
                    boxplot_group: None,
                    name: Some("Target".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: line_csv_path,
                log_path: work_dir.join("line.log"),
            },
        ];
        let out_path = work_dir.join("chart.html");
        let request = test_request(&work_dir, &out_path);

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains("\"category\""));
        assert!(plan.payload.contains("type:\"bar\""));
        assert!(plan.payload.contains("type:\"line\""));
        assert!(plan.payload.contains("barMaxWidth:48"));
        assert!(plan.payload.contains("[\"2023\",10]"));
        assert!(plan.payload.contains("[\"2024\",18]"));
    }

    #[test]
    fn build_render_plan_aligns_category_lines_with_boxplot_labels() {
        let work_dir = unique_test_dir();
        let line_csv_path = work_dir.join("line.csv");
        let boxplot_csv_path = work_dir.join("boxplot.csv");
        write_csv(&line_csv_path, "label,value\nbeta,8\nalpha,5\n");
        write_csv(
            &boxplot_csv_path,
            "label,value\nalpha,10\nalpha,20\nbeta,5\nbeta,15\n",
        );

        let prepared = vec![
            PreparedSeries {
                index: 0,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "label".to_string(),
                    y_expr: "value".to_string(),
                    mark: SeriesMark::LinesPoints,
                    boxplot_group: None,
                    name: Some("Mean".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: line_csv_path,
                log_path: work_dir.join("line.log"),
            },
            PreparedSeries {
                index: 1,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "label".to_string(),
                    y_expr: "value".to_string(),
                    mark: SeriesMark::Boxplot,
                    boxplot_group: None,
                    name: Some("Distribution".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: boxplot_csv_path,
                log_path: work_dir.join("boxplot.log"),
            },
        ];
        let out_path = work_dir.join("chart.html");
        let request = test_request(&work_dir, &out_path);

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains("const labels = [\"alpha\",\"beta\"]"));
        assert!(plan.payload.contains("name:\"Mean\""));
        assert!(plan.payload.contains("values:[[\"alpha\",5],[\"beta\",8]]"));
        assert!(plan.payload.contains("datasetIndex:0"));
        assert!(plan.payload.contains(
            "encode:{x:\"x\",y:\"y\",itemName:\"x\",tooltip:[\"y\"]}"
        ));
        assert!(plan.payload.contains("name:\"Distribution\""));
        assert!(plan.payload.contains("datasetIndex:2"));
    }

    #[test]
    fn build_render_plan_supports_transform_based_boxplots() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("boxplot.csv");
        write_csv(
            &csv_path,
            "metric,value\nalpha,10\nalpha,20\nalpha,30\nbeta,5\nbeta,15\nbeta,25\n",
        );

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 1),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "metric".to_string(),
                y_expr: "value".to_string(),
                mark: SeriesMark::Boxplot,
                boxplot_group: None,
                name: Some("Latency".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path,
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let request = test_request(&work_dir, &out_path);

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains("type:\"boxplot\""));
        assert!(
            plan.payload
                .contains("groupedValues:[[10,20,30],[5,15,25]]")
        );
        assert!(plan.payload.contains("rawDatasetIndex:0"));
        assert!(plan.payload.contains("datasetIndex:1"));
        assert!(plan.payload.contains("outlierDatasetIndex:2"));
        assert!(
            plan.payload
                .contains("fromDatasetIndex:0,transform:{type:\"boxplot\"")
        );
        assert!(
            plan.payload
                .contains("fromDatasetIndex:1,fromTransformResult:1")
        );
        assert!(plan.payload.contains(
            "const axisCategoryLabels = {\"x-bottom\":[\"alpha\",\"beta\"]}"
        ));
        assert!(plan.payload.contains("dataset: datasets"));
        assert!(
            plan.payload
                .contains("seriesData.flatMap(function(seriesItem) {")
        );
        assert!(plan.payload.contains(
            "data: seriesItem.outlierValues !== undefined ? seriesItem.outlierValues : undefined,"
        ));
        assert!(
            plan.payload
                .contains("datasetIndex: seriesItem.outlierDatasetIndex,")
        );
        assert!(plan.payload.contains("type: \"custom\","));
        assert!(
            plan.payload
                .contains("cx: point[0] + getBoxplotOffsetPixels(seriesItem),")
        );
        assert!(
            plan.payload
                .contains("function getBoxplotOffsetPixels(seriesItem) {")
        );
        assert!(
            plan.payload
                .contains("function visitSeriesAxisValues(seriesItem, axisDimension, visitor) {")
        );
        assert!(
            !plan
                .payload
                .contains("Math.max.apply(null, Array.from(candidateLabels)")
        );
        assert!(
            plan.payload
                .contains("Median: <span style=\\\"font-weight:600;\\\">")
        );
        assert!(
            plan.payload
                .contains("const OUTLIER_SERIES_ID_SUFFIX = \"__outliers\";")
        );
        assert!(
            plan.payload
                .contains("function isOutlierSeriesEvent(params) {")
        );
        assert!(
            plan.payload
                .contains("function shouldUseDirectItemTooltip(seriesName) {")
        );
        assert!(
            plan.payload
                .contains("function showTooltipForSeriesEvent(event) {")
        );
        assert!(
            plan.payload
                .contains("function renderTooltipMarker(seriesItem) {")
        );
        assert!(
            plan.payload
                .contains("function getBoxplotTooltipStats(boxValues) {")
        );
        assert!(plan.payload.contains("min: boxValues[1],"));
        assert!(plan.payload.contains(
            "escapeTooltipHtml(seriesItem.name) + \" outlier</span>\""
        ));
        assert!(plan.payload.contains(
            "renderTooltipMarker(seriesItem) + escapeTooltipHtml(seriesItem.name)"
        ));
        assert!(plan.payload.contains(
            "function buildBoxplotOutlierSeries(seriesItem, axisIndexes, isHovered) {"
        ));
        assert!(
            plan.payload
                .contains("id: seriesItem.name + OUTLIER_SERIES_ID_SUFFIX,")
        );
        assert!(plan.payload.contains("name: seriesItem.name,"));
        assert!(
            plan.payload
                .contains("function buildItemTooltipConfig(seriesItem) {")
        );
        assert!(plan.payload.contains("borderColor: seriesItem.color,"));
        assert!(plan.payload.contains(
            "formatter: function(params) { return formatHoveredSeriesTooltip(params); }"
        ));
        assert!(
            !plan
                .payload
                .contains("encode: seriesItem.type === \"boxplot\"")
        );
    }

    #[test]
    fn build_render_plan_merges_numbered_boxplot_groups() {
        let work_dir = unique_test_dir();
        let first_csv_path = work_dir.join("boxplot-a.csv");
        let second_csv_path = work_dir.join("boxplot-b.csv");
        write_csv(
            &first_csv_path,
            "metric,value\nalpha,10\nalpha,20\nbeta,5\n",
        );
        write_csv(
            &second_csv_path,
            "metric,value\nalpha,30\nbeta,15\nbeta,25\n",
        );

        let prepared = vec![
            PreparedSeries {
                index: 0,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "metric".to_string(),
                    y_expr: "value".to_string(),
                    mark: SeriesMark::Boxplot,
                    boxplot_group: Some(1),
                    name: Some("Latency".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: first_csv_path,
                log_path: work_dir.join("series-a.log"),
            },
            PreparedSeries {
                index: 1,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "metric".to_string(),
                    y_expr: "value".to_string(),
                    mark: SeriesMark::Boxplot,
                    boxplot_group: Some(1),
                    name: Some("Latency".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: second_csv_path,
                log_path: work_dir.join("series-b.log"),
            },
        ];
        let out_path = work_dir.join("chart.html");
        let request = test_request(&work_dir, &out_path);

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(
            plan.payload
                .contains("groupedValues:[[10,20,30],[5,15,25]]")
        );
        assert!(plan.payload.contains("datasetIndex:1"));
        assert!(plan.payload.contains("outlierDatasetIndex:2"));
        assert!(plan.payload.contains("name:\"Latency\""));
        assert!(!plan.payload.contains("name:\"series 2\""));
    }

    #[test]
    fn build_render_plan_downsamples_merged_boxplot_groups() {
        let work_dir = unique_test_dir();
        let first_csv_path = work_dir.join("boxplot-a.csv");
        let second_csv_path = work_dir.join("boxplot-b.csv");
        let mut first_csv = String::from("metric,value\n");
        let mut second_csv = String::from("metric,value\n");
        for value in 0..120 {
            first_csv.push_str(&format!("alpha,{value}\n"));
        }
        for value in 120..240 {
            second_csv.push_str(&format!("beta,{value}\n"));
        }
        write_csv(&first_csv_path, &first_csv);
        write_csv(&second_csv_path, &second_csv);

        let prepared = vec![
            PreparedSeries {
                index: 0,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "metric".to_string(),
                    y_expr: "value".to_string(),
                    mark: SeriesMark::Boxplot,
                    boxplot_group: Some(1),
                    name: Some("Latency".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: first_csv_path,
                log_path: work_dir.join("series-a.log"),
            },
            PreparedSeries {
                index: 1,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "metric".to_string(),
                    y_expr: "value".to_string(),
                    mark: SeriesMark::Boxplot,
                    boxplot_group: Some(1),
                    name: Some("Latency".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: second_csv_path,
                log_path: work_dir.join("series-b.log"),
            },
        ];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.backend_options =
            BackendOptions::Echarts(EchartsBackendOptions {
                theme: Some("light".to_string()),
                max_points: Some(10),
                output_mode: EchartsOutputMode::Page,
                runtime_mode: EchartsRuntimeMode::Cdn,
            });

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains(
            "console.info(\"msp echarts points:\", { original: 240, embedded: 10, maxPerSeries: 10 });"
        ));
        assert!(plan.payload.contains(
            "groupedValues:[[0,26,53,79,106],[132,159,185,212,239]]"
        ));
        assert!(
            plan.payload
                .contains("fromDatasetIndex:0,transform:{type:\"boxplot\"")
        );
        assert!(plan.payload.contains("dataset: datasets"));
    }

    #[test]
    fn build_render_plan_rejects_mixed_numeric_and_category_x_on_same_axis() {
        let work_dir = unique_test_dir();
        let numeric_csv_path = work_dir.join("numeric.csv");
        let category_csv_path = work_dir.join("category.csv");
        write_csv(&numeric_csv_path, "x,y\n1,10\n2,20\n");
        write_csv(&category_csv_path, "label,y\nalpha,5\nbeta,8\n");

        let prepared = vec![
            PreparedSeries {
                index: 0,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "x".to_string(),
                    y_expr: "y".to_string(),
                    mark: SeriesMark::Lines,
                    boxplot_group: None,
                    name: Some("Numeric".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: numeric_csv_path,
                log_path: work_dir.join("numeric.log"),
            },
            PreparedSeries {
                index: 1,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "label".to_string(),
                    y_expr: "y".to_string(),
                    mark: SeriesMark::Points,
                    boxplot_group: None,
                    name: Some("Category".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: category_csv_path,
                log_path: work_dir.join("category.log"),
            },
        ];
        let out_path = work_dir.join("chart.html");
        let request = test_request(&work_dir, &out_path);

        let err = match EchartsBackend.build_render_plan(&request, &prepared) {
            Ok(_) => panic!("expected mixed numeric/category x axis error"),
            Err(err) => err.to_string(),
        };

        assert!(err.contains("x axis x1 mixes numeric and category series"));
        assert!(err.contains("Numeric"));
        assert!(err.contains("Category"));
    }

    #[test]
    fn build_render_plan_rejects_mismatched_category_sets_on_same_axis() {
        let work_dir = unique_test_dir();
        let first_csv_path = work_dir.join("first.csv");
        let second_csv_path = work_dir.join("second.csv");
        write_csv(&first_csv_path, "label,y\nalpha,10\nbeta,20\n");
        write_csv(&second_csv_path, "label,y\nalpha,8\ngamma,9\n");

        let prepared = vec![
            PreparedSeries {
                index: 0,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "label".to_string(),
                    y_expr: "y".to_string(),
                    mark: SeriesMark::Lines,
                    boxplot_group: None,
                    name: Some("First".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: first_csv_path,
                log_path: work_dir.join("first.log"),
            },
            PreparedSeries {
                index: 1,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "label".to_string(),
                    y_expr: "y".to_string(),
                    mark: SeriesMark::Points,
                    boxplot_group: None,
                    name: Some("Second".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: second_csv_path,
                log_path: work_dir.join("second.log"),
            },
        ];
        let out_path = work_dir.join("chart.html");
        let request = test_request(&work_dir, &out_path);

        let err = match EchartsBackend.build_render_plan(&request, &prepared) {
            Ok(_) => panic!("expected mismatched category set error"),
            Err(err) => err.to_string(),
        };

        assert!(err.contains("x axis x1 has incompatible category sets"));
        assert!(err.contains("alpha, beta"));
        assert!(err.contains("alpha, gamma"));
    }

    #[test]
    fn build_render_plan_supports_third_y_axis() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,value\n1,5\n2,8\n");

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 3),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "value".to_string(),
                mark: SeriesMark::Lines,
                boxplot_group: None,
                name: Some("Jitter".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path.clone(),
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.plot.axes.insert(
            AxisRef::y(3),
            AxisSpec {
                scale: AxisScale::Log10,
                number_format: AxisValueFormat::Plain { decimals: None },
                range: Some(1.0..10.0),
                label: Some("Tertiary Y".to_string()),
                ticks: TickSpec {
                    major: None,
                    custom: Vec::new(),
                },
            },
        );

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains("yAxisKey:\"y-right-2\""));
        assert!(plan.payload.contains("id:\"y-right-2\""));
        assert!(plan.payload.contains("Tertiary Y"));
        assert!(plan.payload.contains("scaleType:\"log\""));
        assert!(plan.payload.contains("\"y-right-2\":{min:1,max:10}"));
    }

    #[test]
    fn build_render_plan_initializes_logscale_state_from_axis_specs() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,value\n1,5\n10,8\n");

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 2),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "value".to_string(),
                mark: SeriesMark::Lines,
                boxplot_group: None,
                name: Some("Latency".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path.clone(),
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.plot.axes.insert(
            AxisRef::x(1),
            AxisSpec {
                scale: AxisScale::Log10,
                number_format: AxisValueFormat::Scientific { decimals: None },
                range: Some(1.0..100.0),
                label: Some("Sample X".to_string()),
                ticks: TickSpec {
                    major: None,
                    custom: Vec::new(),
                },
            },
        );

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains(
            "axisDimension:\"x\",scaleType:\"log\",numberFormat:{mode:\"scientific\",decimals:undefined},name:\"Sample X\""
        ));
        assert!(plan.payload.contains(
            "axisDimension:\"y\",scaleType:\"log\",numberFormat:{mode:\"plain\",decimals:undefined},name:\"Secondary Y\""
        ));
        assert!(plan.payload.contains(
            "toggleableAxisCells.map(function(cell) { return [cell.id, cell.scaleType === \"log\"]; })"
        ));
    }

    #[test]
    fn build_render_plan_honors_explicit_log_axis_ranges() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,value\n2,4\n8,32\n");

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 2),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "value".to_string(),
                mark: SeriesMark::Lines,
                boxplot_group: None,
                name: Some("Latency".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path,
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.plot.axes.insert(
            AxisRef::x(1),
            AxisSpec {
                scale: AxisScale::Log10,
                number_format: AxisValueFormat::Scientific { decimals: None },
                range: Some(2.0..32.0),
                label: Some("Sample X".to_string()),
                ticks: TickSpec {
                    major: None,
                    custom: Vec::new(),
                },
            },
        );
        request.plot.axes.insert(
            AxisRef::y(2),
            AxisSpec {
                scale: AxisScale::Log10,
                number_format: AxisValueFormat::Plain { decimals: None },
                range: Some(4.0..64.0),
                label: Some("Secondary Y".to_string()),
                ticks: TickSpec {
                    major: None,
                    custom: Vec::new(),
                },
            },
        );

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains("\"x-bottom\":{min:2,max:32}"));
        assert!(plan.payload.contains("\"y-right-1\":{min:4,max:64}"));
    }

    #[test]
    fn build_render_plan_emits_standard_ticks_for_numeric_axes() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,y\n0,10\n10,20\n");

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 1),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "y".to_string(),
                mark: SeriesMark::Lines,
                boxplot_group: None,
                name: Some("Throughput".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path,
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.plot.axes.insert(
            AxisRef::x(1),
            AxisSpec {
                scale: AxisScale::Linear,
                number_format: AxisValueFormat::Scientific { decimals: None },
                range: None,
                label: Some("Sample X".to_string()),
                ticks: TickSpec {
                    major: Some(StandardTickSpec {
                        range: Some(0.0..10.0),
                        step: 2.0,
                    }),
                    custom: Vec::new(),
                },
            },
        );

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(
            plan.payload
                .contains("\"x-bottom\":{interval:2,min:0,max:10,custom:[]}")
        );
    }

    #[test]
    fn build_render_plan_emits_custom_ticks_for_numeric_axes() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,y\n1,5\n2,9\n");

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 1),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "y".to_string(),
                mark: SeriesMark::Lines,
                boxplot_group: None,
                name: Some("Latency".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path,
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.plot.axes.insert(
            AxisRef::y(1),
            AxisSpec {
                scale: AxisScale::Linear,
                number_format: AxisValueFormat::Suffix { decimals: None },
                range: Some(0.0..10.0),
                label: Some("Primary Y".to_string()),
                ticks: TickSpec {
                    major: None,
                    custom: vec![
                        (5.0, "P50".to_string()),
                        (9.0, "P90".to_string()),
                    ],
                },
            },
        );

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains(
            "\"y-left-1\":{interval:undefined,min:0,max:10,custom:[[5,\"P50\"],[9,\"P90\"]]}"
        ));
        assert!(plan.payload.contains("function findCustomTickLabel"));
        assert!(plan.payload.contains("function hasCustomTicks(axisId)"));
        assert!(plan.payload.contains(
            "function getAxisTickInterval(tickConfig, axisId, isLogScale)"
        ));
        assert!(plan.payload.contains("function resolveAxisMin(range, tickConfig, axisId, axisDimension, isLogScale)"));
        assert!(plan.payload.contains("function resolveAxisMax(range, tickConfig, axisId, axisDimension, isLogScale, minValue)"));
        assert!(plan.payload.contains("function buildCustomTickGraphics()"));
        assert!(plan.payload.contains("function updateCustomTickGraphics()"));
        assert!(plan.payload.contains(
            "return formatAxisLabel(cell.id, value, isLogScale, cell.numberFormat);"
        ));
        assert!(
            plan.payload.contains(
                "if (hasCustomTicks(axisId)) {\n    return \"\";\n  }"
            )
        );
        assert!(plan.payload.contains(
            "const tickInterval = getAxisTickInterval(tickConfig, cell.id, isLogScale);"
        ));
        assert!(plan.payload.contains(
            "const minValue = resolveAxisMin(range, tickConfig, cell.id, axisDimension, isLogScale);"
        ));
        assert!(plan.payload.contains(
            "axisTick: { show: isVisible && !hideNativeTicks, length: 6 }"
        ));
        assert!(plan.payload.contains(
            "axisLabel: {\n      show: isVisible && !hideNativeTicks,"
        ));
        assert!(plan.payload.contains("chart.on(\"dataZoom\", function() {"));
    }

    #[test]
    fn build_render_plan_derives_evenly_spaced_custom_tick_interval() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,y\n0,10\n0.5,20\n1,30\n");

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 1),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "y".to_string(),
                mark: SeriesMark::Lines,
                boxplot_group: None,
                name: Some("Percentile".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path,
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.plot.axes.insert(
            AxisRef::x(1),
            AxisSpec {
                scale: AxisScale::Linear,
                number_format: AxisValueFormat::Plain { decimals: None },
                range: Some(0.0..1.0),
                label: Some("Percentile".to_string()),
                ticks: TickSpec {
                    major: None,
                    custom: vec![
                        (0.0, "0".to_string()),
                        (0.25, "25".to_string()),
                        (0.5, "50".to_string()),
                        (0.75, "75".to_string()),
                        (1.0, "100".to_string()),
                    ],
                },
            },
        );

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains(
            "\"x-bottom\":{interval:undefined,min:0,max:1,custom:[[0,\"0\"],[0.25,\"25\"],[0.5,\"50\"],[0.75,\"75\"],[1,\"100\"]]}"
        ));
        assert!(plan.payload.contains(
            "const firstStep = customTickValues[1] - customTickValues[0];"
        ));
        assert!(plan.payload.contains("return firstStep;"));
    }

    #[test]
    fn build_render_plan_sanitizes_non_positive_log_axis_minimums() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,y\n0.01,10\n0.1,20\n1,30\n");

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 1),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "y".to_string(),
                mark: SeriesMark::Lines,
                boxplot_group: None,
                name: Some("Percentile".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path,
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.plot.axes.insert(
            AxisRef::x(1),
            AxisSpec {
                scale: AxisScale::Linear,
                number_format: AxisValueFormat::Plain { decimals: None },
                range: Some(0.0..1.0),
                label: Some("Percentile".to_string()),
                ticks: TickSpec {
                    major: None,
                    custom: vec![
                        (0.0, "0".to_string()),
                        (0.1, "10".to_string()),
                        (1.0, "100".to_string()),
                    ],
                },
            },
        );

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains(
            "if (explicitMin !== undefined && explicitMin > 0) {\n    return explicitMin;\n  }"
        ));
        assert!(plan.payload.contains(
            "const customTickMin = getPositiveCustomTickMin(axisId);"
        ));
        assert!(
            plan.payload
                .contains("return getLogAxisMin(axisId, axisDimension);")
        );
    }

    #[test]
    fn build_render_plan_wires_legend_position_and_font() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,y\n1,10\n2,20\n");

        let prepared = vec![
            PreparedSeries {
                index: 0,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "x".to_string(),
                    y_expr: "y".to_string(),
                    mark: SeriesMark::Lines,
                    boxplot_group: None,
                    name: Some("Alice".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: csv_path.clone(),
                log_path: work_dir.join("series-a.log"),
            },
            PreparedSeries {
                index: 1,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "x".to_string(),
                    y_expr: "y".to_string(),
                    mark: SeriesMark::Lines,
                    boxplot_group: None,
                    name: Some("Bob".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: csv_path,
                log_path: work_dir.join("series-b.log"),
            },
        ];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.plot.legend.position = "bottom left".to_string();
        request.plot.legend.font = Some(crate::spec::FontSpec {
            family: "Fira Sans".to_string(),
            size: 16,
        });

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains(
            "id:\"legend\",kind:\"legend\",side:\"bottom\",track:2,size:"
        ));
        assert!(!plan.payload.contains("id:\"title\",kind:\"title\""));
        assert!(!plan.payload.contains("option.title = {"));
        assert!(plan.payload.contains("minorSpan:\"50%\",align:\"start\""));
        assert!(
            plan.payload
                .contains("legendFont: '16px \"Fira Sans\", sans-serif'")
        );
        assert!(plan.payload.contains(
            "textStyle: { color: \"#5f6b85\", fontFamily: \"Fira Sans\", fontSize: 16 }"
        ));
    }

    #[test]
    fn build_render_plan_emits_explicit_plot_title() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,y\n1,10\n2,20\n");

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 1),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "y".to_string(),
                mark: SeriesMark::Lines,
                boxplot_group: None,
                name: Some("Throughput".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path,
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.plot.title = Some("Network Percentiles".to_string());

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains("id:\"title\",kind:\"title\""));
        assert!(plan.payload.contains("text:\"Network Percentiles\""));
        assert!(plan.payload.contains("option.title = {"));
        assert!(plan.payload.contains("text: titleCell.text"));
    }

    #[test]
    fn build_render_plan_rejects_unknown_legend_position() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,y\n1,10\n2,20\n");

        let prepared = vec![
            PreparedSeries {
                index: 0,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "x".to_string(),
                    y_expr: "y".to_string(),
                    mark: SeriesMark::Lines,
                    boxplot_group: None,
                    name: Some("Alice".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: csv_path.clone(),
                log_path: work_dir.join("series-a.log"),
            },
            PreparedSeries {
                index: 1,
                spec: SeriesSpec {
                    axis_binding: SeriesAxisBinding::new(1, 1),
                    input_ref: 1,
                    input_filter: "true".to_string(),
                    output_filter: "true".to_string(),
                    opseq: String::new(),
                    x_expr: "x".to_string(),
                    y_expr: "y".to_string(),
                    mark: SeriesMark::Lines,
                    boxplot_group: None,
                    name: Some("Bob".to_string()),
                    style: SeriesStyle { raw: None },
                },
                output_path: csv_path,
                log_path: work_dir.join("series-b.log"),
            },
        ];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.plot.legend.position = "center right".to_string();

        let err = match EchartsBackend.build_render_plan(&request, &prepared) {
            Ok(_) => panic!("expected unknown legend position to fail"),
            Err(err) => err,
        };

        assert!(
            err.to_string()
                .contains("Unknown echarts legend position 'center right'")
        );
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
                axis_binding: SeriesAxisBinding::new(1, 1),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "y".to_string(),
                mark: SeriesMark::Lines,
                boxplot_group: None,
                name: Some("Downsampled".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path,
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.backend_options =
            BackendOptions::Echarts(EchartsBackendOptions {
                theme: Some("light".to_string()),
                max_points: Some(5),
                output_mode: EchartsOutputMode::Page,
                runtime_mode: EchartsRuntimeMode::Cdn,
            });

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains("original: 20"));
        assert!(plan.payload.contains("embedded: 5"));
        assert!(plan.payload.contains("[0,0]"));
        assert!(plan.payload.contains("[19,190]"));
    }

    #[test]
    fn build_render_plan_scales_chart_size_from_layout() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,y\n1,10\n2,20\n");

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 1),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "y".to_string(),
                mark: SeriesMark::Lines,
                boxplot_group: None,
                name: Some("Scaled".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path,
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.plot.layout.width = 0.5;
        request.plot.layout.height = 1.5;

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains(
            "class=\"msp-echarts-root\" style=\"width:400px;max-width:100%;\""
        ));
        assert!(plan.payload.contains(
            ".msp-echarts-canvas {\n  width: 100%;\n  height: 1200px;"
        ));
    }

    #[test]
    fn build_render_plan_emits_percentage_number_format_objects() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,y\n1,0.12\n2,0.34\n");

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 1),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "y".to_string(),
                mark: SeriesMark::Lines,
                boxplot_group: None,
                name: Some("Rate".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path,
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.plot.axes.insert(
            AxisRef::y(1),
            AxisSpec {
                scale: AxisScale::Linear,
                number_format: AxisValueFormat::Percentage {
                    decimals: Some(1),
                },
                range: None,
                label: Some("Rate".to_string()),
                ticks: TickSpec {
                    major: None,
                    custom: Vec::new(),
                },
            },
        );

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(
            plan.payload
                .contains("numberFormat:{mode:\"percentage\",decimals:1}")
        );
        assert!(plan.payload.contains(
            "return trimTrailingZeroes((value * 100).toFixed(precision)) + \"%\";"
        ));
    }

    #[test]
    fn build_render_plan_emits_timestamp_number_format_objects() {
        let work_dir = unique_test_dir();
        let csv_path = work_dir.join("series.csv");
        write_csv(&csv_path, "x,y\n1724457600,10\n1724544000,20\n");

        let prepared = vec![PreparedSeries {
            index: 0,
            spec: SeriesSpec {
                axis_binding: SeriesAxisBinding::new(1, 1),
                input_ref: 1,
                input_filter: "true".to_string(),
                output_filter: "true".to_string(),
                opseq: String::new(),
                x_expr: "x".to_string(),
                y_expr: "y".to_string(),
                mark: SeriesMark::Lines,
                boxplot_group: None,
                name: Some("Events".to_string()),
                style: SeriesStyle { raw: None },
            },
            output_path: csv_path,
            log_path: work_dir.join("series.log"),
        }];
        let out_path = work_dir.join("chart.html");
        let mut request = test_request(&work_dir, &out_path);
        request.plot.axes.insert(
            AxisRef::x(1),
            AxisSpec {
                scale: AxisScale::Linear,
                number_format: AxisValueFormat::Timestamp {
                    unit: TimestampUnit::Seconds,
                    timezone: Some("UTC".to_string()),
                },
                range: None,
                label: Some("Time".to_string()),
                ticks: TickSpec {
                    major: None,
                    custom: Vec::new(),
                },
            },
        );

        let plan = EchartsBackend
            .build_render_plan(&request, &prepared)
            .unwrap();

        assert!(plan.payload.contains(
            "numberFormat:{mode:\"timestamp\",unit:\"s\",timezone:\"UTC\"}"
        ));
        assert!(
            plan.payload
                .contains("formatterOptions.timeZone = formatSpec.timezone;")
        );
        assert!(plan.payload.contains("return new Intl.DateTimeFormat("));
    }
}
