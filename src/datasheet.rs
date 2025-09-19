use core::panic;
use std::{
    io::{Read, Write},
    str::FromStr,
};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::preprocess::ColumnExpr;

#[derive(Debug, Clone, Default)]
pub struct Column {
    name: String,
    data: Vec<f64>,
    sorted: bool,
}

impl Column {
    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &f64> {
        self.data.iter()
    }

    pub fn new(name: String, data: Vec<f64>, sorted: bool) -> Self {
        Self { name, data, sorted }
    }

    pub fn is_unique(&self) -> bool {
        self.data
            .iter()
            .zip(self.data[1..].iter())
            .all(|(a, b)| a.is_finite() && b.is_finite() && a != b)
    }

    pub fn is_sortable(&self) -> bool {
        self.data.iter().all(|x| x.is_finite())
    }

    pub fn sort(&mut self) -> Result<()> {
        if !self.sorted {
            if self.is_sortable() {
                bail!("{} contains INF/NAN.", self.name);
            }
            self.data.sort_by(|a, b| a.partial_cmp(b).unwrap());
            self.sorted = true;
        }
        Ok(())
    }
}

// Datasheet: The x, y columns to be used for subsequent processing
#[derive(Debug, Clone, Default)]
pub struct Datasheet {
    pub x: Column,
    pub y: Column,
}

#[derive(
    Debug, Clone, strum::Display, strum::EnumString, Serialize, Deserialize,
)]
pub enum DatasheetFormat {
    #[strum(serialize = "csv", to_string = "csv({has_header})")]
    CSV { has_header: bool },
    #[strum(serialize = "lnk")]
    SPLNK,
}

impl Default for DatasheetFormat {
    fn default() -> Self {
        Self::CSV { has_header: true }
    }
}

impl DatasheetFormat {
    fn set_has_header(&mut self, has_header: bool) {
        match self {
            Self::CSV { has_header: h } => {
                *h = has_header;
            }
            Self::SPLNK { .. } => {}
        }
    }

    pub fn new_raw(fmt_str: &str, has_header: bool) -> Result<Self> {
        let mut fmt = Self::from_str(fmt_str)?;
        fmt.set_has_header(has_header);
        Ok(fmt)
    }

    pub fn has_header(&self) -> Option<bool> {
        match self {
            Self::CSV { has_header } => Some(*has_header),
            Self::SPLNK { .. } => None,
        }
    }
}

#[derive(strum::Display, Clone, Copy)]
pub enum ColumnID {
    X,
    Y,
}

impl Datasheet {
    pub fn from_csv<R>(
        csv_source: R,
        has_header: bool,
        mut xcol: ColumnExpr,
        mut ycol: ColumnExpr,
    ) -> Result<Self>
    where
        R: Read,
    {
        xcol.evaluate()?;
        ycol.evaluate()?;

        if matches!(xcol, ColumnExpr::ColumnExpr(_))
            || matches!(ycol, ColumnExpr::ColumnExpr(_))
        {
            bail!(
                "Cannot process column expressions. (x: {}, y: {})",
                xcol,
                ycol
            );
        }
        if !has_header
            && (matches!(xcol, ColumnExpr::Name(_))
                || matches!(ycol, ColumnExpr::Name(_)))
        {
            bail!(
                "Required column names but input does not have header. (x: {}, y: {}).",
                xcol,
                ycol,
            );
        }

        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(has_header)
            .from_reader(csv_source);

        if has_header {
            let headers = rdr.headers().unwrap();
            if let ColumnExpr::Name(name) = &xcol {
                let xindex =
                    headers.iter().position(|s| s == name).ok_or_else(
                        || anyhow!("Column {} not found in header.", name),
                    )?;
                xcol = ColumnExpr::Index(xindex);
            }
            if let ColumnExpr::Name(name) = &ycol {
                let yindex =
                    headers.iter().position(|s| s == name).ok_or_else(
                        || anyhow!("Column {} not found in header.", name),
                    )?;
                ycol = ColumnExpr::Index(yindex);
            }
        }

        let (x, y) = rdr.records().enumerate().try_fold(
            (Vec::new(), Vec::new()),
            |(mut x, mut y), (i, record)| {
                let record = record.map_err(|e| {
                    anyhow!("Failed to read record #{}: {}", i, e)
                })?;
                match &xcol {
                    ColumnExpr::Index(xindex) => {
                        x.push(record[*xindex - 1].parse::<f64>().map_err(
                            |e| anyhow!("Invalid x value in record #{i}: {e}"),
                        )?);
                    }
                    ColumnExpr::Instant(val) => {
                        x.push(*val);
                    }
                    _ => panic!("Invalid x column expression {}.", xcol),
                }
                match &ycol {
                    ColumnExpr::Index(yindex) => {
                        y.push(record[*yindex - 1].parse::<f64>().map_err(
                            |e| anyhow!("Invalid y value in record #{i}: {e}"),
                        )?);
                    }
                    ColumnExpr::Instant(val) => {
                        y.push(*val);
                    }
                    _ => panic!("Invalid y column expression {}.", ycol),
                }
                Ok::<_, anyhow::Error>((x, y))
            },
        )?;

        let xname = if let ColumnExpr::Index(i) = &xcol {
            rdr.headers().unwrap()[*i - 1].to_string()
        } else {
            "1".to_string()
        };

        let yname = if let ColumnExpr::Index(i) = &ycol {
            rdr.headers().unwrap()[*i - 1].to_string()
        } else {
            "2".to_string()
        };

        Ok(Self::new(
            Column::new(xname, x, false),
            Column::new(yname, y, false),
        ))
    }

    pub fn new(x: Column, y: Column) -> Self {
        Self { x, y }
    }

    pub fn to_csv<W: Write>(
        &self,
        write_header: bool,
        writer: W,
    ) -> Result<()> {
        let mut writer = csv::WriterBuilder::new().from_writer(writer);
        if write_header {
            writer
                .write_record(&[&self.x.name, &self.y.name])
                .map_err(|e| anyhow!("Failed to write headers: {}", e))?;
        }
        self.x
            .data
            .iter()
            .zip(self.y.data.iter())
            .try_for_each(|(x, y)| {
                writer
                    .write_record(&[x.to_string(), y.to_string()])
                    .map_err(|e| anyhow!("Failed to write record: {}", e))
            })?;

        writer
            .flush()
            .map_err(|e| anyhow!("Failed to flush: {}", e))?;
        Ok(())
    }

    pub fn exchange_column(&mut self) {
        std::mem::swap(&mut self.x, &mut self.y);
    }

    pub fn is_unique(&self, col: ColumnID) -> bool {
        match col {
            ColumnID::X => self.x.is_unique(),
            ColumnID::Y => self.y.is_unique(),
        }
    }

    pub fn is_sorted(&self, col: ColumnID) -> bool {
        match col {
            ColumnID::X => self.x.sorted,
            ColumnID::Y => self.y.sorted,
        }
    }

    // sort the datasheet (all columns) by the specified column
    // (incremental order)
    pub fn sort(&mut self, col: ColumnID) -> Result<()> {
        if self.is_sorted(col) {
            return Ok(());
        }

        let (sort_col, standby_col) = match col {
            ColumnID::X => (&mut self.x, &mut self.y),
            ColumnID::Y => (&mut self.y, &mut self.x),
        };

        if !sort_col.is_sortable() {
            bail!("Column {} ({}) contains INF/NAN.", col, sort_col.name);
        }

        let mut pair_vec = sort_col
            .data
            .drain(..)
            .zip(standby_col.data.drain(..))
            .collect::<Vec<_>>();
        pair_vec.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

        let (new_sort_col, new_standby_col) = pair_vec.into_iter().unzip();

        sort_col.data = new_sort_col;
        standby_col.data = new_standby_col;
        sort_col.sorted = true;
        standby_col.sorted = false;

        Ok(())
    }
}
