use std::fs;
use std::fs::File;
use std::io::BufWriter;
use std::io::Write;
use std::io::stdout;
use std::process::Command;

use crate::cachefile::StateCacheWriter;
use crate::column::process_column_expressions_on_datasheet;
use crate::commons::ProtectedDir;
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
use log::info;

#[derive(Clone)]
pub struct AdditionalGnuplotCommand {
    cmd: String,
}

impl AdditionalGnuplotCommand {
    pub fn new(cmd: &str) -> Self {
        Self {
            cmd: cmd.to_string(),
        }
    }
    pub fn from_file(path: &str) -> Result<Self> {
        let cmd = fs::read_to_string(path)
            .context("Error reading gnuplot command file")?;
        Ok(Self { cmd })
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
        gpcmd: AdditionalGnuplotCommand,
    ) -> Result<Self> {
        let (mut ds, ops_skip_len) = Datasheet::read(&ds_in_format, ds_path)?;
        let (ops, skipped_ops_str) = match &ops_skip_len {
            Some(skip_len) => {
                (&ops_str[*skip_len..], ops_str[..*skip_len].to_string())
            }
            None => {
                ds = process_column_expressions_on_datasheet(ds, xexpr, yexpr)?;
                (ops_str, "".to_string())
            }
        };

        Ok(Self {
            ds: Some(ds),
            skipped_ops_str,
            ops: OpSeq::new(ops, &|| Box::new(PlotDumper::new(&gpcmd)))?,
            xexpr: xexpr.to_string(),
            yexpr: yexpr.to_string(),
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
                "Applying operator {}/{}: {}",
                i + 1,
                self.ops.ops.len(),
                op.to_string()
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
                            let filename = self.skipped_ops_str.clone()
                                + &self.ops.to_string(i, true);
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
                            .write_cache_metadata(&self.ops, i)?;
                    }
                }
            }
        }
        Ok(())
    }
}

pub struct PlotDumper {
    gpcmd: AdditionalGnuplotCommand,
}

const GNUPLOT_INIT_CMD: &str = r"
set key autotitle columnhead
set terminal dumb size `tput cols`,`echo $(($(tput lines) - 1))`
set datafile separator ','
";

impl PlotDumper {
    pub fn new(gpcmd: &AdditionalGnuplotCommand) -> Self {
        Self {
            gpcmd: gpcmd.clone(),
        }
    }
    fn generate_full_gpcmd(&self, out_datasheet_path: &str) -> String {
        let gpcmd = format!(
            "{}\n{}\nplot '{}' using 1:2\n",
            GNUPLOT_INIT_CMD, self.gpcmd.cmd, out_datasheet_path
        );

        gpcmd
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
        let out_datasheet_name = out_file_basename.with_extension("csv");
        let out_datasheet = File::create(out_datasheet_name.clone())?;
        ds.to_csv(
            true,
            &mut csv::Writer::from_writer(BufWriter::new(out_datasheet)),
        )?;

        // generate temporary gnuplot scr1ipt file
        let out_gp_name = out_file_basename.with_extension("gp");
        let mut out_gp = File::create(out_gp_name.clone())?;
        let gpcmd = self
            .generate_full_gpcmd(&out_datasheet_name.clone().to_string_lossy());
        writeln!(out_gp, "{}", gpcmd)?;

        info!(
            "temporary data sheet file: {}",
            out_datasheet_name.display()
        );
        info!("temporary gnuplot script file: {}", out_gp_name.display());

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
        Ok(())
    }
    fn to_string(&self) -> String {
        "P".to_string()
    }
}
