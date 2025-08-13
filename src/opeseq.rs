// Implementation of operators and the interpretation of operation sequence

use std::io::Write;

use crate::datasheet::{DataSheetFormat, Datasheet};
use anyhow::{Result, anyhow};

// Internal representation of operators, no associated functionalities
#[derive(Debug)]
struct Op {
    op: char,
    arg: Vec<f64>,
}

impl Op {
    // try to retrieve operator and its argument from the beginning of the
    // string, returns the operator and number of characters consumed
    fn from_str(s: &str) -> Result<(Self, usize)> {
        // operators are alphabets
        let op = match s.chars().nth(0) {
            Some(c @ 'a'..='z') => c,
            Some(c @ 'A'..='Z') => c,
            Some(c) => return Err(anyhow!("Non-alphabetic operator '{}'", c)),
            None => return Err(anyhow!("Empty string")),
        };
        // arguments are comma-separated numbers that follows operators
        let (arg, argstr_len) = match s[1..]
            .find(|c: char| char::is_ascii_alphabetic(&c))
            .unwrap_or(s.len() - 1)
        {
            0 => (vec![], 0),
            i => (
                s[1..1 + i]
                    .split(',')
                    .map(|s| s.parse::<f64>().map_err(|e| anyhow!("{}", e)))
                    .collect::<Result<Vec<f64>, anyhow::Error>>()?,
                i,
            ),
        };

        Ok((Self { op, arg }, 1 + argstr_len))
    }
}

// Operator trait
pub trait Transformer {
    // apply the operator to the datasheet, returns the transformed datasheet
    // WARNING: `apply` should NEVER fail!
    fn apply(&self, ds: Datasheet, x: usize, y: usize) -> Result<Datasheet>;
    // return the string representation of the operator
    fn to_string(&self) -> String;
    // generate standardized column names for the transformed datasheet,
    // returns a Vec<String> that contains the new column names to be used as
    // Datasheet.headers
    fn get_converted_column_names(
        &self,
        xname: &str,
        yname: &str,
    ) -> (String, String);
}

fn name_pair_to_column_header(name_pair: (String, String)) -> Vec<String> {
    vec![name_pair.0, name_pair.1]
}

// CDF operator
pub struct CDFOperator {}

impl Transformer for CDFOperator {
    fn apply(&self, ds: Datasheet, x: usize, y: usize) -> Result<Datasheet> {
        // use the old y value as the new x value since y value would be used
        // to compute the CDF function
        let mut xval = ds.columns[y].to_owned();
        if !Datasheet::is_sortable(&xval) {
            return Err(anyhow!("y column contains INF/NAN."));
        }
        xval.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let yval = (1..=xval.len())
            .map(|i| i as f64 / xval.len() as f64)
            .collect::<Vec<_>>();
        let headers = name_pair_to_column_header(
            self.get_converted_column_names(&ds.headers[x], &ds.headers[y]),
        );
        let data = vec![xval, yval];
        Ok(Datasheet::new(headers, data, Some(x)))
    }
    fn to_string(&self) -> String {
        "c".to_string()
    }
    fn get_converted_column_names(
        &self,
        _: &str,
        yname: &str,
    ) -> (String, String) {
        (yname.to_string(), "CDF".to_string())
    }
}

impl CDFOperator {
    pub fn new() -> Self {
        Self {}
    }
}

pub struct DerivationOperator {
    window: f64,
}

impl DerivationOperator {
    pub fn new(window: f64) -> Result<Self> {
        if !window.is_finite() || window <= 0.0 {
            Err(anyhow!("Window value must be a positive finite number"))
        } else {
            Ok(Self { window })
        }
    }
}

impl Transformer for DerivationOperator {
    fn apply(
        &self,
        mut ds: Datasheet,
        x: usize,
        y: usize,
    ) -> Result<Datasheet> {
        ds.sort(x)?;
        if !ds.is_unique(x)? {
            return Err(anyhow!("Column {} contains duplicated values.", x));
        }
        let xval = &ds.columns[x];
        let yval = &ds.columns[y];
        let (xval, yval) = xval
            .iter()
            .zip(yval.iter())
            .scan((None, None), |(start_x, start_y), (&x, &y)| match start_x {
                None => {
                    start_x.replace(x);
                    start_y.replace(y);
                    Some(None)
                }
                Some(x0) => {
                    if *x0 + self.window > x {
                        Some(None)
                    } else {
                        let derivation = (y - start_y.unwrap()) / (x - *x0);
                        start_x.replace(x);
                        start_y.replace(y);
                        Some(Some((x, derivation)))
                    }
                }
            })
            .fold((Vec::new(), Vec::new()), |(mut xval, mut yval), pair| {
                if let Some((x, y)) = pair {
                    xval.push(x);
                    yval.push(y);
                }
                (xval, yval)
            });

        let headers = name_pair_to_column_header(
            self.get_converted_column_names(&ds.headers[x], &ds.headers[y]),
        );
        let data = vec![xval, yval];
        Ok(Datasheet::new(headers, data, Some(x)))
    }
    fn to_string(&self) -> String {
        format!("d{}", self.window)
    }
    fn get_converted_column_names(
        &self,
        xname: &str,
        yname: &str,
    ) -> (String, String) {
        (
            xname.to_string(),
            format!("{}:Derivation", yname.to_string()),
        )
    }
}

pub struct IntegralOperator {}

impl IntegralOperator {
    pub fn new() -> Self {
        Self {}
    }
}

impl Transformer for IntegralOperator {
    fn apply(
        &self,
        mut ds: Datasheet,
        x: usize,
        y: usize,
    ) -> Result<Datasheet> {
        ds.sort(x)?;
        if !ds.is_unique(x)? {
            return Err(anyhow!("Column {} contains duplicated values.", x));
        }

        let xval = &ds.columns[x];
        let yval = &ds.columns[y];
        let yval = yval
            .iter()
            .scan(0.0, |acc, &y| {
                *acc += y;
                Some(*acc)
            })
            .collect::<Vec<_>>();
        let headers = name_pair_to_column_header(
            self.get_converted_column_names(&ds.headers[x], &ds.headers[y]),
        );
        let data = vec![xval.to_owned(), yval];
        Ok(Datasheet::new(headers, data, Some(x)))
    }
    fn to_string(&self) -> String {
        "i".to_string()
    }
    fn get_converted_column_names(
        &self,
        xname: &str,
        yname: &str,
    ) -> (String, String) {
        (xname.to_string(), format!("{}:Integral", yname.to_string()))
    }
}

pub struct MergeOperator {}

impl MergeOperator {
    pub fn new() -> Self {
        Self {}
    }
}

impl Transformer for MergeOperator {
    fn apply(&self, ds: Datasheet, x: usize, y: usize) -> Result<Datasheet> {
        let headers = name_pair_to_column_header(
            self.get_converted_column_names(&ds.headers[x], &ds.headers[y]),
        );
        let mut prev_x = None;
        let mut acc = 0.0;
        let mut xval = Vec::new();
        let mut yval = Vec::new();
        ds.columns[x]
            .iter()
            .zip(ds.columns[y].iter())
            .for_each(|(x, y)| {
                if prev_x.is_none() {
                    prev_x = Some(*x);
                    acc = *y;
                } else if prev_x.as_ref().unwrap() == x {
                    acc += *y;
                } else {
                    xval.push(prev_x.unwrap());
                    yval.push(acc);
                    prev_x = Some(*x);
                    acc = *y;
                }
            });
        if prev_x.is_some() {
            xval.push(prev_x.unwrap());
            yval.push(acc);
        }

        let data = vec![xval, yval];
        Ok(Datasheet::new(headers, data, None))
    }
    fn to_string(&self) -> String {
        "m".to_string()
    }
    fn get_converted_column_names(
        &self,
        xname: &str,
        yname: &str,
    ) -> (String, String) {
        (xname.to_string(), format!("{}:Merge", yname.to_string()))
    }
}

pub struct RotateOperator {}

impl RotateOperator {
    pub fn new() -> Self {
        Self {}
    }
}

impl Transformer for RotateOperator {
    fn apply(
        &self,
        mut ds: Datasheet,
        x: usize,
        y: usize,
    ) -> Result<Datasheet> {
        ds.exchange_column(x, y)?;
        Ok(ds)
    }
    fn to_string(&self) -> String {
        "r".to_string()
    }
    fn get_converted_column_names(
        &self,
        xname: &str,
        yname: &str,
    ) -> (String, String) {
        (yname.to_string(), xname.to_string())
    }
}

pub struct StepOperator {}

impl StepOperator {
    pub fn new() -> Self {
        Self {}
    }
}

impl Transformer for StepOperator {
    fn apply(&self, ds: Datasheet, x: usize, y: usize) -> Result<Datasheet> {
        let yval = &ds.columns[y];
        let yval = yval
            .iter()
            .zip(yval.iter().skip(1))
            .map(|(a, b)| b - a)
            .collect::<Vec<_>>();
        let headers = name_pair_to_column_header(
            self.get_converted_column_names(&ds.headers[x], &ds.headers[y]),
        );
        let data = vec![ds.columns[x][1..].iter().cloned().collect(), yval];
        Ok(Datasheet::new(headers, data, None))
    }
    fn to_string(&self) -> String {
        "s".to_string()
    }
    fn get_converted_column_names(
        &self,
        xname: &str,
        yname: &str,
    ) -> (String, String) {
        (xname.to_string(), format!("{}:Step", yname.to_string()))
    }
}

pub struct SortOperator {}

impl SortOperator {
    pub fn new() -> Self {
        Self {}
    }
}

impl Transformer for SortOperator {
    fn apply(
        &self,
        mut ds: Datasheet,
        x: usize,
        _: usize,
    ) -> Result<Datasheet> {
        ds.sort(x)?;
        Ok(ds)
    }
    fn to_string(&self) -> String {
        "o".to_string()
    }
    fn get_converted_column_names(
        &self,
        xname: &str,
        yname: &str,
    ) -> (String, String) {
        (xname.to_string(), yname.to_string())
    }
}

pub enum Transform {
    CDF(CDFOperator),
    Derivation(DerivationOperator),
    Integral(IntegralOperator),
    Merge(MergeOperator),
    Rotate(RotateOperator),
    Step(StepOperator),
    Sort(SortOperator),
}

impl Transform {
    fn from_op(op: Op) -> Result<Self> {
        match op.op {
            'c' => Ok(Self::CDF(CDFOperator::new())),
            'd' => {
                let res = Self::Derivation(DerivationOperator::new(
                    op.arg
                        .get(0)
                        .ok_or_else(|| {
                            anyhow!(
                                "Derivation: smooth window size not provided"
                            )
                        })?
                        .to_owned(),
                )?);
                Ok(res)
            }
            'i' => Ok(Self::Integral(IntegralOperator::new())),
            'm' => Ok(Self::Merge(MergeOperator::new())),
            'o' => Ok(Self::Sort(SortOperator::new())),
            'r' => Ok(Self::Rotate(RotateOperator::new())),
            's' => Ok(Self::Step(StepOperator::new())),
            _ => Err(anyhow!("Unknown transform operator {}", op.op)),
        }
    }
}

impl Transformer for Transform {
    fn apply(&self, ds: Datasheet, x: usize, y: usize) -> Result<Datasheet> {
        match self {
            Self::CDF(operator) => operator.apply(ds, x, y),
            Self::Derivation(operator) => operator.apply(ds, x, y),
            Self::Integral(operator) => operator.apply(ds, x, y),
            Self::Merge(operator) => operator.apply(ds, x, y),
            Self::Rotate(operator) => operator.apply(ds, x, y),
            Self::Step(operator) => operator.apply(ds, x, y),
            Self::Sort(operator) => operator.apply(ds, x, y),
        }
    }
    fn to_string(&self) -> String {
        match self {
            Self::CDF(operator) => operator.to_string(),
            Self::Derivation(operator) => operator.to_string(),
            Self::Integral(operator) => operator.to_string(),
            Self::Merge(operator) => operator.to_string(),
            Self::Rotate(operator) => operator.to_string(),
            Self::Step(operator) => operator.to_string(),
            Self::Sort(operator) => operator.to_string(),
        }
    }
    fn get_converted_column_names(
        &self,
        xname: &str,
        yname: &str,
    ) -> (String, String) {
        match self {
            Self::CDF(operator) => {
                operator.get_converted_column_names(xname, yname)
            }
            Self::Derivation(operator) => {
                operator.get_converted_column_names(xname, yname)
            }
            Self::Integral(operator) => {
                operator.get_converted_column_names(xname, yname)
            }
            Self::Merge(operator) => {
                operator.get_converted_column_names(xname, yname)
            }
            Self::Rotate(operator) => {
                operator.get_converted_column_names(xname, yname)
            }
            Self::Step(operator) => {
                operator.get_converted_column_names(xname, yname)
            }
            Self::Sort(operator) => {
                operator.get_converted_column_names(xname, yname)
            }
        }
    }
}

pub enum OutputFormat {
    DataSheet(DataSheetFormat),
    Plot,
}

impl OutputFormat {
    pub fn to_string(&self) -> String {
        match self {
            Self::DataSheet(format) => format.to_string(),
            Self::Plot => "plot".to_string(),
        }
    }
}

pub trait Dumper {
    fn apply(
        &self,
        ds: &Datasheet,
        format: &OutputFormat,
        w: &mut dyn Write,
    ) -> Result<()>;
    fn to_string(&self) -> String;
}

pub struct DataSheetDumper {
    target_code: char,
}

impl DataSheetDumper {
    pub fn new(target_code: char) -> Self {
        Self { target_code }
    }
}

impl Dumper for DataSheetDumper {
    fn apply(
        &self,
        ds: &Datasheet,
        format: &OutputFormat,
        w: &mut dyn Write,
    ) -> Result<()> {
        match format {
            OutputFormat::DataSheet(DataSheetFormat::CSV(write_header)) => {
                ds.to_csv(*write_header, &mut csv::Writer::from_writer(w))
            }
            _ => {
                Err(anyhow!("Illegal datasheet format {}", format.to_string()))
            }
        }
    }
    fn to_string(&self) -> String {
        self.target_code.to_string()
    }
}

pub enum Dump {
    DataSheet(DataSheetDumper),
    Plotter(Box<dyn Dumper>),
}

impl Dump {
    fn from_op(
        op: Op,
        plotter_factory: &dyn Fn() -> Box<dyn Dumper>,
    ) -> Result<Self> {
        match op.op {
            'P' => Ok(Self::Plotter(plotter_factory())),
            'C' | 'O' => Ok(Self::DataSheet(DataSheetDumper::new(op.op))),
            _ => Err(anyhow!("Unknown dump operator {}", op.op)),
        }
    }
}

impl Dumper for Dump {
    fn apply(
        &self,
        ds: &Datasheet,
        format: &OutputFormat,
        w: &mut dyn Write,
    ) -> Result<()> {
        match self {
            Self::DataSheet(operator) => operator.apply(ds, format, w),
            Self::Plotter(operator) => operator.apply(ds, format, w),
        }
    }
    fn to_string(&self) -> String {
        match self {
            Self::DataSheet(operator) => operator.to_string(),
            Self::Plotter(operator) => operator.to_string(),
        }
    }
}

pub enum Operator {
    Transform(Transform),
    Dump(Dump),
}

impl Operator {
    fn from_op(
        op: Op,
        plotter_factory: &dyn Fn() -> Box<dyn Dumper>,
    ) -> Result<Self> {
        if op.op.is_ascii_lowercase() {
            Ok(Self::Transform(Transform::from_op(op)?))
        } else if op.op.is_ascii_uppercase() {
            Ok(Self::Dump(Dump::from_op(op, plotter_factory)?))
        } else {
            Err(anyhow!("Unknown operator {}", op.op))
        }
    }
    pub fn to_string(&self) -> String {
        match self {
            Self::Transform(transform) => transform.to_string(),
            Self::Dump(dump) => dump.to_string(),
        }
    }
}

// Fake plotter that is only used in opseq string checkers
struct DumbPlotter {}

impl Dumper for DumbPlotter {
    fn apply(
        &self,
        _: &Datasheet,
        _: &OutputFormat,
        _: &mut dyn Write,
    ) -> Result<()> {
        Ok(())
    }
    fn to_string(&self) -> String {
        "P".to_string()
    }
}

// OpSeq: The major data structure that Plotter works on
// Represents a sequence of Operations, enables deserialization from string
pub struct OpSeq {
    pub ops: Vec<Operator>,
}

impl OpSeq {
    fn str_to_ops(s: &str) -> Result<Vec<Op>> {
        let mut ops = Vec::new();
        let len = s.len();
        let mut i = 0;
        while i < len {
            let (op, n) = Op::from_str(&s[i..])?;
            i += n;
            ops.push(op);
        }

        Ok(ops)
    }

    pub fn get_converted_column_names(
        xname: &str,
        yname: &str,
        opseq_str: &str,
    ) -> Result<(String, String)> {
        let ops = Self::new_dumb(opseq_str)?;
        let mut xname = xname.to_string();
        let mut yname = yname.to_string();
        for op in &ops.ops {
            if let Operator::Transform(transform) = op {
                let names =
                    transform.get_converted_column_names(&xname, &yname);
                xname = names.0;
                yname = names.1;
            }
        }
        Ok((xname, yname))
    }

    pub fn new(
        s: &str,
        plotter_factory: &dyn Fn() -> Box<dyn Dumper>,
    ) -> Result<Self> {
        let ops = Self::str_to_ops(s)?
            .into_iter()
            .map(|op| Operator::from_op(op, plotter_factory))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { ops })
    }

    pub fn new_dumb(s: &str) -> Result<Self> {
        Self::new(s, &|| Box::new(DumbPlotter {}))
    }

    pub fn check_string(s: &str) -> Result<()> {
        Self::new_dumb(s)?;
        Ok(())
    }

    pub fn to_string(&self, until_index: usize, include_dump: bool) -> String {
        self.ops
            .iter()
            .take(until_index + 1)
            .map(|op| match op {
                Operator::Transform(transform) => transform.to_string(),
                Operator::Dump(dump) => {
                    if include_dump {
                        dump.to_string()
                    } else {
                        "".to_string()
                    }
                }
            })
            .collect::<Vec<_>>()
            .join("")
    }

    pub fn match_split<'a>(
        opseq_str: &str,
        sub_opseq_str: &'a str,
    ) -> Option<(&'a str, usize)> {
        // associate the original string with character index numbers
        let opseq_iter = opseq_str
            .chars()
            .zip(0..opseq_str.len())
            .filter(|(c, _)| !c.is_ascii_uppercase());

        let sub_opseq_iter =
            sub_opseq_str.chars().filter(|c| !c.is_ascii_uppercase());

        let match_res = opseq_iter.zip(sub_opseq_iter).fold(
            Some(0),
            |res, ((c, i), sc)| {
                if res.is_some() && c == sc {
                    Some(i + 1)
                } else {
                    None
                }
            },
        );
        if let Some(split_pos) = match_res {
            Some((sub_opseq_str, split_pos))
        } else {
            None
        }
    }
}
