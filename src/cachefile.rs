// implements the splnk cache file format

use anyhow::{Context, Result, anyhow};
use log::debug;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};

use crate::{
    commons::{ProtectedDir, to_absolute_path},
    datasheet::DataSheetFormat,
    opeseq::OpSeq,
};

pub struct StateCacheHeader {
    pub ds_path: String,
    pub xexpr: String,
    pub yexpr: String,
    pub ds_in_format: DataSheetFormat,
    pub ds_out_format: DataSheetFormat,
}

fn read_line_into_string(reader: &mut BufReader<File>) -> Result<String> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.ends_with('\n') {
        line.pop();
    }
    Ok(line)
}

impl StateCacheHeader {
    // build StateCacheHeader from command line options
    pub fn new(
        ds_path: &str,
        xexpr: &str,
        yexpr: &str,
        ds_in_format: &str,
        input_has_header: bool,
        ds_out_format: &str,
        opseq_str: &str,
    ) -> Result<Self> {
        Ok(Self {
            ds_path: to_absolute_path(&ds_path)?,
            xexpr: xexpr.to_string(),
            yexpr: yexpr.to_string(),
            ds_in_format: DataSheetFormat::new(
                ds_in_format,
                input_has_header,
                opseq_str,
            )?,
            ds_out_format: DataSheetFormat::new(ds_out_format, true, "")?,
        })
    }

    pub fn from_reader(reader: &mut BufReader<File>) -> Result<Self> {
        let ds_path = read_line_into_string(reader)?;
        let xexpr = read_line_into_string(reader)?;
        let yexpr = read_line_into_string(reader)?;
        let ds_in_format = read_line_into_string(reader)?;
        let ds_out_format = read_line_into_string(reader)?;

        let ds_in_format = DataSheetFormat::from_str(&ds_in_format)?;
        let ds_out_format = DataSheetFormat::from_str(&ds_out_format)?;

        Ok(Self {
            ds_path,
            xexpr,
            yexpr,
            ds_in_format,
            ds_out_format,
        })
    }
    pub fn to_string(&self) -> String {
        format!(
            "{}\n{}\n{}\n{}\n{}",
            self.ds_path,
            self.xexpr,
            self.yexpr,
            self.ds_in_format.to_string(),
            self.ds_out_format.to_string(),
        )
    }
}

pub struct StateCacheWriter {
    writer: BufWriter<File>,
}

impl StateCacheWriter {
    pub fn new(
        dir: &ProtectedDir,
        ds_path: &str,
        xexpr: &str,
        yexpr: &str,
        ds_in_format: &str,
        input_has_header: bool,
        ds_out_format: &str,
    ) -> Result<Self> {
        let mut scw = Self {
            writer: dir
                .create_output_file("splnk")
                .context("Failed to create output file")?,
        };
        let mut header = StateCacheHeader::new(
            ds_path,
            xexpr,
            yexpr,
            ds_in_format,
            input_has_header,
            ds_out_format,
            // recall that SPLNK files could not refer to another SPLNK source
            // file and thus the opseq_str is always empty
            "",
        )?
        .to_string();
        header.push('\n');
        scw.writer
            .write(header.as_bytes())
            .context("Failed to write header")?;
        scw.writer.flush()?;
        Ok(scw)
    }

    pub fn write_cache_metadata(
        &mut self,
        opseq: &OpSeq,
        until_index: usize,
    ) -> Result<()> {
        debug!(
            "Writing cache metadata: opseq {}, pos {}",
            opseq.to_string(until_index, true),
            until_index
        );
        let mut metadata = opseq.to_string(until_index, true);
        metadata.push('\n');
        self.writer.write(metadata.as_bytes())?;
        self.writer.flush()?;
        Ok(())
    }
}

pub struct StateCache {
    pub dir: ProtectedDir,
    pub header: StateCacheHeader,
    pub opseq_cache: Vec<String>,
}

pub struct StateCacheReader {}

impl StateCacheReader {
    pub fn read(path: &str) -> Result<StateCache> {
        let dir_path = Path::new(path).parent().ok_or_else(|| {
            anyhow!(
                "Failed to get parent directory of state cache file: {}",
                path
            )
        })?;
        let dir = ProtectedDir::from_path(dir_path)
            .context("Failed to open directory")?;

        let mut reader = BufReader::new(File::open(path)?);
        let header = StateCacheHeader::from_reader(&mut reader)?;
        let mut opseq_cache = Vec::new();
        for line in reader.lines() {
            let line = line.context("Failed to read line")?;
            if opseq_cache.is_empty()
                || line.starts_with(opseq_cache.last().unwrap())
            {
                OpSeq::check_string(&line)?;
                opseq_cache.push(line);
            } else {
                return Err(anyhow!(
                    "Bad state cache file: non-incremetal operation sequence: {}",
                    line
                ));
            }
        }

        Ok(StateCache {
            dir,
            header,
            opseq_cache,
        })
    }
}
