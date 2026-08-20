use std::{fs::File, io::Write, process::Command};

use anyhow::{Context, bail};
use spreadsheet_plotter::{
    AxisOptions, DataSeriesOptions, GnuplotTemplate, PlotType, StandardTics,
    Terminal,
};

use crate::{
    backend::{Backend, RenderPlan},
    spec::{
        AxisId, AxisScale, BackendOptions, GnuplotBackendOptions,
        PreparedSeries, ResolvedMspRequest, SeriesMark,
    },
};

pub struct GnuplotBackend;

fn parse_terminal(
    request: &ResolvedMspRequest,
    options: &GnuplotBackendOptions,
) -> anyhow::Result<Terminal> {
    let terminal_name = options.terminal.as_deref().unwrap_or_else(|| {
        if request.render_target.out.is_some() {
            "postscript"
        } else {
            "x11"
        }
    });

    match terminal_name.to_ascii_lowercase().as_str() {
        "x11" => Ok(Terminal::X11),
        "postscript" | "pdf" => Ok(Terminal::Postscript),
        "dumb" => Ok(Terminal::Dumb(None, None)),
        other => bail!("Unknown gnuplot terminal '{other}'"),
    }
}

fn axis_options(
    request: &ResolvedMspRequest,
    axis_id: AxisId,
) -> anyhow::Result<AxisOptions> {
    let axis = match axis_id {
        AxisId::X => &request.plot.axes.x,
        AxisId::Y => &request.plot.axes.y,
        AxisId::X2 => &request.plot.axes.x2,
        AxisId::Y2 => &request.plot.axes.y2,
    };
    let base = match axis_id {
        AxisId::X => AxisOptions::new_x(),
        AxisId::Y => AxisOptions::new_y(),
        AxisId::X2 => AxisOptions::new_x2(),
        AxisId::Y2 => AxisOptions::new_y2(),
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

fn plot_type(mark: SeriesMark) -> PlotType {
    match mark {
        SeriesMark::Points => PlotType::Points(None),
        SeriesMark::Lines => PlotType::Lines(None),
        SeriesMark::LinesPoints => PlotType::Linespoints(None, None),
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
    let terminal = parse_terminal(request, backend_options)?;
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
            .with_plot_type(plot_type(series.spec.mark))
            .with_label(series.spec.name.as_deref())
            .with_additional_option(series.spec.style.raw.as_deref())
            .with_use_x2(series.spec.axis_binding.use_x2())
            .with_use_y2(series.spec.axis_binding.use_y2()))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(GnuplotTemplate::default()
        .with_additional_command(backend_options.pre_plot_snippet.as_deref())
        .with_data_series_options(data_series_options)
        .with_xopt(axis_options(request, AxisId::X)?)
        .with_yopt(axis_options(request, AxisId::Y)?)
        .with_x2opt(axis_options(request, AxisId::X2)?)
        .with_y2opt(axis_options(request, AxisId::Y2)?)
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
