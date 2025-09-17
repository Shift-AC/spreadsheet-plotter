// supports manipulation of column data from the original spreadsheet
//
// this implementation preserves the grammar of column references, replace the
// column references with real values to build an expression to be evaluated
// by `bc`

use std::{
    io::{BufReader, BufWriter, Read, Write},
    process::{Command, Stdio},
};

use anyhow::{Result, anyhow};

use crate::{column::*, datasheet::Datasheet};

enum Item {
    Column(usize),
    RawExpression(String),
}

impl Item {
    fn check_column_number(
        column_number: usize,
        column_count: usize,
    ) -> Result<()> {
        if column_number > 0 && column_number <= column_count {
            Ok(())
        } else {
            Err(anyhow!(ParseError::InvalidColumnReference(
                column_number.to_string(),
            )))
        }
    }

    fn from_parse_state(state: &ParseState, ds: &Datasheet) -> Result<Self> {
        match state {
            ParseState::None | ParseState::ColumnIndexStart => {
                Err(anyhow!("BUG: Creating column Item from nothing"))
            }
            ParseState::RawExpression(s) => Ok(Item::RawExpression(s.clone())),
            ParseState::RawColumnIndex(s) => {
                let column_number = s.parse().map_err(|_| {
                    anyhow!(ParseError::InvalidColumnReference(s.clone()))
                })?;
                Item::check_column_number(column_number, ds.columns.len())?;
                Ok(Item::Column(column_number))
            }
            ParseState::ExcelColumnIndex(s) => {
                let column_number = excel_column_name_to_index(s)?;
                Item::check_column_number(column_number, ds.columns.len())?;
                Ok(Item::Column(column_number))
            }
            ParseState::ColumnTitle(s) => {
                Ok(Item::Column(ds.get_column_index(s).ok_or_else(|| {
                    anyhow!(ParseError::InvalidColumnReference(s.clone()))
                })?))
            }
        }
    }
}

struct Expr {
    items: Vec<Item>,
}

enum ParseState {
    None,
    RawExpression(String),
    ColumnIndexStart,
    RawColumnIndex(String),
    ExcelColumnIndex(String),
    ColumnTitle(String),
}

impl Expr {
    fn generate_context_str(s: &str, i: usize) -> String {
        format!("{}\n{}^", s, " ".repeat(i))
    }

    fn from_string(s: &str, ds: &Datasheet) -> Result<Self> {
        let mut state = ParseState::None;
        let mut items = Vec::new();
        let chars: Vec<_> = s.chars().collect();

        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            match state {
                ParseState::None => match c {
                    '#' => state = ParseState::ColumnIndexStart,
                    '@' => state = ParseState::ColumnTitle("".to_string()),
                    _ => state = ParseState::RawExpression("".to_string()),
                },
                ParseState::RawExpression(ref mut expr) => {
                    match c {
                        '#' | '@' => {
                            items.push(
                                Item::from_parse_state(&state, ds).map_err(
                                    |e| {
                                        e.context(Expr::generate_context_str(
                                            s, i,
                                        ))
                                    },
                                )?,
                            );
                            i -= 1;
                            state = ParseState::None;
                        }
                        _ => {
                            expr.push(c);
                        }
                    };
                }
                ParseState::ColumnIndexStart => {
                    if c.is_digit(10) {
                        state = ParseState::RawColumnIndex(c.to_string());
                    } else if c.is_ascii_alphabetic() {
                        state = ParseState::ExcelColumnIndex(c.to_string());
                    } else {
                        return Err(anyhow!(
                            ParseError::InvalidColumnReference(c.to_string())
                        )
                        .context(Expr::generate_context_str(s, i)));
                    }
                }
                ParseState::RawColumnIndex(ref mut index) => {
                    if c.is_digit(10) {
                        index.push(c);
                    } else if c.is_whitespace() {
                        items.push(
                            Item::from_parse_state(&state, ds).map_err(
                                |e| e.context(Expr::generate_context_str(s, i)),
                            )?,
                        );
                        i -= 1;
                        state = ParseState::None;
                    } else {
                        return Err(anyhow!(
                            ParseError::InvalidColumnReference(c.to_string())
                        )
                        .context(Expr::generate_context_str(s, i)));
                    }
                }
                ParseState::ExcelColumnIndex(ref mut index) => {
                    if c.is_ascii_alphabetic() {
                        index.push(c);
                    } else if c.is_whitespace() {
                        items.push(
                            Item::from_parse_state(&state, ds).map_err(
                                |e| e.context(Expr::generate_context_str(s, i)),
                            )?,
                        );
                        i -= 1;
                        state = ParseState::None;
                    } else {
                        return Err(anyhow!(
                            ParseError::InvalidColumnReference(c.to_string())
                        )
                        .context(Expr::generate_context_str(s, i)));
                    }
                }
                ParseState::ColumnTitle(ref mut index) => match c {
                    '\\' => {
                        i += 1;
                        if i + 1 < chars.len() {
                            index.push(chars[i + 1]);
                        } else {
                            return Err(anyhow!(ParseError::UnexpectedEof)
                                .context(Expr::generate_context_str(s, i)));
                        }
                    }
                    '@' => {
                        items.push(
                            Item::from_parse_state(&state, ds).map_err(
                                |e| e.context(Expr::generate_context_str(s, i)),
                            )?,
                        );
                        state = ParseState::None;
                    }
                    _ => {
                        index.push(c);
                    }
                },
            }
            i += 1;
        }
        Ok(Expr { items })
    }

    fn print_bc_expression<W>(
        &self,
        columns: &Vec<Vec<f64>>,
        row: usize,
        writer: &mut W,
    ) -> Result<()>
    where
        W: Write,
    {
        for item in &self.items {
            match item {
                Item::Column(i) => {
                    write!(writer, "{}", columns[*i][row])?;
                }
                Item::RawExpression(s) => {
                    write!(writer, "{}", s)?;
                }
            }
        }
        writeln!(writer)?;
        writer.flush()?;
        Ok(())
    }
}

// send one expression to `bc` and return the result
fn send_expression_to_bc<R, W>(
    expr: &Expr,
    columns: &Vec<Vec<f64>>,
    output_buf: &mut Vec<u8>,
    row: usize,
    reader: &mut R,
    writer: &mut W,
) -> Result<f64>
where
    R: Read,
    W: Write,
{
    // print the expression to `bc`
    expr.print_bc_expression(columns, row, writer)?;

    // read the output of `bc`
    reader.read_to_end(output_buf)?;
    let output_str = String::from_utf8_lossy(&output_buf);
    let output = output_str.trim().parse::<f64>()?;
    Ok(output)
}

// handle the arithmetic expression, but via `bc`
pub fn process_column_expressions_on_datasheet(
    ds: Datasheet,
    xexpr_str: &str,
    yexpr_str: &str,
) -> Result<Datasheet> {
    // parse the expression
    let xexpr = Expr::from_string(xexpr_str, &ds)?;
    let yexpr = Expr::from_string(yexpr_str, &ds)?;

    // execute the `bc` command and open a pipe to it
    let mut bc = Command::new("bc")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    // for each row in the datasheet, print the expression to `bc` and collect
    // the result
    let mut stdin = BufWriter::new(bc.stdin.take().unwrap());
    let mut stdout = BufReader::new(bc.stdout.take().unwrap());
    let mut output_buf = Vec::new();
    let (xresults, yresults) = (0..ds.columns[0].len())
        .map(|row| -> Result<(f64, f64)> {
            if row % 1000 == 1 {
                log::info!("processed {} rows", row);
            }
            Ok((
                send_expression_to_bc(
                    &xexpr,
                    &ds.columns,
                    &mut output_buf,
                    row,
                    &mut stdout,
                    &mut stdin,
                )?,
                send_expression_to_bc(
                    &yexpr,
                    &ds.columns,
                    &mut output_buf,
                    row,
                    &mut stdout,
                    &mut stdin,
                )?,
            ))
        })
        .try_fold(
            (Vec::new(), Vec::new()),
            |mut acc, pair| -> Result<(Vec<f64>, Vec<f64>)> {
                let (x, y) = pair?;
                acc.0.push(x);
                acc.1.push(y);
                Ok(acc)
            },
        )?;

    let mut columns = Vec::new();
    columns.push(xresults);
    columns.push(yresults);
    let mut column_names = Vec::new();
    column_names.push(xexpr_str.to_string());
    column_names.push(yexpr_str.to_string());
    let new_ds = Datasheet::new(column_names, columns, None);
    Ok(new_ds)
}
