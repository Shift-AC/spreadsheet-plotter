// supports manipulation of column data from the original spreadsheet

use std::fmt;

use crate::datasheet::Datasheet;
use anyhow::{Context, Result};
use log::debug;

// Helper trait to check for finite numbers
trait FiniteCheck {
    fn check_finite(self) -> Result<f64, EvaluationError>;
}

impl FiniteCheck for f64 {
    fn check_finite(self) -> Result<f64, EvaluationError> {
        if self.is_finite() {
            Ok(self)
        } else {
            Err(EvaluationError::NonFiniteNumber)
        }
    }
}

trait OperatorCheck {
    fn is_operator(self) -> bool;
}

impl OperatorCheck for char {
    fn is_operator(self) -> bool {
        self == '+'
            || self == '-'
            || self == '*'
            || self == '/'
            || self == '%'
            || self == '^'
            || self == '('
            || self == ')'
    }
}

// Error types
#[derive(Debug)]
pub enum ExpressionError {
    Parse(ParseError),
    Evaluate(EvaluationError),
}

#[derive(Debug, PartialEq)]
pub enum ParseError {
    InvalidCharacter(char),
    InvalidNumber,
    InvalidColumnReference(String),
    UnexpectedToken(String),
    MismatchedParentheses(String),
}

#[derive(Debug, PartialEq)]
pub enum EvaluationError {
    ColumnNotFound(usize),
    RowIndexOutOfBounds,
    ColumnsDifferentLengths,
    NonFiniteNumber,
}

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

impl std::error::Error for EvaluationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

// Token types
#[derive(Debug, PartialEq, Clone)]
enum Token {
    Number(f64),
    Column(usize),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
    Eof,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Number(n) => write!(f, "number '{}'", n),
            Token::Column(i) => write!(f, "column reference '@{}'", i),
            Token::Plus => write!(f, "+"),
            Token::Minus => write!(f, "-"),
            Token::Star => write!(f, "*"),
            Token::Slash => write!(f, "/"),
            Token::Percent => write!(f, "%"),
            Token::Caret => write!(f, "^"),
            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
            Token::Eof => write!(f, "end of input"),
        }
    }
}

// AST
#[derive(Debug, Clone)]
enum Expr {
    Number(f64),
    Column(usize),
    Add(Box<Expr>, Box<Expr>),
    Subtract(Box<Expr>, Box<Expr>),
    Multiply(Box<Expr>, Box<Expr>),
    Divide(Box<Expr>, Box<Expr>),
    Modulus(Box<Expr>, Box<Expr>),
    Exponentiate(Box<Expr>, Box<Expr>),
    UnaryNegate(Box<Expr>),
}

const DUMB_COLUMNS: Vec<Vec<f64>> = vec![];

impl Expr {
    // check if this expression does not require any column data to compute
    // if so, returns the result value
    fn constant_result(&self) -> Option<f64> {
        match self {
            Expr::Number(n) => Some(*n),
            _ => self.evaluate(&DUMB_COLUMNS, 1).ok(),
        }
    }

    fn evaluate(
        &self,
        columns: &[Vec<f64>],
        row: usize,
    ) -> Result<f64, EvaluationError> {
        match self {
            Expr::Number(n) => Ok(n.check_finite()?),
            Expr::Column(i) => {
                let column = columns
                    .get(*i - 1)
                    .ok_or_else(|| EvaluationError::ColumnNotFound(*i))?;
                let value = column
                    .get(row)
                    .ok_or(EvaluationError::RowIndexOutOfBounds)?;
                value.check_finite()
            }
            Expr::Add(a, b) => (a.evaluate(columns, row)?
                + b.evaluate(columns, row)?)
            .check_finite(),
            Expr::Subtract(a, b) => (a.evaluate(columns, row)?
                - b.evaluate(columns, row)?)
            .check_finite(),
            Expr::Multiply(a, b) => (a.evaluate(columns, row)?
                * b.evaluate(columns, row)?)
            .check_finite(),
            Expr::Divide(a, b) => (a.evaluate(columns, row)?
                / b.evaluate(columns, row)?)
            .check_finite(),
            Expr::Modulus(a, b) => (a.evaluate(columns, row)?
                % b.evaluate(columns, row)?)
            .check_finite(),
            Expr::Exponentiate(a, b) => a
                .evaluate(columns, row)?
                .powf(b.evaluate(columns, row)?)
                .check_finite(),
            Expr::UnaryNegate(inner) => {
                (-inner.evaluate(columns, row)?).check_finite()
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

// Fixed Lexer with improved number parsing
struct Lexer {
    input: Vec<char>,
    position: usize,
}

impl Lexer {
    fn new(input: &str) -> Self {
        Lexer {
            input: input.chars().collect(),
            position: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.position).copied()
    }

    fn next(&mut self) -> Option<char> {
        if self.position < self.input.len() {
            let c = self.input[self.position];
            self.position += 1;
            Some(c)
        } else {
            None
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.next();
            } else {
                break;
            }
        }
    }

    fn next_token(&mut self, ds: &Datasheet) -> Result<Token, ParseError> {
        self.skip_whitespace();

        match self.next() {
            Some('+') => Ok(Token::Plus),
            Some('-') => Ok(Token::Minus),
            Some('*') => Ok(Token::Star),
            Some('/') => Ok(Token::Slash),
            Some('%') => Ok(Token::Percent),
            Some('^') => Ok(Token::Caret),
            Some('(') => Ok(Token::LParen),
            Some(')') => Ok(Token::RParen),

            // Column references: # followed by digits
            Some('#') => {
                let mut index_str = String::new();
                while let Some(d) = self.peek() {
                    if d.is_ascii_digit() || d.is_ascii_alphabetic() {
                        index_str.push(d);
                        self.next();
                    } else {
                        break;
                    }
                }
                if index_str.is_empty() {
                    return Err(ParseError::InvalidColumnReference(
                        "Empty column reference".to_string(),
                    ));
                }

                if index_str.starts_with(|c: char| c.is_ascii_alphabetic()) {
                    let index = excel_column_name_to_index(&index_str)?;
                    Ok(Token::Column(index))
                } else {
                    let index = index_str.parse().map_err(|_| {
                        ParseError::InvalidColumnReference(format!(
                            "Invalid column index: {}",
                            index_str
                        ))
                    })?;
                    Ok(Token::Column(index))
                }
            }

            // Column names: strings quoted with '@',
            // '\' escapes the next character
            Some('@') => {
                let mut name = String::new();
                while let Some(c) = self.peek() {
                    if c == '\\' {
                        self.next();
                        name.push(self.next().unwrap());
                        continue;
                    }
                    if c == '@' {
                        self.next();
                        break;
                    }
                    name.push(c);
                    self.next();
                }
                if name.is_empty() {
                    return Err(ParseError::InvalidColumnReference(
                        "Empty column name".to_string(),
                    ));
                }
                let column_index = ds.get_column_index(&name).ok_or(
                    ParseError::InvalidColumnReference(format!(
                        "Unknown column name '{}' \
                        (did you specified -H option?)",
                        name
                    )),
                )? + 1;
                debug!("Column name {} -> {}", name, column_index);
                Ok(Token::Column(column_index))
            }

            // Number parsing with strict validation (fixed)
            Some(c) if c.is_ascii_digit() || c == '.' => {
                let mut num_str = String::new();
                num_str.push(c);
                let mut has_dot = c == '.';
                let mut has_digits = c.is_ascii_digit();

                while let Some(d) = self.peek() {
                    if d.is_ascii_digit() {
                        num_str.push(d);
                        self.next();
                        has_digits = true;
                    } else if d == '.' && !has_dot {
                        num_str.push(d);
                        self.next();
                        has_dot = true;
                    } else if d.is_ascii_whitespace() || d.is_operator() {
                        break;
                    } else {
                        return Err(ParseError::InvalidNumber);
                    }
                }

                // Validate the number format
                if !has_digits {
                    return Err(ParseError::InvalidNumber); // "." with no digits
                }

                num_str
                    .parse()
                    .map(Token::Number)
                    .map_err(|_| ParseError::InvalidNumber)
            }

            Some(c) => Err(ParseError::InvalidCharacter(c)),
            None => Ok(Token::Eof),
        }
    }

    fn generate_current_status_text(&self) -> String {
        format!(
            "{}\n{}^",
            self.input.iter().collect::<String>(),
            (0..self.position - 1).map(|_| " ").collect::<String>()
        )
    }
}

struct Parser<'a> {
    lexer: Lexer,
    current_token: Token,
    ds: &'a Datasheet,
}

impl<'a> Parser<'a> {
    fn new(lexer: Lexer, ds: &'a Datasheet) -> Result<Self> {
        let mut parser = Parser {
            lexer,
            current_token: Token::Eof,
            ds,
        };
        parser.current_token =
            parser.lexer.next_token(parser.ds).map_err(|e| {
                anyhow::Error::new(e)
                    .context(parser.lexer.generate_current_status_text())
            })?;
        Ok(parser)
    }

    fn eat(&mut self, expected: Token) -> Result<()> {
        if self.current_token == expected {
            self.current_token = self
                .lexer
                .next_token(self.ds)
                .context(self.lexer.generate_current_status_text())?;
            Ok(())
        } else {
            Err(anyhow::Error::new(ParseError::UnexpectedToken(
                self.current_token.to_string(),
            ))
            .context(self.lexer.generate_current_status_text()))
        }
    }

    fn parse_factor(&mut self) -> Result<Expr> {
        if self.current_token == Token::Minus {
            self.eat(Token::Minus)?;
            let inner = self.parse_factor()?;
            return Ok(Expr::UnaryNegate(Box::new(inner)));
        }

        match &self.current_token {
            Token::Number(n) => {
                let expr = Expr::Number(*n);
                self.eat(Token::Number(*n))?;
                Ok(expr)
            }
            Token::Column(i) => {
                let expr = Expr::Column(*i);
                self.eat(Token::Column(*i))?;
                Ok(expr)
            }
            Token::LParen => {
                self.eat(Token::LParen)?;
                let expr = self.parse_expression()?;
                self.eat(Token::RParen).map_err(|e| {
                    anyhow::Error::new(ParseError::MismatchedParentheses(
                        e.to_string(),
                    ))
                })?;
                Ok(expr)
            }
            _ => Err(anyhow::Error::new(ParseError::UnexpectedToken(
                self.current_token.to_string(),
            ))
            .context(self.lexer.generate_current_status_text())),
        }
    }

    fn parse_exponent(&mut self) -> Result<Expr> {
        let mut expr = self.parse_factor()?;
        while matches!(self.current_token, Token::Caret) {
            self.eat(Token::Caret)?;
            let rhs = self.parse_factor()?;
            expr = Expr::Exponentiate(Box::new(expr), Box::new(rhs));
        }
        Ok(expr)
    }

    fn parse_term(&mut self) -> Result<Expr> {
        let mut expr = self.parse_exponent()?;
        while matches!(
            self.current_token,
            Token::Star | Token::Slash | Token::Percent
        ) {
            match self.current_token {
                Token::Star => {
                    self.eat(Token::Star)?;
                    let rhs = self.parse_exponent()?;
                    expr = Expr::Multiply(Box::new(expr), Box::new(rhs));
                }
                Token::Slash => {
                    self.eat(Token::Slash)?;
                    let rhs = self.parse_exponent()?;
                    expr = Expr::Divide(Box::new(expr), Box::new(rhs));
                }
                Token::Percent => {
                    self.eat(Token::Percent)?;
                    let rhs = self.parse_exponent()?;
                    expr = Expr::Modulus(Box::new(expr), Box::new(rhs));
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_expression(&mut self) -> Result<Expr> {
        let mut expr = self.parse_term()?;
        while matches!(self.current_token, Token::Plus | Token::Minus) {
            match self.current_token {
                Token::Plus => {
                    self.eat(Token::Plus)?;
                    let rhs = self.parse_term()?;
                    expr = Expr::Add(Box::new(expr), Box::new(rhs));
                }
                Token::Minus => {
                    self.eat(Token::Minus)?;
                    let rhs = self.parse_term()?;
                    expr = Expr::Subtract(Box::new(expr), Box::new(rhs));
                }
                _ => break,
            }
        }
        Ok(expr)
    }
}

// Compiler function
fn compile_expression(expression: &str, ds: &Datasheet) -> Result<Expr> {
    let lexer = Lexer::new(expression);
    let mut parser = Parser::new(lexer, ds).context(expression.to_string())?;
    let expr = parser.parse_expression()?;

    if parser.current_token != Token::Eof {
        Err(anyhow::Error::new(ExpressionError::Parse(
            ParseError::UnexpectedToken(parser.current_token.to_string()),
        ))
        .context(expression.to_string()))
    } else {
        Ok(expr)
    }
}

fn wrap_expr(
    expr: Expr,
    ds: &Datasheet,
) -> Result<impl Fn() -> Result<Vec<f64>, ExpressionError>> {
    Ok(move || {
        if ds.columns.is_empty() {
            return Ok(Vec::new());
        }

        let num_rows = ds.columns[0].len();
        if !ds.columns.iter().all(|col| col.len() == num_rows) {
            return Err(ExpressionError::Evaluate(
                EvaluationError::ColumnsDifferentLengths,
            ));
        }

        let mut results = Vec::with_capacity(num_rows);
        for row in 0..num_rows {
            let result = expr
                .evaluate(&ds.columns, row)
                .map_err(ExpressionError::Evaluate)?;
            results.push(result);
        }

        Ok(results)
    })
}

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

// handle the arithmetic expression specified by the command line arguments.
// takes the original datasheet, computes the x and y values to be used by
// the OpSeq, then returns the results as a new datasheet
pub fn process_column_expressions_on_datasheet(
    ds: Datasheet,
    xexpr_str: &str,
    yexpr_str: &str,
) -> Result<Datasheet> {
    let xexpr = compile_expression(xexpr_str, &ds)?;
    let yexpr = compile_expression(yexpr_str, &ds)?;

    let xresults = if expression_is_constant(xexpr_str) {
        vec![xexpr.constant_result().unwrap(); ds.columns[0].len()]
    } else {
        wrap_expr(xexpr, &ds)?()?
    };

    let yresults = if expression_is_constant(yexpr_str) {
        vec![yexpr.constant_result().unwrap(); ds.columns[0].len()]
    } else {
        wrap_expr(yexpr, &ds)?()?
    };
    let mut columns = Vec::new();
    columns.push(xresults);
    columns.push(yresults);
    let mut column_names = Vec::new();
    column_names.push(xexpr_str.to_string());
    column_names.push(yexpr_str.to_string());
    let new_ds = Datasheet::new(column_names, columns, None);
    Ok(new_ds)
}

// Display implementations
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::InvalidCharacter(c) => {
                write!(f, "Invalid character '{}'", c)
            }
            ParseError::InvalidNumber => write!(f, "Invalid number format"),
            ParseError::InvalidColumnReference(s) => {
                write!(f, "Invalid column reference: {}", s)
            }
            ParseError::UnexpectedToken(t) => {
                write!(f, "Unexpected token: {}", t)
            }
            ParseError::MismatchedParentheses(l) => {
                write!(f, "Mismatched parentheses: {}", l)
            }
        }
    }
}

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
