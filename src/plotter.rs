use std::borrow::Cow;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

use rand::Rng;

fn temp_filename(prefix: &str) -> PathBuf {
    let tmp_dir = std::env::temp_dir();

    let mut rng = rand::rng();
    const CHARSET: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let suffix: String = (0..16)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect();

    // Combine components: /tmp/prefixXXXXXX
    tmp_dir.join(format!("{}{}", prefix, suffix))
}

fn to_rfc4180_csv_cell(input: &str) -> Cow<'_, str> {
    let needs_quoting = input.contains(|c| {
        matches!(c, ',' | '"' | '\n' | '\r') || c.is_whitespace()
    });

    if !needs_quoting {
        return Cow::Borrowed(input);
    }

    let mut escaped = String::with_capacity(input.len() + 2);
    escaped.push('"');
    for c in input.chars() {
        if c == '"' {
            escaped.push_str("\"\"");
        } else {
            escaped.push(c);
        }
    }
    escaped.push('"');

    Cow::Owned(escaped)
}

pub struct DataPoints {
    pub xtitle: String,
    pub ytitle: String,
    pub points: Vec<(f64, f64)>,
}

pub enum DataSeriesSource {
    File(File),
    Stdin(std::io::Stdin),
    Child(std::process::ChildStdout),
    Points(DataPoints),
}

impl DataSeriesSource {
    pub fn dump(self, force_path: Option<PathBuf>) -> std::io::Result<PathBuf> {
        let temp_ds_path = force_path
            .unwrap_or_else(|| temp_filename("sp-").with_extension("csv"));
        let mut temp_ds = File::create(temp_ds_path.clone())?;
        match self {
            DataSeriesSource::File(mut f) => {
                std::io::copy(&mut f, &mut temp_ds)?;
            }
            DataSeriesSource::Stdin(mut s) => {
                std::io::copy(&mut s, &mut temp_ds)?;
            }
            DataSeriesSource::Child(mut c) => {
                std::io::copy(&mut c, &mut temp_ds)?;
            }
            DataSeriesSource::Points(p) => {
                writeln!(temp_ds, "{},", to_rfc4180_csv_cell(&p.xtitle))?;
                writeln!(temp_ds, "{}\n", to_rfc4180_csv_cell(&p.ytitle))?;
                for (x, y) in p.points.iter() {
                    writeln!(temp_ds, "{},{}\n", x, y)?;
                }
            }
        }
        drop(temp_ds);
        Ok(temp_ds_path)
    }
}

pub struct Plotter {}

impl Plotter {
    pub fn plot(gpcmd: &str) -> std::io::Result<ExitStatus> {
        // generate temporary gnuplot script file
        let out_gp_name = temp_filename("sp-").with_extension("gp");
        let mut out_gp = File::create(out_gp_name.clone())?;
        writeln!(out_gp, "{}", gpcmd)?;
        drop(out_gp);

        log::info!("Temporary gnuplot script file: {}", out_gp_name.display());
        // call gnuplot
        Command::new("gnuplot").arg("-p").arg(&out_gp_name).status()
    }
}
