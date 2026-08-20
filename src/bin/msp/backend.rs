use std::{fs, path::PathBuf, process::Command};

use anyhow::Context;

use crate::spec::{BackendKind, PreparedSeries, ResolvedMspRequest};

#[path = "backends/echarts.rs"]
pub mod echarts;
#[path = "backends/gnuplot.rs"]
pub mod gnuplot;

pub struct RenderPlan {
    pub description: String,
    pub payload: String,
    pub artifact_path: Option<PathBuf>,
}

pub trait Backend {
    fn build_render_plan(
        &self,
        request: &ResolvedMspRequest,
        prepared: &[PreparedSeries],
    ) -> anyhow::Result<RenderPlan>;

    fn execute(
        &self,
        plan: &RenderPlan,
        request: &ResolvedMspRequest,
    ) -> anyhow::Result<()>;
}

pub fn create_backend(kind: BackendKind) -> Box<dyn Backend> {
    match kind {
        BackendKind::Gnuplot => Box::new(gnuplot::GnuplotBackend),
        BackendKind::Echarts => Box::new(echarts::EchartsBackend),
    }
}

pub fn emit_prepare_report(
    request: &ResolvedMspRequest,
    prepared: &[PreparedSeries],
) -> String {
    let mut out = request.describe();
    out.push_str("prepared_series:\n");
    for series in prepared {
        out.push_str(&format!(
            "  - #{} data={} log={}\n",
            series.index + 1,
            series.output_path.display(),
            series.log_path.display()
        ));
    }
    out
}

pub fn write_artifact(plan: &RenderPlan) -> anyhow::Result<()> {
    if let Some(path) = &plan.artifact_path {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create artifact directory '{}'",
                    parent.display()
                )
            })?;
        }
        fs::write(path, &plan.payload).with_context(|| {
            format!("Failed to write artifact '{}'", path.display())
        })?;
    }
    Ok(())
}

pub fn maybe_open_in_browser(path: &PathBuf) -> anyhow::Result<()> {
    if which::which("xdg-open").is_err() {
        return Ok(());
    }

    let status =
        Command::new("xdg-open")
            .arg(path)
            .status()
            .with_context(|| {
                format!("Failed to launch xdg-open for '{}'", path.display())
            })?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "xdg-open failed for '{}' with exit code {:?}",
            path.display(),
            status.code()
        ))
    }
}
