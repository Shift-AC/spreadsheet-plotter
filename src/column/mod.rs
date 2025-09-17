use anyhow::Result;
use core::fmt;

use crate::datasheet::Datasheet;
#[cfg(feature = "bc")]
mod bc;
#[cfg(not(feature = "bc"))]
mod clean_state;

// Error types
#[derive(Debug)]
#[cfg(not(feature = "bc"))]
pub enum ExpressionError {
    Parse(ParseError),
    Evaluate(EvaluationError),
}

#[derive(Debug, PartialEq)]
pub enum ParseError {
    #[cfg(not(feature = "bc"))]
    InvalidCharacter(char),

    #[cfg(not(feature = "bc"))]
    InvalidNumber,
    InvalidColumnReference(String),

    #[cfg(not(feature = "bc"))]
    UnexpectedToken(String),

    #[cfg(not(feature = "bc"))]
    MismatchedParentheses(String),
    #[cfg(feature = "bc")]
    UnexpectedEof,
}

#[derive(Debug, PartialEq)]
#[cfg(not(feature = "bc"))]
pub enum EvaluationError {
    ColumnNotFound(usize),
    RowIndexOutOfBounds,
    ColumnsDifferentLengths,
    NonFiniteNumber,
}

#[cfg(not(feature = "bc"))]
impl std::error::Error for ExpressionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExpressionError::Parse(e) => Some(e),
            ExpressionError::Evaluate(e) => Some(e),
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

#[cfg(not(feature = "bc"))]
impl std::error::Error for EvaluationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

// Display implementations
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(not(feature = "bc"))]
            ParseError::InvalidCharacter(c) => {
                write!(f, "Invalid character '{}'", c)
            }

            #[cfg(not(feature = "bc"))]
            ParseError::InvalidNumber => write!(f, "Invalid number format"),
            ParseError::InvalidColumnReference(s) => {
                write!(f, "Invalid column reference: {}", s)
            }

            #[cfg(not(feature = "bc"))]
            ParseError::UnexpectedToken(t) => {
                write!(f, "Unexpected token: {}", t)
            }

            #[cfg(not(feature = "bc"))]
            ParseError::MismatchedParentheses(l) => {
                write!(f, "Mismatched parentheses: {}", l)
            }
            #[cfg(feature = "bc")]
            ParseError::UnexpectedEof => {
                write!(f, "Unexpected end of expression")
            }
        }
    }
}

#[cfg(not(feature = "bc"))]
impl fmt::Display for EvaluationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvaluationError::ColumnNotFound(i) => {
                write!(f, "Column #{} not found", i)
            }
            EvaluationError::RowIndexOutOfBounds => {
                write!(f, "Row index out of bounds")
            }
            EvaluationError::ColumnsDifferentLengths => {
                write!(f, "Columns have different lengths")
            }
            EvaluationError::NonFiniteNumber => {
                write!(f, "Non-finite result (inf or NaN)")
            }
        }
    }
}

#[cfg(not(feature = "bc"))]
impl fmt::Display for ExpressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExpressionError::Parse(e) => write!(f, "Parse error: {}", e),
            ExpressionError::Evaluate(e) => {
                write!(f, "Evaluation error: {}", e)
            }
        }
    }
}

pub fn excel_column_name_to_index(s: &str) -> Result<usize, ParseError> {
    if s.chars().all(|c| c.is_ascii_alphabetic()) {
        let mut sum = 0;
        for c in s.chars() {
            sum =
                sum * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1);
        }
        Ok(sum)
    } else {
        Err(ParseError::InvalidColumnReference(format!(
            "Invalid column index: {}",
            s
        )))
    }
}

#[cfg(not(feature = "bc"))]
pub fn expression_is_constant(expr: &str) -> bool {
    !expr.contains('#') && !expr.contains('@')
}

pub fn expression_is_single_column(expr: &str) -> bool {
    let expr = expr.trim();

    if expr.starts_with('#') {
        return expr[1..].chars().all(|c| c.is_ascii_alphanumeric());
    }

    if !expr.starts_with('@') || !expr.ends_with('@') {
        return false;
    }

    let expr = &expr[1..expr.len() - 1];
    // if expr is a single column, it should not contain any standalone '@',
    // and all '@'s in the column name should be escaped with "\@".
    expr.chars()
        .scan(false, |escaped, c| {
            // returns None if current character caused the check to fail
            if *escaped {
                *escaped = false;
            } else if c == '@' {
                return None;
            } else if c == '\\' {
                *escaped = true;
            }
            Some(c)
        })
        // the expression is a single column iff all characters passed the test
        .count()
        == expr.len()
}

pub fn process_column_expressions_on_datasheet(
    ds: Datasheet,
    xexpr_str: &str,
    yexpr_str: &str,
) -> Result<Datasheet> {
    // choose the implementation from bc only if the `bc` feature is enabled
    #[cfg(feature = "bc")]
    {
        bc::process_column_expressions_on_datasheet(ds, xexpr_str, yexpr_str)
    }
    // otherwise, use the default implementation
    #[cfg(not(feature = "bc"))]
    {
        clean_state::process_column_expressions_on_datasheet(
            ds, xexpr_str, yexpr_str,
        )
    }
}
