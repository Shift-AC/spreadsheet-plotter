use std::{
    process::{Command, Stdio},
    str::FromStr,
};

use anyhow::bail;

use crate::datasheet::{Datasheet, DatasheetFormat};

#[derive(strum::Display)]
pub enum ColumnExpr {
    #[strum(serialize = "InstantExpr({0})")]
    InstantExpr(String),
    #[strum(serialize = "Instant({0})")]
    Instant(f64),
    #[strum(serialize = "Index({0})")]
    /// Column index, 1-based
    Index(usize),
    #[strum(serialize = "Name({0})")]
    Name(String),
    #[strum(serialize = "ColumnExpr({0})")]
    ColumnExpr(String),
}

impl ColumnExpr {
    pub fn to_column_header(&self) -> String {
        match self {
            ColumnExpr::InstantExpr(_) => "INSTANT".to_string(),
            ColumnExpr::Instant(v) => v.to_string(),
            ColumnExpr::Index(i) => i.to_string(),
            ColumnExpr::Name(s) => s.to_string(),
            ColumnExpr::ColumnExpr(s) => s.to_string(),
        }
    }

    fn retrieve_column_index(s: &str) -> Option<usize> {
        if s.len() > 7 && &s[..4] == "$[[[" && &s[s.len() - 3..s.len()] == "]]]"
        {
            match s[4..s.len() - 3].parse() {
                Ok(i) => Some(i),
                Err(_) => None,
            }
        } else {
            None
        }
    }

    /// conservatively check for strings that is definitely a column name,
    /// leaving out rare cases
    fn retrieve_column_name(s: &str) -> Option<&str> {
        let s = s.trim();

        if s.starts_with('$') {
            // simple column name that does not contain any special characters
            if s[1..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                Some(&s[1..])
            }
            // braced column name that does not contain '}'
            else if !s[..s.len() - 1].contains('}')
                && s[1..].starts_with('{')
                && s.ends_with('}')
            {
                Some(&s[2..s.len() - 1])
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn evaluate(&mut self) -> anyhow::Result<()> {
        if let ColumnExpr::InstantExpr(s) = self {
            let command =
                format!("mlr --csv put 'begin{{print ({})}}' <<< ''", s);
            let output = Command::new("bash")
                .stderr(Stdio::inherit())
                .arg("-c")
                .arg(&command)
                .output()?;
            if output.status.success() {
                let s = String::from_utf8(output.stdout)?;
                if let Ok(v) = s.trim().parse::<f64>() {
                    *self = Self::Instant(v);
                }
            }
        }
        Ok(())
    }
}

impl FromStr for ColumnExpr {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(v) = s.parse::<f64>() {
            Ok(Self::Instant(v))
        } else if !s.contains('$') {
            Ok(Self::InstantExpr(s.to_string()))
        } else if let Some(i) = ColumnExpr::retrieve_column_index(s) {
            Ok(Self::Index(i))
        } else if let Some(name) = ColumnExpr::retrieve_column_name(s) {
            Ok(Self::Name(name.to_string()))
        } else {
            Ok(Self::ColumnExpr(s.to_string()))
        }
    }
}

pub struct DataPreprocessor {}

impl DataPreprocessor {
    fn build_datasheet_from_mlr<R>(
        mut rdr: R,
        has_header: bool,
        xexpr: &str,
        yexpr: &str,
        filter_expr: Option<&str>,
    ) -> anyhow::Result<Datasheet>
    where
        R: std::io::Read,
    {
        let filter_subcommand = filter_expr
            .map_or_else(|| "".to_string(), |s| format!(" filter '{}' +", s));

        let command = format!(
            "mlr --csv{}{} filter 'print ({}).\",\".({}); false'",
            if has_header { "" } else { " --hi" },
            filter_subcommand,
            xexpr,
            yexpr
        );
        log::info!("mlr command: {}", command);
        let mut child = Command::new("bash")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .arg("-c")
            .arg(&command)
            .spawn()?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        let read_thread = std::thread::spawn(move || {
            let ds = Datasheet::from_csv(
                stdout,
                true,
                ColumnExpr::Index(1),
                ColumnExpr::Index(2),
            )?;
            Ok::<_, anyhow::Error>(ds)
        });
        std::io::copy(&mut rdr, &mut stdin)?;
        drop(stdin);

        let ds = read_thread
            .join()
            .map_err(|e| anyhow::anyhow!("read thread panicked: {:?}", e))??;

        if let Err(e) = child.wait() {
            bail!("mlr command failed: {}", e)
        }

        Ok(ds)
    }

    pub fn preprocess<R>(
        rdr: R,
        fmt: DatasheetFormat,
        xexpr: &str,
        yexpr: &str,
        filter_expr: Option<&str>,
    ) -> anyhow::Result<Datasheet>
    where
        R: std::io::Read,
    {
        let xcol = ColumnExpr::from_str(xexpr)?;
        let ycol = ColumnExpr::from_str(yexpr)?;

        match fmt {
            DatasheetFormat::CSV { has_header } => {
                if matches!(xcol, ColumnExpr::ColumnExpr(_))
                    || matches!(ycol, ColumnExpr::ColumnExpr(_))
                    || filter_expr.is_some()
                {
                    let mut ds = Self::build_datasheet_from_mlr(
                        rdr,
                        has_header,
                        xexpr,
                        yexpr,
                        filter_expr,
                    )?;
                    ds.x.set_name(xexpr.to_string());
                    ds.y.set_name(yexpr.to_string());
                    Ok(ds)
                } else {
                    Datasheet::from_csv(rdr, has_header, xcol, ycol)
                }
            }
            DatasheetFormat::SPLNK => {
                bail!("SPLNK format cannot be preprocessed")
            }
        }
    }
}
