// Implementation of operators and the interpretation of operation sequence

use std::{fmt::Display, str::FromStr};

use anyhow::{Result, anyhow, bail};
use strum::Display;

// Internal representation of operators, no associated functionalities
#[derive(Debug)]
pub struct Op {
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
            Some(c) => bail!("Non-alphabetic operator '{c}'"),
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
                    .map(|s| s.parse::<f64>().map_err(|e| anyhow!("{e}")))
                    .collect::<Result<Vec<f64>, anyhow::Error>>()?,
                i,
            ),
        };

        Ok((Self { op, arg }, 1 + argstr_len))
    }
}

pub struct OperateInfo {
    src_table: String,
    tmp_table_num: usize,
    x_name: String,
    y_name: String,
}

pub struct OperateResult {
    subquery: String,
    x_name: String,
    y_name: String,
}

pub trait Operator: std::fmt::Debug + Clone + Display + TryFrom<Op> {
    fn to_sql(&self, info: &OperateInfo) -> OperateResult;
    fn append_column_name(&self, name: &str) -> String {
        if !name.contains('-') {
            format!("{name}-{self}")
        } else {
            format!("{name}{self}")
        }
    }
}

macro_rules! declare_operator_no_param {
    ($op:ident) => {
        #[derive(Debug, Clone)]
        pub struct $op {}

        impl Display for $op {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(
                    f,
                    "{}",
                    stringify!($op)
                        .chars()
                        .next()
                        .unwrap()
                        .to_ascii_lowercase()
                )
            }
        }

        impl TryFrom<Op> for $op {
            type Error = anyhow::Error;

            fn try_from(op: Op) -> Result<Self> {
                let op_char = stringify!($op)
                    .chars()
                    .next()
                    .unwrap()
                    .to_ascii_lowercase();
                if op.op != op_char {
                    bail!(
                        "{} only accepts '{}' as operator",
                        stringify!($op),
                        op_char
                    );
                }
                Ok(Self {})
            }
        }
    };
}

#[derive(Debug, Clone)]
struct RelativeRange {
    left_window: f64,
    right_window: f64,
}

impl Display for RelativeRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.left_window == 0.0 && self.right_window == 0.0 {
            write!(f, "")
        } else if self.left_window == self.right_window {
            write!(f, "{}", self.left_window)
        } else {
            write!(f, "{},{}", self.left_window, self.right_window)
        }
    }
}

impl RelativeRange {
    fn from_args(args: &[f64]) -> anyhow::Result<Self> {
        let left_window = *args.first().unwrap_or(&0.0);
        let right_window = *args.get(1).unwrap_or(&left_window);
        if !left_window.is_finite()
            || !right_window.is_finite()
            || left_window < 0.0
            || right_window < 0.0
        {
            bail!("RelativeRange only accepts non-negative finite window size");
        }
        Ok(Self {
            left_window,
            right_window,
        })
    }

    fn generate_window_clause(&self) -> String {
        format!(
            "RANGE BETWEEN {} PRECEDING AND {} FOLLOWING",
            self.left_window, self.right_window
        )
    }
}

macro_rules! declare_operator_with_single_arg {
    ($op:ident, $arg_name:ident) => {
        #[derive(Debug, Clone)]
        pub struct $op($arg_name);

        impl Display for $op {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(
                    f,
                    "{}{}",
                    stringify!($op)
                        .chars()
                        .next()
                        .unwrap()
                        .to_ascii_lowercase(),
                    self.0
                )
            }
        }

        impl TryFrom<Op> for $op {
            type Error = anyhow::Error;

            fn try_from(op: Op) -> Result<Self> {
                let op_char = stringify!($op)
                    .chars()
                    .next()
                    .unwrap()
                    .to_ascii_lowercase();
                if op.op != op_char {
                    bail!(
                        "{} only accepts '{}' as operator",
                        stringify!($op),
                        op_char
                    );
                }
                let range = RelativeRange::from_args(&op.arg)?;
                Ok(Self(range))
            }
        }
    };
}
declare_operator_with_single_arg!(AverageOperator, RelativeRange);

impl Operator for AverageOperator {
    fn to_sql(&self, info: &OperateInfo) -> OperateResult {
        let x_name = info.x_name.to_string();
        let y_name = self.append_column_name(&info.y_name);

        OperateResult {
            subquery: format!(
                "t{} AS (SELECT \"{}\", avg(\"{}\") over w as \"{}\" FROM {} WINDOW w AS (ORDER BY \"{}\" {})",
                info.tmp_table_num,
                x_name,
                info.y_name,
                y_name,
                info.src_table,
                info.x_name,
                self.0.generate_window_clause(),
            ),
            x_name,
            y_name,
        }
    }
}

declare_operator_no_param!(CDFOperator);

impl Operator for CDFOperator {
    fn to_sql(&self, info: &OperateInfo) -> OperateResult {
        let x_name = info.y_name.to_string();
        let y_name = self.append_column_name(&info.y_name);

        OperateResult {
            subquery: format!(
                "t{} AS (SELECT \"{}\", cume_dist() OVER (ORDER BY \"{}\") AS \"{}\" FROM {} ORDER BY \"{}\")",
                info.tmp_table_num,
                info.y_name,
                info.y_name,
                y_name,
                info.src_table,
                info.y_name
            ),
            x_name,
            y_name,
        }
    }
}

declare_operator_with_single_arg!(DerivativeOperator, RelativeRange);

impl Operator for DerivativeOperator {
    fn to_sql(&self, info: &OperateInfo) -> OperateResult {
        let x_name = info.x_name.to_string();
        let y_name = self.append_column_name(&info.y_name);

        let window = if self.0.to_string() == "" {
            format!("ORDER BY \"{}\" ROWS 1 PRECEDING", info.x_name)
        } else {
            format!(
                "ORDER BY \"{}\" RANGE BETWEEN {} PRECEDING AND {} FOLLOWING",
                info.x_name, self.0.left_window, self.0.right_window
            )
        };

        OperateResult {
            subquery: format!(
                "t{} AS (SELECT \"{}\", (last_value(\"{}\") over w - first_value(\"{}\") over w) / (last_value(\"{}\") over w - first_value(\"{}\") over w) as \"{}\" FROM {} WINDOW w AS ({}))",
                info.tmp_table_num,
                info.x_name,
                info.y_name,
                info.y_name,
                info.x_name,
                info.x_name,
                y_name,
                info.src_table,
                window
            ),
            x_name,
            y_name,
        }
    }
}

declare_operator_no_param!(FilterFiniteOperator);

impl Operator for FilterFiniteOperator {
    fn to_sql(&self, info: &OperateInfo) -> OperateResult {
        let x_name = info.x_name.to_string();
        let y_name = self.append_column_name(&info.y_name);

        OperateResult {
            subquery: format!(
                "t{} AS (SELECT \"{}\", \"{}\" as \"{}\" FROM {} WHERE \"{}\" IS NOT NULL AND \"{}\" NOT IN ('-nan', 'nan', 'inf', '-inf'))",
                info.tmp_table_num,
                info.x_name,
                info.y_name,
                y_name,
                info.src_table,
                info.y_name,
                info.y_name,
            ),
            x_name,
            y_name,
        }
    }
}

declare_operator_no_param!(IntegralOperator);

impl Operator for IntegralOperator {
    fn to_sql(&self, info: &OperateInfo) -> OperateResult {
        let x_name = info.x_name.to_string();
        let y_name = self.append_column_name(&info.y_name);

        OperateResult {
            subquery: format!(
                "t{} AS (SELECT \"{}\", sum(\"{}\") over w as \"{}\" FROM {} WINDOW w AS (ORDER BY \"{}\"))",
                info.tmp_table_num,
                info.x_name,
                info.y_name,
                y_name,
                info.src_table,
                info.x_name,
            ),
            x_name,
            y_name,
        }
    }
}

declare_operator_no_param!(MergeOperator);

impl Operator for MergeOperator {
    fn to_sql(&self, info: &OperateInfo) -> OperateResult {
        let x_name = info.x_name.to_string();
        let y_name = self.append_column_name(&info.y_name);

        OperateResult {
            subquery: format!(
                "t{} AS (SELECT \"{}\", sum(\"{}\") as \"{}\" FROM {} GROUP BY \"{}\")",
                info.tmp_table_num,
                info.x_name,
                info.y_name,
                y_name,
                info.src_table,
                info.x_name,
            ),
            x_name,
            y_name,
        }
    }
}

declare_operator_no_param!(OrderOperator);

impl Operator for OrderOperator {
    fn to_sql(&self, info: &OperateInfo) -> OperateResult {
        let x_name = info.x_name.to_string();
        let y_name = self.append_column_name(&info.y_name);

        OperateResult {
            subquery: format!(
                "t{} AS (SELECT \"{}\", \"{}\" FROM {} ORDER BY \"{}\")",
                info.tmp_table_num,
                info.x_name,
                info.y_name,
                info.src_table,
                info.x_name,
            ),
            x_name,
            y_name,
        }
    }
}

declare_operator_no_param!(StepOperator);

impl Operator for StepOperator {
    fn to_sql(&self, info: &OperateInfo) -> OperateResult {
        let x_name = info.x_name.to_string();
        let y_name = self.append_column_name(&info.y_name);

        OperateResult {
            subquery: format!(
                "t{} AS (SELECT \"{}\", \"{}\" - lag(\"{}\") over () FROM {})",
                info.tmp_table_num,
                info.x_name,
                info.y_name,
                info.y_name,
                info.src_table,
            ),
            x_name,
            y_name,
        }
    }
}

declare_operator_no_param!(UniqueOperator);

impl Operator for UniqueOperator {
    fn to_sql(&self, info: &OperateInfo) -> OperateResult {
        let x_name = info.x_name.to_string();
        let y_name = self.append_column_name(&info.y_name);

        OperateResult {
            subquery: format!(
                "t{} AS (SELECT first(\"{}\"), first(\"{}\") FROM {} GROUP BY \"{}\")",
                info.tmp_table_num,
                info.x_name,
                info.y_name,
                info.src_table,
                info.x_name,
            ),
            x_name,
            y_name,
        }
    }
}

#[derive(Display, Debug, Clone)]
pub enum GenericOperator {
    #[strum(to_string = "{0}")]
    Average(AverageOperator),
    #[strum(to_string = "{0}")]
    Cdf(CDFOperator),
    #[strum(to_string = "{0}")]
    Derivative(DerivativeOperator),
    #[strum(to_string = "{0}")]
    FilterFinite(FilterFiniteOperator),
    #[strum(to_string = "{0}")]
    Integral(IntegralOperator),
    #[strum(to_string = "{0}")]
    Merge(MergeOperator),
    #[strum(to_string = "{0}")]
    Order(OrderOperator),
    #[strum(to_string = "{0}")]
    Step(StepOperator),
    #[strum(to_string = "{0}")]
    Unique(UniqueOperator),
}

impl TryFrom<Op> for GenericOperator {
    type Error = anyhow::Error;
    fn try_from(op: Op) -> Result<Self, Self::Error> {
        match op.op {
            'a' => Ok(GenericOperator::Average(op.try_into()?)),
            'c' => Ok(GenericOperator::Cdf(op.try_into()?)),
            'd' => Ok(GenericOperator::Derivative(op.try_into()?)),
            'f' => Ok(GenericOperator::FilterFinite(op.try_into()?)),
            'i' => Ok(GenericOperator::Integral(op.try_into()?)),
            'm' => Ok(GenericOperator::Merge(op.try_into()?)),
            'o' => Ok(GenericOperator::Order(op.try_into()?)),
            's' => Ok(GenericOperator::Step(op.try_into()?)),
            'u' => Ok(GenericOperator::Unique(op.try_into()?)),
            _ => Err(anyhow!("Invalid operator: {}", op.op)),
        }
    }
}

impl Operator for GenericOperator {
    fn to_sql(&self, info: &OperateInfo) -> OperateResult {
        match self {
            GenericOperator::Average(average) => average.to_sql(info),
            GenericOperator::Cdf(cdf) => cdf.to_sql(info),
            GenericOperator::Derivative(derivative) => derivative.to_sql(info),
            GenericOperator::FilterFinite(filter_finite) => {
                filter_finite.to_sql(info)
            }
            GenericOperator::Integral(integral) => integral.to_sql(info),
            GenericOperator::Merge(merge) => merge.to_sql(info),
            GenericOperator::Order(order) => order.to_sql(info),
            GenericOperator::Step(step) => step.to_sql(info),
            GenericOperator::Unique(unique) => unique.to_sql(info),
        }
    }
}

// OpSeq: The major data structure that Plotter works on
// Represents a sequence of Operations, enables deserialization from string
#[derive(Debug, Clone)]
pub struct OpSeq {
    pub ops: Vec<GenericOperator>,
}

impl FromStr for OpSeq {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let ops = Self::str_to_ops(s)?
            .into_iter()
            .map(GenericOperator::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { ops })
    }
}

impl Display for OpSeq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.ops
                .iter()
                .map(|op| op.to_string())
                .collect::<Vec<_>>()
                .join("")
        )
    }
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

    pub fn get_tmp_table_name(&self) -> String {
        format!("t{}", self.ops.len())
    }

    pub fn to_sql(
        &self,
        src_table: &str,
        x_name: &str,
        y_name: &str,
    ) -> String {
        if self.ops.is_empty() {
            return "".to_string();
        }
        format!(
            "WITH \n{}\n",
            self.ops
                .iter()
                .scan(
                    OperateInfo {
                        src_table: src_table.to_string(),
                        tmp_table_num: 1,
                        x_name: x_name.to_string(),
                        y_name: y_name.to_string(),
                    },
                    |info, op| {
                        let OperateResult {
                            subquery,
                            x_name,
                            y_name,
                        } = op.to_sql(info);
                        info.src_table = format!("t{}", info.tmp_table_num);
                        info.tmp_table_num += 1;
                        info.x_name = x_name;
                        info.y_name = y_name;
                        Some(subquery)
                    },
                )
                .collect::<Vec<_>>()
                .join(",\n")
        )
    }
}
