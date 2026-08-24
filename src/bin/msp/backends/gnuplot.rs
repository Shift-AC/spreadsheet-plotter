use std::{fs::File, io::Write, process::Command};

use anyhow::{Context, bail};
use spreadsheet_plotter::{
    AxisOptions, DataSeriesOptions, GnuplotTemplate, PlotType, StandardTics,
    Terminal,
};

use crate::{
    backend::{Backend, RenderPlan},
    spec::{
        AxisRef, AxisScale, BackendKind, BackendOptions, PreparedSeries,
        ResolvedMspRequest, SeriesMark,
    },
};

pub struct GnuplotBackend;

fn parse_terminal(request: &ResolvedMspRequest) -> anyhow::Result<Terminal> {
    match request.backend {
        BackendKind::GnuplotPostscript => Ok(Terminal::Postscript),
        BackendKind::GnuplotDumb => Ok(Terminal::Dumb(None, None)),
        BackendKind::GnuplotX11 => Ok(Terminal::X11),
        other => bail!(
            "gnuplot backend expected a gnuplot.* backend kind, got '{}'",
            other.display_name()
        ),
    }
}

fn axis_options(
    request: &ResolvedMspRequest,
    axis_id: AxisRef,
) -> anyhow::Result<AxisOptions> {
    let axis = match axis_id {
        axis if axis == AxisRef::x(1) => request.plot.axes.x1(),
        axis if axis == AxisRef::y(1) => request.plot.axes.y1(),
        axis if axis == AxisRef::x(2) => request.plot.axes.x2(),
        axis if axis == AxisRef::y(2) => request.plot.axes.y2(),
        _ => bail!("gnuplot backend supports only x1, x2, y1, and y2"),
    };
    let base = match axis_id {
        axis if axis == AxisRef::x(1) => AxisOptions::new_x(),
        axis if axis == AxisRef::y(1) => AxisOptions::new_y(),
        axis if axis == AxisRef::x(2) => AxisOptions::new_x2(),
        axis if axis == AxisRef::y(2) => AxisOptions::new_y2(),
        _ => bail!("gnuplot backend supports only x1, x2, y1, and y2"),
    };
    let log = match axis.scale {
        AxisScale::Linear => None,
        AxisScale::Log10 => Some(10.0),
    };
    let base = base
        .with_range(axis.range.clone())
        .with_label(axis.label.as_deref())
        .with_logscale(log)
        .with_standard_tics(axis.ticks.major.as_ref().map(|tics| {
            StandardTics {
                range: tics.range.clone(),
                step: tics.step,
            }
        }));
    Ok(if axis.ticks.custom.is_empty() {
        base
    } else {
        base.with_custom_tics(axis.ticks.custom.clone())
    })
}

fn plot_type(mark: SeriesMark) -> anyhow::Result<PlotType> {
    match mark {
        SeriesMark::Points => Ok(PlotType::Points(None)),
        SeriesMark::Lines => Ok(PlotType::Lines(None)),
        SeriesMark::LinesPoints => Ok(PlotType::Linespoints(None, None)),
        SeriesMark::Bar => bail!(
            "gnuplot backend does not support bar series via the backend-neutral mark model yet"
        ),
        SeriesMark::Boxplot => bail!(
            "gnuplot backend does not support boxplot series via the backend-neutral mark model yet"
        ),
    }
}

fn build_script(
    request: &ResolvedMspRequest,
    prepared: &[PreparedSeries],
) -> anyhow::Result<String> {
    let backend_options = match &request.backend_options {
        BackendOptions::Gnuplot(options) => options,
        _ => bail!("Expected gnuplot backend options"),
    };
    let terminal = parse_terminal(request)?;
    let font = request
        .plot
        .theme
        .font
        .as_ref()
        .map(|font| (font.family.as_str(), font.size));
    let key_font = request
        .plot
        .legend
        .font
        .as_ref()
        .map(|font| (font.family.as_str(), font.size))
        .or(font);

    let data_series_options = prepared
        .iter()
        .map(|series| {
            Ok(DataSeriesOptions::from_datasheet_path(
                series.output_path.display().to_string(),
            )
            .with_plot_type(plot_type(series.spec.mark)?)
            .with_label(series.spec.name.as_deref())
            .with_additional_option(series.spec.style.raw.as_deref())
            .with_use_x2(series.spec.axis_binding.use_x2())
            .with_use_y2(series.spec.axis_binding.use_y2()))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(GnuplotTemplate::default()
        .with_additional_command(backend_options.pre_plot_snippet.as_deref())
        .with_data_series_options(data_series_options)
        .with_xopt(axis_options(request, AxisRef::x(1))?)
        .with_yopt(axis_options(request, AxisRef::y(1))?)
        .with_x2opt(axis_options(request, AxisRef::x(2))?)
        .with_y2opt(axis_options(request, AxisRef::y(2))?)
        .with_terminal(terminal)
        .with_font(font)
        .with_grid(request.plot.grid)
        .with_key_font(key_font)
        .with_key_position(request.plot.legend.position.clone())
        .with_output(
            request
                .render_target
                .out
                .as_ref()
                .map(|path| path.display().to_string()),
        )
        .with_plot_size(
            request.plot.layout.width as f64,
            request.plot.layout.height as f64,
        )
        .to_string())
}

impl Backend for GnuplotBackend {
    fn build_render_plan(
        &self,
        request: &ResolvedMspRequest,
        prepared: &[PreparedSeries],
    ) -> anyhow::Result<RenderPlan> {
        let script = build_script(request, prepared)?;
        let gp_path = request.render_target.work_dir.join("msp-render.gnuplot");
        Ok(RenderPlan {
            description: format!("gnuplot script -> {}", gp_path.display()),
            payload: script,
            artifact_path: Some(gp_path),
        })
    }

    fn execute(
        &self,
        plan: &RenderPlan,
        _request: &ResolvedMspRequest,
    ) -> anyhow::Result<()> {
        let path = plan
            .artifact_path
            .as_ref()
            .context("gnuplot render plan does not have a script path")?;
        let mut file = File::create(path).with_context(|| {
            format!("Failed to create gnuplot script '{}'", path.display())
        })?;
        write!(file, "{}", plan.payload).with_context(|| {
            format!("Failed to write gnuplot script '{}'", path.display())
        })?;
        drop(file);

        let status = Command::new("gnuplot")
            .arg("-p")
            .arg(path)
            .status()
            .context("Failed to launch gnuplot")?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "gnuplot failed with exit code {:?}",
                status.code()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_terminal;
    use crate::spec::{
        BackendKind, BackendOptions, DataPrepSpec, GnuplotBackendOptions,
        LayoutSpec, LegendSpec, PlotAxes, PlotSpec, RenderTarget,
        ResolvedMspRequest, ThemeSpec,
    };
    use spreadsheet_plotter::Terminal;

    fn request(backend: BackendKind) -> ResolvedMspRequest {
        ResolvedMspRequest {
            mode: crate::spec::ExecutionMode::DryRun,
            backend,
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
                axes: PlotAxes::default(),
                grid: false,
            },
            render_target: RenderTarget {
                work_dir: std::env::temp_dir(),
                out: None,
                format_hint: None,
                open: false,
            },
            backend_options: BackendOptions::Gnuplot(GnuplotBackendOptions {
                pre_plot_snippet: None,
            }),
        }
    }

    #[test]
    fn parse_terminal_maps_explicit_gnuplot_backends() {
        assert!(matches!(
            parse_terminal(&request(BackendKind::GnuplotPostscript)).unwrap(),
            Terminal::Postscript
        ));
        assert!(matches!(
            parse_terminal(&request(BackendKind::GnuplotDumb)).unwrap(),
            Terminal::Dumb(_, _)
        ));
        assert!(matches!(
            parse_terminal(&request(BackendKind::GnuplotX11)).unwrap(),
            Terminal::X11
        ));
    }

    #[test]
    fn parse_terminal_rejects_non_gnuplot_backend_kind() {
        let err = parse_terminal(&request(BackendKind::Echarts)).unwrap_err();
        assert!(
            err.to_string()
                .contains("expected a gnuplot.* backend kind")
        );
    }
}
