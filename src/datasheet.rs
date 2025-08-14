use std::{collections::HashMap, io::Write};

use crate::{
    cachefile::{StateCacheReader, state_cache_filename},
    opeseq::OpSeq,
};
use anyhow::{Result, anyhow};
use log::{info, trace};

// Datasheet: The unified internal data structure of spreadsheets.
// All types of spreadsheets are converted to this data structure upon input
// and converted back to specified type upon output.
// Note that Datasheets are stored in columns.
#[derive(Debug)]
pub struct Datasheet {
    pub headers: Vec<String>,
    pub rev_headers: HashMap<String, usize>,
    pub columns: Vec<Vec<f64>>,
    sorted_by: Option<usize>,
}

#[derive(Clone)]
pub enum DataSheetFormat {
    CSV(bool),
    SPLNK(String),
}

impl DataSheetFormat {
    fn new_raw_format(fmt: &str, has_header: bool) -> Result<Self> {
        match fmt {
            "csv" => Ok(Self::CSV(has_header)),
            _ => {
                return Err(anyhow!("Unknown datasheet format: {}", fmt));
            }
        }
    }

    // construct a DataSheetFormat from command line arguments
    pub fn new(fmt: &str, has_header: bool, opseq_str: &str) -> Result<Self> {
        if fmt == "lnk" {
            Ok(Self::SPLNK(opseq_str.to_string()))
        } else {
            Self::new_raw_format(fmt, has_header)
        }
    }

    // read from a SPLNK file. note that link files may not refer to SPLNK
    // files, and therefore opseq_str must be empty.
    pub fn from_str(line: &str) -> Result<Self> {
        let mut parts = line.splitn(2, ' ');
        let fmt = parts.next().unwrap();
        let has_header = parts
            .next()
            .unwrap()
            .parse::<bool>()
            .map_err(|e| anyhow!("Error parsing has_header: {}", e))?;
        Self::new(fmt, has_header, "")
    }

    pub fn to_string(&self) -> String {
        match self {
            Self::CSV(has_header) => {
                format!("csv {}", has_header)
            }
            Self::SPLNK(opseq_str) => format!("lnk({})", opseq_str),
        }
    }

    pub fn get_fmt_str(&self) -> String {
        match self {
            Self::CSV(_) => "csv".to_string(),
            Self::SPLNK(_) => "lnk".to_string(),
        }
    }

    pub fn has_header(&self) -> bool {
        match self {
            Self::CSV(has_header) => *has_header,
            Self::SPLNK(_) => true,
        }
    }
}

pub struct CacheLoadInfo {
    pub xexpr: String,
    pub yexpr: String,
    pub opseq_skip_len: usize,
}

impl Datasheet {
    fn set_column_name(&mut self, index: usize, name: String) {
        self.headers[index] = name.clone();
        self.rev_headers.insert(name, index);
    }

    // returns the datasheet and the number of characters to skip in the
    // operator sequence string (if reading a cache file)
    pub fn read(
        fmt: &DataSheetFormat,
        filename: &str,
    ) -> Result<(Datasheet, Option<CacheLoadInfo>)> {
        match fmt {
            DataSheetFormat::CSV(has_header) => {
                let csv_content = std::fs::read_to_string(filename)?;
                let ds = Datasheet::from_csv(&csv_content, *has_header)?;
                Ok((ds, None))
            }
            DataSheetFormat::SPLNK(opseq_str) => {
                let sc = StateCacheReader::read(filename)?;
                // find the last cache file that matches the operation sequence
                match sc
                    .opseq_cache
                    .iter()
                    .rev()
                    .find_map(|s| OpSeq::match_split(opseq_str, &s))
                {
                    Some((matched_cache_str, opseq_skip_len)) => {
                        let best_cache_filename = sc.dir.full_file_name(
                            &state_cache_filename(matched_cache_str),
                        );
                        info!(
                            "Matched cache file: {}, skip_len {}",
                            best_cache_filename, opseq_skip_len
                        );
                        let mut ds = Datasheet::read(
                            &sc.header.ds_out_format,
                            &best_cache_filename,
                        )?
                        .0;
                        let (xname, yname) = OpSeq::get_converted_column_names(
                            &sc.header.xexpr,
                            &sc.header.yexpr,
                            &opseq_str[0..opseq_skip_len],
                        )?;
                        ds.set_column_name(0, xname);
                        ds.set_column_name(1, yname);
                        Ok((
                            ds,
                            Some(CacheLoadInfo {
                                xexpr: sc.header.xexpr.clone(),
                                yexpr: sc.header.yexpr.clone(),
                                opseq_skip_len,
                            }),
                        ))
                    }
                    None => {
                        // cache not available, restart processing from the
                        // original spreadsheet file
                        let ds = Datasheet::read(
                            &sc.header.ds_in_format,
                            &sc.header.ds_path,
                        )?
                        .0;
                        Ok((ds, None))
                    }
                }
            }
        }
    }

    pub fn new(
        headers: Vec<String>,
        columns: Vec<Vec<f64>>,
        sorted_by: Option<usize>,
    ) -> Self {
        let rev_headers = headers
            .iter()
            .enumerate()
            // the collect() method overwrites the previous value if the key
            // is the same, so we reverse the order to make sure the first
            // header is the one that is used.
            .rev()
            .map(|(i, s)| (s.clone(), i))
            .collect();
        trace!("headers: {:?}", headers);
        trace!("rev_headers: {:?}", rev_headers);
        Self {
            headers,
            rev_headers,
            columns,
            sorted_by,
        }
    }

    // read a header-less csv file
    fn read_csv_into_columns(
        rdr: &mut csv::Reader<&[u8]>,
    ) -> Result<Vec<Vec<f64>>> {
        let mut columns = Vec::new();
        let mut row = 1;
        for result in rdr.deserialize() {
            let record: Vec<String> = result.map_err(|e| {
                anyhow!("Failed to read record #{}: {}", row, e)
            })?;
            if columns.is_empty() {
                for _ in 0..record.len() {
                    columns.push(Vec::new());
                }
            }
            for i in 0..record.len() {
                let f64val = record[i].parse::<f64>().unwrap_or(0.0);
                columns[i].push(f64val);
            }
            row += 1;
        }
        Ok(columns)
    }

    fn from_csv_without_header(csv: &str) -> Result<Self> {
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(false)
            .from_reader(csv.as_bytes());
        let mut headers = Vec::new();
        let mut rev_headers = HashMap::new();
        let columns = Self::read_csv_into_columns(&mut rdr)?;
        // use column numbers if we do not have headers by default
        for i in 1..columns.len() {
            headers.push(format!("{}", i));
            rev_headers.insert(format!("{}", i), i - 1);
        }
        Ok(Self {
            headers,
            rev_headers,
            columns,
            sorted_by: None,
        })
    }

    fn from_csv_with_header(csv: &str) -> Result<Self> {
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_reader(csv.as_bytes());
        let headers = rdr
            .headers()
            .map_err(|e| anyhow!("Failed to read headers: {}", e))?
            .iter()
            .map(|s| s.to_string())
            .collect();
        let columns = Self::read_csv_into_columns(&mut rdr)?;
        Ok(Self::new(headers, columns, None))
    }

    pub fn from_csv(csv: &str, has_header: bool) -> Result<Self> {
        if has_header {
            Self::from_csv_with_header(csv)
        } else {
            Self::from_csv_without_header(csv)
        }
    }

    pub fn to_csv<W: Write>(
        &self,
        write_header: bool,
        writer: &mut csv::Writer<W>,
    ) -> Result<()> {
        if write_header {
            writer
                .write_record(&self.headers)
                .map_err(|e| anyhow!("Failed to write headers: {}", e))?;
        }
        let rowcnt = self.columns[0].len();
        let colcnt = self.columns.len();
        for i in 0..rowcnt {
            for j in 0..colcnt {
                writer
                    .write_field(self.columns[j][i].to_string())
                    .map_err(|e| anyhow!("Failed to write record: {}", e))?;
                trace!(
                    "Write field ({}, {}): {}",
                    i,
                    j,
                    self.columns[j][i].to_string()
                );
            }
            writer
                .write_record(None::<&[u8]>)
                .map_err(|e| anyhow!("Failed to write record: {}", e))?;
        }
        writer
            .flush()
            .map_err(|e| anyhow!("Failed to flush: {}", e))?;
        Ok(())
    }

    pub fn get_column_index(&self, name: &str) -> Option<usize> {
        self.rev_headers.get(name).map(|x| *x)
    }

    pub fn is_sortable(col: &Vec<f64>) -> bool {
        col.iter().all(|x| x.is_finite())
    }

    pub fn exchange_column(&mut self, i: usize, j: usize) -> Result<()> {
        self.columns.swap(i, j);
        self.rev_headers.insert(self.headers[i].clone(), j);
        self.rev_headers.insert(self.headers[j].clone(), i);
        self.headers.swap(i, j);
        Ok(())
    }

    pub fn is_unique(&self, col: usize) -> Result<bool> {
        match self.sorted_by {
            Some(sorted_col) => {
                if sorted_col == col {
                    Ok(self.columns[col]
                        .iter()
                        .zip(self.columns[col][1..].iter())
                        .all(|(a, b)| a != b))
                } else {
                    Err(anyhow!("Column {} is not sorted.", col))
                }
            }
            None => Err(anyhow!("Column {} is not sorted.", col)),
        }
    }

    // sort the datasheet (all columns) by the specified column
    // (incremental order)
    pub fn sort(&mut self, col: usize) -> Result<()> {
        if let Some(sorted_by_col) = self.sorted_by {
            if sorted_by_col == col {
                return Ok(());
            }
        }

        if !Datasheet::is_sortable(&self.columns[col]) {
            return Err(anyhow!(
                "Column {} contains INF/NAN.",
                col.to_string()
            ));
        }

        let mut col_with_index =
            self.columns[col].iter().enumerate().collect::<Vec<_>>();
        col_with_index.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

        let sorted_index =
            col_with_index.iter().map(|x| x.0).collect::<Vec<_>>();

        for i in 0..self.columns.len() {
            let mut sorted_column = Vec::new();
            for index in sorted_index.iter() {
                sorted_column.push(self.columns[i][*index]);
            }
            self.columns[i] = sorted_column;
        }

        self.sorted_by.replace(col);

        Ok(())
    }
}
