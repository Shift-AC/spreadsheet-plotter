use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use crate::commons::temp_filename;
use anyhow::bail;
use anyhow::{Context, Result};

pub enum InputStream {
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

pub struct Plotter {
    input: Option<InputStream>,
}

impl Plotter {
    pub fn new(input: Option<InputStream>) -> Self {
        Self { input }
    }

    pub fn get_temp_datasheet_path() -> PathBuf {
        std::env::temp_dir().join(format!("{}.spdata", env!("VERSION")))
    }

    pub fn plot(mut self, gpcmd: &str) -> Result<()> {
        if let Some(ref mut input) = self.input {
            // dump the input to a temporary datasheet file
            let temp_ds_path = Self::get_temp_datasheet_path();
            let mut temp_ds = File::create(temp_ds_path.clone())?;
            std::io::copy(input, &mut temp_ds)?;
            drop(temp_ds);
        }

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
}
