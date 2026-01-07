use std::{fmt::Display, str::FromStr};

use anyhow::bail;

#[derive(Debug, Clone)]
pub enum DataFormat {
    /// translates into `select * from '<input>'`
    Auto,
    /// translates into `select * from read_<format>('<input>')`
    Explicit(String),
}

impl Display for DataFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Explicit(fmt) => write!(f, "{fmt}"),
        }
    }
}

impl Default for DataFormat {
    fn default() -> Self {
        Self::Auto
    }
}

impl FromStr for DataFormat {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "auto" => Self::Auto,
            fmt => Self::Explicit(fmt.to_string()),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct DataInput {
    format: DataFormat,
    input: String,
    header: Option<bool>,
}

impl DataInput {
    fn format_check(
        format: &DataFormat,
        header: Option<bool>,
    ) -> anyhow::Result<()> {
        if header.is_some() {
            if let DataFormat::Explicit(fmt) = format {
                if fmt == "csv" || fmt == "xlsx" {
                    return Ok(());
                }
            }

            bail!("--header must be used with --format csv or --format xlsx");
        }
        Ok(())
    }

    pub fn new(
        format: DataFormat,
        input: String,
        header: Option<bool>,
    ) -> anyhow::Result<Self> {
        Self::format_check(&format, header)?;
        Ok(Self {
            format,
            input,
            header,
        })
    }

    pub fn to_sql(&self, table_name: &str) -> String {
        match self.format {
            DataFormat::Auto => format!(
                "CREATE TABLE {} AS SELECT * FROM '{}';\n",
                table_name, self.input
            ),
            DataFormat::Explicit(ref fmt) => {
                let header_opt = match self.header {
                    Some(true) => ", header=true",
                    Some(false) => ", header=false",
                    None => "",
                };

                format!(
                    "CREATE TABLE {} AS SELECT * FROM read_{}('{}'{});\n",
                    table_name, fmt, self.input, header_opt
                )
            }
        }
    }
}
