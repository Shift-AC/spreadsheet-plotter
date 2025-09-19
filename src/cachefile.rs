// implements the splnk cache file format

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
};

use crate::datasheet::{Datasheet, DatasheetFormat};

const MAGIC_CACHE_FILE_DELIMITER: &str =
    "ENDOFMETADATAENDOFMETADATAENDOFMETADATAENDOFMETADATAENDOFMETADATA";

#[derive(Serialize, Deserialize, Clone)]
pub struct StateCacheHeader {
    pub input_path: PathBuf,
    pub xexpr: String,
    pub yexpr: String,
    pub input_format: DatasheetFormat,
    pub output_format: DatasheetFormat,
    pub opstr: String,
}

fn read_line_into_string<R: BufRead>(reader: &mut R) -> Result<Option<String>> {
    let mut line = String::new();
    let read_len = reader.read_line(&mut line)?;
    if read_len == 0 {
        return Ok(None);
    }
    if line.ends_with('\n') {
        line.pop();
    }
    Ok(Some(line))
}

pub struct StateCache<'a> {
    pub header: StateCacheHeader,
    pub ds: Cow<'a, Datasheet>,
}

impl<'a> StateCache<'a> {
    pub fn from_reader<R: Read>(rdr: &mut R) -> Result<Self> {
        let mut rdr = BufReader::new(rdr);

        let mut header_str = String::new();
        loop {
            let line = read_line_into_string(&mut rdr)?;
            if line.is_none()
                || line.as_ref().unwrap() == MAGIC_CACHE_FILE_DELIMITER
            {
                break;
            }
            header_str.push_str(line.as_ref().unwrap());
            header_str.push('\n');
        }
        let header: StateCacheHeader = toml::from_str(&header_str)?;
        let mut csv_data = Vec::new();
        rdr.read_to_end(&mut csv_data)?;
        let ds = Datasheet::from_csv(
            &mut csv_data.as_slice(),
            header.input_format.has_header().unwrap(),
            crate::preprocess::ColumnExpr::Index(0),
            crate::preprocess::ColumnExpr::Index(1),
        )?;
        Ok(Self {
            header,
            ds: Cow::Owned(ds),
        })
    }

    pub fn write(&self, wtr: &mut dyn Write) -> Result<()> {
        let mut header_str = toml::to_string(&self.header)?;
        header_str.push('\n');
        header_str.push_str(MAGIC_CACHE_FILE_DELIMITER);
        header_str.push('\n');
        wtr.write_all(header_str.as_bytes())?;
        self.ds
            .to_csv(self.header.output_format.has_header().unwrap(), wtr)?;
        Ok(())
    }
}
