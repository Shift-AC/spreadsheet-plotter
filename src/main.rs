use std::backtrace::BacktraceStatus;

use crate::{
    datasheet::DataSheetFormat,
    plotter::{AdditionalGnuplotCommand, Plotter},
};
use anyhow::Result;

mod cachefile;
mod column;
mod commons;
mod datasheet;
mod opeseq;
mod plotter;

fn build_print_opt() -> getopts::Options {
    let mut opts = getopts::Options::new();
    opts.optflag("h", "help", "Print help message");
    opts.optflag("v", "version", "Print version message");
    opts
}

fn build_opt() -> getopts::Options {
    let mut opts = getopts::Options::new();

    opts.optopt("f", "iformat", "Format of input file", "FORMAT");
    opts.optopt(
        "g",
        "gpcmd",
        "Additional gnuplot command to be executed before 'plot', preferred over -G",
        "GPCMD",
    );
    opts.reqopt("i", "input", "Input spreadsheet file", "PATH");
    opts.optopt(
        "o",
        "output",
        "Output directory, default to current directory",
        "PATH",
    );
    opts.reqopt("e", "opseq", "Operator sequence to execute", "OPSEQ");
    opts.optopt("x", "xname", "The expression to be used as x axis", "EXPR");
    opts.optopt("y", "yname", "The expression to be used as y axis", "EXPR");

    opts.optopt("F", "oformat", "Format of output files", "FORMAT");
    opts.optopt(
        "G",
        "gpfile",
        "Additional gnuplot source file to be executed before 'plot'",
        "PATH",
    );
    opts.optflag(
        "t",
        "has-hdr",
        "If given, assume presence of column header in input file",
    );
    opts.optopt(
        "X",
        "xhdr",
        "Header text of the column to be used as x axis",
        "TEXT",
    );
    opts.optopt(
        "Y",
        "yhdr",
        "Header text of the column to be used as y axis",
        "TEXT",
    );
    opts
}

const ARG_EXPLAIN_MSG: &str = r"
    EXPR = 
        Arithmetic opration of COLUMNs (supported operators: +, -, *, /, ^, %).
        COLUMN = 
            Column identifier that is either #INDEX or @TITLE.
            INDEX = 
                column number (starting from 1) or Microsoft Excel flavored
                column name (case insensitive).
            TITLE = 
                column title
    FORMAT = (csv|lnk)
        Format of files, defaults to csv.
        csv: comma-separated spreadsheet file
        lnk: sp intermediate cache file, only available as input format
    PATH = 
        File path of the input file or output directory.
        If a input file is specified as '-', stdin would be used.
    OPSEQ = {[operator](arg)}+
        operator = 
            c: cdf
            d(smooth-window): derivation
              smooth-window: minimum x interval or derivation computation
            i: integral
            m: merge (sum of y values with consecutive same x value)
            o: sort by x axis
            r: rotate (swap x and y)
            s: step (difference of the consecutive y values)
            C: save current datasheet as file
            O: print current datasheet and exit
            P: plot current datasheet and exit
";
fn print_help(prog_name: &str, opts: &getopts::Options) {
    let brief = format!("Usage: {} [options]", prog_name);
    print!("{}", opts.usage(&brief));
    print!("{}", ARG_EXPLAIN_MSG);
}

fn print_version() -> Result<()> {
    // NOTE: This will output everything, and requires all features enabled.
    // NOTE: See the specific builder documentation for configuration options.
    let build = vergen::BuildBuilder::all_build().unwrap();
    let cargo = vergen::CargoBuilder::all_cargo().unwrap();
    let rustc = vergen::RustcBuilder::all_rustc().unwrap();
    let si = vergen::SysinfoBuilder::all_sysinfo().unwrap();
    vergen::Emitter::default()
        .add_instructions(&build)?
        .add_instructions(&cargo)?
        .add_instructions(&rustc)?
        .add_instructions(&si)?
        .emit()?;
    Ok(())
}

fn parse_args(
    print_opts: &getopts::Options,
    opts: &getopts::Options,
) -> Result<Plotter> {
    match print_opts.parse(std::env::args().skip(1)) {
        Ok(matches) => {
            if matches.opt_present("h") {
                print_help(std::env::args().next().unwrap().as_str(), &opts);
                std::process::exit(0);
            }
            if matches.opt_present("v") {
                print_version()?;
                std::process::exit(0);
            }
        }
        Err(_) => {
            // ignore any errors from this match since we are only checking if
            // the user simply wants us to print some information
        }
    };

    let matches = opts.parse(std::env::args().skip(1))?;

    let input = matches.opt_str("i").unwrap();
    let opseq_str = matches.opt_str("e").unwrap();
    let xexpr = matches.opt_str("x").unwrap();
    let yexpr = matches.opt_str("y").unwrap();
    let output = match matches.opt_str("o") {
        Some(f) => f,
        None => ".".to_string(),
    };

    let ifmt = match matches.opt_str("f") {
        Some(f) => f,
        None => "csv".to_string(),
    };
    let ofmt = match matches.opt_str("F") {
        Some(f) => f,
        None => "csv".to_string(),
    };

    let ihdr = matches.opt_present("t");

    let ifmt = DataSheetFormat::new(&ifmt, ihdr, &opseq_str)?;
    let ofmt = DataSheetFormat::new(&ofmt, true, &opseq_str)?;

    let gpcmd = if let Some(cmd) = matches.opt_str("g") {
        AdditionalGnuplotCommand::new(&cmd)
    } else if let Some(file) = matches.opt_str("G") {
        AdditionalGnuplotCommand::from_file(&file)?
    } else {
        AdditionalGnuplotCommand::new("")
    };

    let plotter = Plotter::new(
        &input, &opseq_str, &xexpr, &yexpr, ifmt, ofmt, &output, gpcmd,
    )?;

    Ok(plotter)
}

fn handle_err(e: anyhow::Error) {
    e.chain().for_each(|e| eprintln!("Error: {}", e));
    let bt = e.backtrace();
    match bt.status() {
        BacktraceStatus::Captured => {
            eprintln!("Backtrace:\n{}", bt);
        }
        BacktraceStatus::Unsupported => {
            eprintln!("Backtrace is unsupported.");
        }
        BacktraceStatus::Disabled => {
            eprintln!("Backtrace is disabled.");
        }
        _ => {
            eprintln!("Unknown backtrace status: {:?}", bt.status());
        }
    }
}

fn main() {
    env_logger::init();

    let print_opt = build_print_opt();
    let opt = build_opt();
    let mut plotter = match parse_args(&print_opt, &opt) {
        Ok(p) => p,
        Err(e) => {
            handle_err(e);
            std::process::exit(1);
        }
    };
    match plotter.apply() {
        Ok(_) => {}
        Err(e) => {
            handle_err(e);
            std::process::exit(1);
        }
    }
}
