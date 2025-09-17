use std::fs;
use std::fs::File;
use std::io::BufWriter;
use std::io::Write;
use std::io::stdout;
use std::process::Command;

use crate::cachefile::StateCacheWriter;
use crate::cachefile::state_cache_filename;
use crate::column::excel_column_name_to_index;
use crate::column::expression_is_single_column;
use crate::column::process_column_expressions_on_datasheet;
use crate::commons::ProtectedDir;
use crate::commons::get_current_time_micros;
use crate::commons::temp_filename;
use crate::datasheet::DataSheetFormat;
use crate::datasheet::Datasheet;
use crate::opeseq::Dumper;
use crate::opeseq::OpSeq;
use crate::opeseq::Operator;
use crate::opeseq::OutputFormat;
use crate::opeseq::Transformer;
use anyhow::{Context, Result, anyhow};
use log::debug;

const GNUPLOT_INIT_CMD: &str = r"
set key autotitle columnhead
set terminal dumb size `tput cols`,`echo $(($(tput lines) - 1))`
set datafile separator ','
";

#[derive(Clone)]
pub struct GnuplotCommand {
    cmd: String,
}

impl GnuplotCommand {
    // fit provided additional commands into hard-coded command template
    pub fn from_additional_cmd(additional_cmd: &str) -> Self {
        let cmd = format!(
            "{}\n{}\nplot input_file using xaxis:yaxis",
            GNUPLOT_INIT_CMD, additional_cmd
        );
        Self { cmd }
    }
    // read gnuplot command from file
    pub fn from_file(path: &str) -> Result<Self> {
        let cmd = fs::read_to_string(path)
            .context("Error reading gnuplot command file")?;
        Ok(Self { cmd })
    }

    pub fn to_full_cmd(
        &self,
        datasheet_path: &str,
        xaxis: &str,
        yaxis: &str,
    ) -> String {
        let macro_cmd = format!(
            "set macro\n\
            input_file = '{}'\n\
            xaxis={}\n\
            yaxis={}\n",
            datasheet_path, xaxis, yaxis
        );

        format!("{}{}\n", macro_cmd, self.cmd)
    }
}

// The core of spreadsheet plotter.
// It takes a datasheet and a operation sequence (OpSeq) as input, performs
// proper transformations on the datasheet and dumps/plots the result as
// specified by the operations. Also handles processing of raw command line
// arguments
pub struct Plotter {
    ds: Option<Datasheet>,
    skipped_ops_str: String,
    ops: OpSeq,
    xexpr: String,
    yexpr: String,
    ds_path: String,
    ds_in_format: DataSheetFormat,
    ds_out_format: DataSheetFormat,
    out_dir: ProtectedDir,
    splnk: Option<StateCacheWriter>,
}

impl Plotter {
    pub fn new(
        ds_path: &str,
        ops_str: &str,
        xexpr: &str,
        yexpr: &str,
        ds_in_format: DataSheetFormat,
        ds_out_format: DataSheetFormat,
        out_dir: &str,
        gpcmd: GnuplotCommand,
        preserve: bool,
    ) -> Result<Self> {
        let plot_only = env!("CONFIG_PLOT_ONLY_MODE_ENABLED") == "1"
            && &ds_in_format.get_fmt_str() != "lnk"
            && expression_is_single_column(xexpr)
            && expression_is_single_column(yexpr)
            && ops_str == "P";

        // If no preprocessing is needed and only plotting is required,
        // we can directly tell gnuplot to plot from the original datasheet.
        //
        // NOTE: directly calling gnuplot on a gigantic sheet could be
        // significantly slower than creating the temporary file with only two
        // columns.

        if plot_only {
            return Ok(Self {
                ds: Some(Datasheet::new(Vec::new(), Vec::new(), None)),
                skipped_ops_str: "".to_string(),
                ops: OpSeq::new(ops_str, &|| {
                    Box::new(PlotDumper::new(
                        &gpcmd,
                        Some(PlotDataInfo::new(
                            ds_path.to_string(),
                            xexpr,
                            yexpr,
                        )),
                        preserve,
                    ))
                })?,
                xexpr: xexpr.to_string(),
                yexpr: yexpr.to_string(),
                ds_path: ds_path.to_string(),
                ds_in_format,
                ds_out_format,
                out_dir: ProtectedDir::from_path_str(out_dir)?,
                splnk: None,
            });
        }

        let (mut ds, load_info) = Datasheet::read(&ds_in_format, ds_path)?;
        debug!("Time info: after_load = {}", get_current_time_micros());
        let (ops, skipped_ops_str) = match &load_info {
            Some(info) => {
                let skip_len = info.opseq_skip_len;
                (&ops_str[skip_len..], ops_str[..skip_len].to_string())
            }
            None => {
                ds = process_column_expressions_on_datasheet(ds, xexpr, yexpr)?;
                (ops_str, "".to_string())
            }
        };
        debug!(
            "Time info: after_preprocess = {}",
            get_current_time_micros()
        );

        let (xexpr, yexpr) = match load_info {
            Some(info) => (info.xexpr, info.yexpr),
            None => (xexpr.to_string(), yexpr.to_string()),
        };

        Ok(Self {
            ds: Some(ds),
            skipped_ops_str,
            ops: OpSeq::new(ops, &|| {
                Box::new(PlotDumper::new(&gpcmd, None, preserve))
            })?,
            xexpr,
            yexpr,
            ds_path: ds_path.to_string(),
            ds_in_format,
            ds_out_format,
            out_dir: ProtectedDir::from_path_str(out_dir)?,
            splnk: None,
        })
    }

    pub fn apply(&mut self) -> Result<()> {
        for i in 0..self.ops.ops.len() {
            let op = &self.ops.ops[i];
            debug!(
                "Time info: before_apply_{} = {}",
                i + 1,
                get_current_time_micros()
            );
            debug!(
                "Current operator: {} ({}/{})",
                op.to_string(),
                i + 1,
                self.ops.ops.len()
            );
            match op {
                Operator::Transform(transform) => {
                    self.ds = Some(transform.apply(
                        self.ds.take().ok_or_else(|| {
                            anyhow!("BUG: empty datasheet in plotter")
                        })?,
                        0,
                        1,
                    )?);
                }
                Operator::Dump(dump) => {
                    let (out_format, out_target): (
                        OutputFormat,
                        &mut dyn Write,
                    ) = match dump.to_string().as_str() {
                        "C" => {
                            let filename = state_cache_filename(
                                &(self.skipped_ops_str.clone()
                                    + &self.ops.to_string(i - 1, true)),
                            );
                            (
                                OutputFormat::DataSheet(
                                    self.ds_out_format.clone(),
                                ),
                                &mut self
                                    .out_dir
                                    .create_output_file(&filename)
                                    .context(anyhow!(
                                        "Error creating output file {}",
                                        filename
                                    ))?,
                            )
                        }
                        "O" => (
                            OutputFormat::DataSheet(self.ds_out_format.clone()),
                            &mut stdout(),
                        ),
                        "P" => (OutputFormat::Plot, &mut stdout()),

                        _ => {
                            return Err(anyhow!(
                                "BUG: unexpected dump operator {}",
                                dump.to_string()
                            ));
                        }
                    };
                    dump.apply(
                        self.ds.as_ref().ok_or_else(|| {
                            anyhow!("BUG: empty datasheet in plotter")
                        })?,
                        &out_format,
                        out_target,
                    )?;
                    if dump.to_string() == "C" {
                        if self.splnk.is_none() {
                            self.splnk = Some(StateCacheWriter::new(
                                &self.out_dir,
                                &self.ds_path,
                                &self.xexpr,
                                &self.yexpr,
                                &self.ds_in_format.get_fmt_str(),
                                self.ds_in_format.has_header(),
                                &self.ds_out_format.get_fmt_str(),
                            )?);
                        }
                        self.splnk
                            .as_mut()
                            .unwrap()
                            .write_cache_metadata(&self.ops, i - 1)?;
                    }
                }
            }
        }
        debug!("Time info: after_apply = {}", get_current_time_micros());
        Ok(())
    }
}

struct PlotDataInfo {
    filename: String,
    xexpr: String,
    yexpr: String,
}

impl PlotDataInfo {
    fn convert_raw_expr(expr: &str) -> String {
        let expr = expr.trim();
        if expr.starts_with('#') {
            if expr[1..].chars().any(|c| c.is_ascii_digit()) {
                expr[1..].to_string()
            } else {
                excel_column_name_to_index(&expr[1..]).unwrap().to_string()
            }
        } else if expr.starts_with('@') {
            format!(
                "'{}'",
                expr[1..expr.len() - 1].to_string().replace("\\@", "@")
            )
        } else {
            panic!("BUG: unexpected non-single-column expression {}", expr)
        }
    }

    pub fn new(filename: String, xexpr: &str, yexpr: &str) -> Self {
        Self {
            filename,
            xexpr: Self::convert_raw_expr(xexpr),
            yexpr: Self::convert_raw_expr(yexpr),
        }
    }
}

struct PlotDumper {
    gpcmd: GnuplotCommand,
    // if present, overrides the temporary datasheet file
    data_info: Option<PlotDataInfo>,
    preserve: bool,
}

impl PlotDumper {
    pub fn new(
        gpcmd: &GnuplotCommand,
        data_info: Option<PlotDataInfo>,
        preserve: bool,
    ) -> Self {
        Self {
            gpcmd: gpcmd.clone(),
            data_info,
            preserve,
        }
    }
}

impl Dumper for PlotDumper {
    fn apply(
        &self,
        ds: &Datasheet,
        _: &OutputFormat,
        _: &mut dyn Write,
    ) -> Result<()> {
        let out_file_basename = temp_filename("sp-");
        // generate temporary data sheet file
        let out_datasheet_name = match &self.data_info {
            Some(info) => info.filename.clone(),
            None => {
                let name = out_file_basename.with_extension("csv");
                ds.to_csv(
                    true,
                    &mut csv::Writer::from_writer(BufWriter::new(
                        File::create(name.clone())?,
                    )),
                )?;
                format!("{}", name.display())
            }
        };

        let (xaxis, yaxis) = match &self.data_info {
            Some(info) => (info.xexpr.clone(), info.yexpr.clone()),
            None => ("1".to_string(), "2".to_string()),
        };

        // generate temporary gnuplot script file
        let out_gp_name = out_file_basename.with_extension("gp");
        let mut out_gp = File::create(out_gp_name.clone())?;
        let gpcmd = self.gpcmd.to_full_cmd(&out_datasheet_name, &xaxis, &yaxis);
        writeln!(out_gp, "{}", gpcmd)?;
        drop(out_gp);

        if self.preserve {
            println!("Temporary data sheet file: {}", out_datasheet_name);
            println!(
                "Temporary gnuplot script file: {}",
                out_gp_name.display()
            );
        }

        // call gnuplot
        let status = Command::new("gnuplot")
            .arg(out_file_basename.with_extension("gp"))
            .status()
            .context("Failed to execute gnuplot")?;
        if !status.success() {
            return Err(anyhow!(
                "gnuplot failed with status: {}",
                status.code().unwrap()
            ));
        }
        if !self.preserve {
            std::fs::remove_file(out_datasheet_name)?;
            std::fs::remove_file(out_gp_name)?;
        }
        Ok(())
    }
    fn to_string(&self) -> String {
        "P".to_string()
    }
}
