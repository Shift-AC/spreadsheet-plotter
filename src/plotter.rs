use std::fmt::Display;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::io::stdout;
use std::path::PathBuf;
use std::process::Command;

use crate::cachefile::StateCache;
use crate::cachefile::StateCacheHeader;
use crate::commons::get_current_time_micros;
use crate::commons::temp_filename;
use crate::datasheet::Datasheet;
use crate::datasheet::DatasheetFormat;
use crate::opeseq::Dumper;
use crate::opeseq::OpSeq;
use crate::opeseq::Operator;
use crate::opeseq::OutputFormat;
use crate::opeseq::Transformer;
use crate::preprocess::DataPreprocessor;
use anyhow::bail;
use anyhow::{Context, Result, anyhow};
use log::debug;

// The core of spreadsheet plotter.
// It takes a datasheet and a operation sequence (OpSeq) as input, performs
// proper transformations on the datasheet and dumps/plots the result as
// specified by the operations. Also handles processing of raw command line
// arguments
pub struct Plotter {
    ds: Datasheet,
    ops: OpSeq,
    xexpr: String,
    yexpr: String,
    input_path: Option<PathBuf>,
    input_format: DatasheetFormat,
    output_format: DatasheetFormat,
    cache_header: Option<StateCacheHeader>,
    cache_output_order: usize,
    cache_output_prefix: String,
}

enum InputStream {
    File(File),
    Stdin(std::io::Stdin),
}

impl Read for InputStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            InputStream::File(f) => f.read(buf),
            InputStream::Stdin(s) => s.read(buf),
        }
    }
}

impl Plotter {
    pub fn get_temp_datasheet_path() -> PathBuf {
        std::env::temp_dir().join(format!("{}.spdata", env!("VERSION")))
    }

    pub fn plot(gpcmd: &str) -> Result<()> {
        // generate temporary gnuplot script file
        let out_gp_name = temp_filename("sp-").with_extension("gp");
        let mut out_gp = File::create(out_gp_name.clone())?;
        writeln!(out_gp, "{}", gpcmd)?;
        drop(out_gp);

        log::info!("Temporary gnuplot script file: {}", out_gp_name.display());
        // call gnuplot
        let status = Command::new("gnuplot")
            .arg(&out_gp_name)
            .status()
            .context("Failed to execute gnuplot")?;
        if !status.success() {
            bail!("gnuplot failed with status: {}", status.code().unwrap());
        }
        Ok(())
    }

    pub fn from_single_input_file(
        input_path: Option<PathBuf>,
        ops_str: String,
        xexpr: String,
        yexpr: String,
        input_format: DatasheetFormat,
        output_format: DatasheetFormat,
        gpcmd: String,
        cache_output_prefix: String,
    ) -> Result<Self> {
        // If no preprocessing is needed and only plotting is required,
        // it is possible to tell gnuplot to plot from the original datasheet.
        //
        // However, this feature was removed because directly calling gnuplot
        // on a gigantic sheet could be significantly slower than creating the
        // temporary file with only two columns.
        let cache_output_prefix = cache_output_prefix.to_string();
        let tmp_datasheet_path = Self::get_temp_datasheet_path();

        let mut input_stream = match &input_path {
            Some(path) => InputStream::File(File::open(path)?),
            None => InputStream::Stdin(std::io::stdin()),
        };

        debug!("Time info: before_load = {}", get_current_time_micros());
        let (skip_len, ds, cache_header) =
            if matches!(input_format, DatasheetFormat::SPLNK) {
                let cache = StateCache::from_reader(&mut input_stream)?;
                let skip_len =
                    OpSeq::opseq_matched_len(&ops_str, &cache.header.opstr);
                // match failed, try to restart from the source path stored in cache
                if skip_len == 0 {
                    return Self::from_single_input_file(
                        Some(cache.header.input_path),
                        ops_str,
                        xexpr,
                        yexpr,
                        cache.header.input_format.clone(),
                        output_format.clone(),
                        gpcmd,
                        cache_output_prefix,
                    );
                }
                // otherwise use the cached datasheet
                let ds = cache.ds.into_owned();
                (skip_len, ds, Some(cache.header))
            } else {
                let ds = DataPreprocessor::preprocess(
                    &mut input_stream,
                    input_format.clone(),
                    &xexpr,
                    &yexpr,
                )?;
                debug!(
                    "Time info: after_preprocess = {}",
                    get_current_time_micros()
                );
                (0, ds, None)
            };

        debug!("Time info: after_load = {}", get_current_time_micros());

        let ops: OpSeq = OpSeq::new(&ops_str[skip_len..], &|| {
            Box::new(PlotDumper::new(gpcmd.clone(), tmp_datasheet_path.clone()))
        })?;

        Ok(Self {
            ds: ds,
            ops,
            xexpr,
            yexpr,
            input_path,
            input_format,
            output_format,
            cache_header,
            cache_output_order: 0,
            cache_output_prefix,
        })
    }

    fn generate_cache(&self, opseq_index: usize) -> Result<StateCacheHeader> {
        if self.input_path.is_none() && self.cache_header.is_none() {
            bail!(
                "Cache file could only be created for non-stdin source datasheets"
            );
        }

        match &self.cache_header {
            Some(header) => {
                let mut header = header.clone();
                header.output_format = self.output_format.clone();
                header.opstr = format!(
                    "{}{}",
                    header.opstr,
                    self.ops.to_string(opseq_index - 1, true)
                );
                Ok(header)
            }
            None => Ok(StateCacheHeader {
                input_path: self.input_path.clone().unwrap(),
                xexpr: self.xexpr.clone(),
                yexpr: self.yexpr.clone(),
                input_format: self.input_format.clone(),
                output_format: self.output_format.clone(),
                opstr: self.ops.to_string(opseq_index - 1, true),
            }),
        }
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
                    self.ds = transform.apply(std::mem::take(&mut self.ds))?;
                }
                Operator::Dump(dump) => {
                    let (out_format, out_target): (
                        OutputFormat,
                        &mut dyn Write,
                    ) = match dump.to_string().as_str() {
                        "C" => {
                            let header = self.generate_cache(i)?;

                            let filename = format!(
                                "{}{}",
                                self.cache_output_prefix,
                                self.cache_output_order,
                            );
                            self.cache_output_order += 1;

                            (
                                OutputFormat::Cache(header),
                                &mut File::create(filename)?,
                            )
                        }
                        "O" => (
                            OutputFormat::DataSheet(self.output_format.clone()),
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
                    dump.apply(&self.ds, &out_format, out_target)?;
                }
            }
        }
        debug!("Time info: after_apply = {}", get_current_time_micros());
        Ok(())
    }
}

struct PlotDumper {
    gpcmd: String,
    tmp_datasheet: PathBuf,
}

impl PlotDumper {
    pub fn new(gpcmd: String, tmp_datasheet: PathBuf) -> Self {
        Self {
            gpcmd,
            tmp_datasheet,
        }
    }
}

impl Display for PlotDumper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "P")
    }
}

impl Dumper for PlotDumper {
    fn apply(
        &self,
        ds: &Datasheet,
        _: &OutputFormat,
        _: &mut dyn Write,
    ) -> Result<()> {
        // print the datasheet to the temporary file
        let mut out_datasheet = File::create(self.tmp_datasheet.clone())?;
        ds.to_csv(true, &mut out_datasheet)?;
        drop(out_datasheet);

        Plotter::plot(&self.gpcmd)?;
        Ok(())
    }
}
