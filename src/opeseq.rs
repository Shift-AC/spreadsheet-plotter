// Implementation of operators and the interpretation of operation sequence

use std::{fmt::Display, io::Write};

use crate::{
    cachefile::{StateCache, StateCacheHeader},
    datasheet::{Column, ColumnID, Datasheet, DatasheetFormat},
};
use anyhow::{Result, anyhow, bail};

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
            Some(c) => bail!("Non-alphabetic operator '{}'", c),
            None => bail!("Empty string"),
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
pub trait Transformer: Display {
    // apply the operator to the datasheet, returns the transformed datasheet
    // WARNING: `apply` should NEVER fail!
    fn apply(&self, ds: Datasheet) -> Result<Datasheet>;
    // generate standardized column names for the transformed datasheet,
    // returns a Vec<String> that contains the new column names to be used as
    // Datasheet.headers
    fn get_converted_column_names(
        &self,
        xname: &str,
        yname: &str,
    ) -> (String, String);
}

// CDF operator
#[derive(Default)]
pub struct CDFOperator {}

impl Transformer for CDFOperator {
    fn apply(&self, ds: Datasheet) -> Result<Datasheet> {
        let (xname, yname) =
            self.get_converted_column_names(ds.x.get_name(), ds.y.get_name());
        // use the old y value as the new x value since y value would be used
        // to compute the CDF function
        let mut xcol = ds.y;
        xcol.sort()?;
        let yval = (1..=xcol.len())
            .map(|i| i as f64 / xcol.len() as f64)
            .collect::<Vec<_>>();
        let ycol = Column::new(yname.to_string(), yval, false);
        xcol.set_name(xname);
        Ok(Datasheet::new(xcol, ycol))
    }
    fn get_converted_column_names(
        &self,
        _: &str,
        yname: &str,
    ) -> (String, String) {
        (yname.to_string(), "CDF".to_string())
    }
}

impl Display for CDFOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "c")
    }
}

#[derive(Default)]
pub struct DerivationOperator {
    window: f64,
}

impl Display for DerivationOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "d{}", self.window)
    }
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
    fn apply(&self, mut ds: Datasheet) -> Result<Datasheet> {
        ds.sort(ColumnID::X)?;
        if !ds.is_unique(ColumnID::X) {
            bail!("Column {} contains duplicated values.", ColumnID::X);
        }
        let (xval, yval) = ds
            .x
            .iter()
            .zip(ds.y.iter())
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

        let (xname, yname) =
            self.get_converted_column_names(ds.x.get_name(), ds.y.get_name());
        let xcol = Column::new(xname.to_string(), xval, true);
        let ycol = Column::new(yname.to_string(), yval, false);

        Ok(Datasheet::new(xcol, ycol))
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

#[derive(Default)]
pub struct IntegralOperator {}

impl Transformer for IntegralOperator {
    fn apply(&self, mut ds: Datasheet) -> Result<Datasheet> {
        ds.sort(ColumnID::X)?;
        if !ds.is_unique(ColumnID::X) {
            bail!("Column {} contains duplicated values.", ColumnID::X);
        }

        let yval =
            ds.y.iter()
                .scan(0.0, |acc, &y| {
                    *acc += y;
                    Some(*acc)
                })
                .collect::<Vec<_>>();
        let (xname, yname) =
            self.get_converted_column_names(ds.x.get_name(), ds.y.get_name());
        let mut xcol = ds.x;
        xcol.set_name(xname);
        let ycol = Column::new(yname.to_string(), yval, false);
        Ok(Datasheet::new(xcol, ycol))
    }
    fn get_converted_column_names(
        &self,
        xname: &str,
        yname: &str,
    ) -> (String, String) {
        (xname.to_string(), format!("{}:Integral", yname.to_string()))
    }
}

impl Display for IntegralOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "i")
    }
}

#[derive(Default)]
pub struct MergeOperator {}

impl Transformer for MergeOperator {
    fn apply(&self, ds: Datasheet) -> Result<Datasheet> {
        let (xname, yname) =
            self.get_converted_column_names(ds.x.get_name(), ds.y.get_name());

        let (xval, yval): (Vec<_>, Vec<_>) =
            ds.x.iter()
                .zip(ds.y.iter())
                .scan((0.0, None), |(acc, prev_x), (x, y)| match prev_x {
                    None => {
                        prev_x.replace(*x);
                        *acc = *y;
                        Some(None)
                    }
                    Some(x0) => {
                        if *x0 == *x {
                            *acc += *y;
                            Some(None)
                        } else {
                            let this_acc = *acc;
                            *acc = *y;
                            Some(Some((prev_x.replace(*x).unwrap(), this_acc)))
                        }
                    }
                })
                .filter_map(|pair| pair)
                .unzip();

        let xcol = Column::new(xname.to_string(), xval, false);
        let ycol = Column::new(yname.to_string(), yval, false);
        Ok(Datasheet::new(xcol, ycol))
    }
    fn get_converted_column_names(
        &self,
        xname: &str,
        yname: &str,
    ) -> (String, String) {
        (xname.to_string(), format!("{}:Merge", yname.to_string()))
    }
}

impl Display for MergeOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "m")
    }
}

#[derive(Default)]
pub struct RotateOperator {}

impl Display for RotateOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "r")
    }
}

impl Transformer for RotateOperator {
    fn apply(&self, mut ds: Datasheet) -> Result<Datasheet> {
        ds.exchange_column();
        Ok(ds)
    }
    fn get_converted_column_names(
        &self,
        xname: &str,
        yname: &str,
    ) -> (String, String) {
        (yname.to_string(), xname.to_string())
    }
}

#[derive(Default)]
pub struct StepOperator {}

impl Display for StepOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "s")
    }
}

impl Transformer for StepOperator {
    fn apply(&self, ds: Datasheet) -> Result<Datasheet> {
        let yval =
            ds.y.iter()
                .zip(ds.y.iter().skip(1))
                .map(|(a, b)| b - a)
                .collect::<Vec<_>>();
        let (_, yname) =
            self.get_converted_column_names(ds.x.get_name(), ds.y.get_name());
        let xcol = ds.x;
        let ycol = Column::new(yname.to_string(), yval, false);
        Ok(Datasheet::new(xcol, ycol))
    }
    fn get_converted_column_names(
        &self,
        xname: &str,
        yname: &str,
    ) -> (String, String) {
        (xname.to_string(), format!("{}:Step", yname.to_string()))
    }
}

#[derive(Default)]
pub struct SortOperator {}

impl Display for SortOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "o")
    }
}

impl Transformer for SortOperator {
    fn apply(&self, mut ds: Datasheet) -> Result<Datasheet> {
        ds.sort(ColumnID::X)?;
        Ok(ds)
    }
    fn get_converted_column_names(
        &self,
        xname: &str,
        yname: &str,
    ) -> (String, String) {
        (xname.to_string(), yname.to_string())
    }
}

#[derive(strum::Display)]
pub enum Transform {
    #[strum(to_string = "{0}")]
    CDF(CDFOperator),
    #[strum(to_string = "{0}")]
    Derivation(DerivationOperator),
    #[strum(to_string = "{0}")]
    Integral(IntegralOperator),
    #[strum(to_string = "{0}")]
    Merge(MergeOperator),
    #[strum(to_string = "{0}")]
    Rotate(RotateOperator),
    #[strum(to_string = "{0}")]
    Step(StepOperator),
    #[strum(to_string = "{0}")]
    Sort(SortOperator),
}

impl Transform {
    fn from_op(op: Op) -> Result<Self> {
        match op.op {
            'c' => Ok(Self::CDF(CDFOperator::default())),
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
            'i' => Ok(Self::Integral(IntegralOperator::default())),
            'm' => Ok(Self::Merge(MergeOperator::default())),
            'o' => Ok(Self::Sort(SortOperator::default())),
            'r' => Ok(Self::Rotate(RotateOperator::default())),
            's' => Ok(Self::Step(StepOperator::default())),
            _ => Err(anyhow!("Unknown transform operator {}", op.op)),
        }
    }
}

impl Transformer for Transform {
    fn apply(&self, ds: Datasheet) -> Result<Datasheet> {
        match self {
            Self::CDF(operator) => operator.apply(ds),
            Self::Derivation(operator) => operator.apply(ds),
            Self::Integral(operator) => operator.apply(ds),
            Self::Merge(operator) => operator.apply(ds),
            Self::Rotate(operator) => operator.apply(ds),
            Self::Step(operator) => operator.apply(ds),
            Self::Sort(operator) => operator.apply(ds),
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

#[derive(strum::Display)]
pub enum OutputFormat {
    DataSheet(DatasheetFormat),
    Cache(StateCacheHeader),
    Plot,
}

pub trait Dumper: Display {
    fn apply(
        &self,
        ds: &Datasheet,
        format: &OutputFormat,
        w: &mut dyn Write,
    ) -> Result<()>;
}

#[derive(Default)]
pub struct DataSheetDumper {}

impl Display for DataSheetDumper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "O")
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
            OutputFormat::DataSheet(DatasheetFormat::CSV { has_header }) => {
                ds.to_csv(*has_header, w)
            }
            _ => Err(anyhow!("Illegal datasheet format {}", format)),
        }
    }
}

#[derive(Default)]
pub struct CacheDumper {}

impl Display for CacheDumper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "C")
    }
}

impl Dumper for CacheDumper {
    fn apply(
        &self,
        ds: &Datasheet,
        format: &OutputFormat,
        w: &mut dyn Write,
    ) -> Result<()> {
        match format {
            OutputFormat::Cache(header) => {
                let cache = StateCache {
                    header: header.clone(),
                    ds: std::borrow::Cow::Borrowed(&ds),
                };
                cache.write(w)
            }
            _ => Err(anyhow!("Illegal datasheet format {}", format)),
        }
    }
}

pub enum Dump {
    DataSheet(DataSheetDumper),
    Cache(CacheDumper),
    Plotter(Box<dyn Dumper>),
}

impl Dump {
    fn from_op<F>(op: Op, plotter_factory: F) -> Result<Self>
    where
        F: Fn() -> Box<dyn Dumper>,
    {
        match op.op {
            'P' => Ok(Self::Plotter(plotter_factory())),
            'C' => Ok(Self::Cache(CacheDumper::default())),
            'O' => Ok(Self::DataSheet(DataSheetDumper::default())),
            _ => Err(anyhow!("Unknown dump operator {}", op.op)),
        }
    }
}

impl Display for Dump {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DataSheet(operator) => write!(f, "{}", operator),
            Self::Cache(operator) => write!(f, "{}", operator),
            Self::Plotter(operator) => write!(f, "{}", operator),
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
            Self::Cache(operator) => operator.apply(ds, format, w),
            Self::Plotter(operator) => operator.apply(ds, format, w),
        }
    }
}

pub enum Operator {
    Transform(Transform),
    Dump(Dump),
}

impl Display for Operator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transform(transform) => write!(f, "{}", transform),
            Self::Dump(dump) => write!(f, "{}", dump),
        }
    }
}

impl Operator {
    fn from_op<F>(op: Op, plotter_factory: F) -> Result<Self>
    where
        F: Fn() -> Box<dyn Dumper>,
    {
        if op.op.is_ascii_lowercase() {
            Ok(Self::Transform(Transform::from_op(op)?))
        } else if op.op.is_ascii_uppercase() {
            Ok(Self::Dump(Dump::from_op(op, plotter_factory)?))
        } else {
            Err(anyhow!("Unknown operator {}", op.op))
        }
    }
}

// Fake plotter that is only used in opseq string checkers
#[derive(Default)]
struct DumbPlotter {}

impl Display for DumbPlotter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "P")
    }
}

impl Dumper for DumbPlotter {
    fn apply(
        &self,
        _: &Datasheet,
        _: &OutputFormat,
        _: &mut dyn Write,
    ) -> Result<()> {
        Ok(())
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

    pub fn opseq_matched_len(full_str: &str, match_str: &str) -> usize {
        // associate the original string with character index numbers
        let opseq_iter = full_str
            .chars()
            .enumerate()
            .filter(|(_, c)| !c.is_ascii_uppercase());

        let sub_opseq_iter =
            match_str.chars().filter(|c| !c.is_ascii_uppercase());

        opseq_iter
            .zip(sub_opseq_iter)
            .try_fold(
                0,
                |_, ((i, c), sc)| {
                    if c == sc { Ok(i + 1) } else { Err(()) }
                },
            )
            .unwrap_or(0)
    }
}
