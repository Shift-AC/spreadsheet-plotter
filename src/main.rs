use std::backtrace::BacktraceStatus;

use crate::{
    commons::get_current_time_micros,
    datasheet::DataSheetFormat,
    plotter::{GnuplotCommand, Plotter},
};
use anyhow::{Result, anyhow};
use log::debug;

mod cachefile;
mod column;
mod commons;
mod datasheet;
mod opeseq;
mod plotter;

fn build_opt() -> getopts::Options {
    let mut opts = getopts::Options::new();

    opts.optopt("e", "opseq", "Operator sequence to execute", "OPSEQ");
    opts.optopt("f", "iformat", "Format of input file", "FORMAT");
    opts.optopt("F", "oformat", "Format of output files", "FORMAT");
    opts.optopt(
        "g",
        "gpcmd",
        "Additional gnuplot command to be executed before 'plot', preferred over -G",
        "GPCMD",
    );
    opts.optopt(
        "G",
        "gpfile",
        "Complete gnuplot source file to be used, use input_file as input path,\
        xaxis and yaxis as the two data series for plotting",
        "PATH",
    );
    opts.optflag("h", "help", "Print help message");
    opts.optflag(
        "H",
        "has-hdr",
        "If given, assume presence of column header in input file",
    );
    opts.optopt("i", "input", "Input spreadsheet file", "PATH");
    opts.optopt(
        "o",
        "output",
        "Output directory, default to current directory",
        "PATH",
    );
    opts.optflag("p", "preserve", "Preserve temporary files");
    opts.optflag("v", "version", "Print version message");
    opts.optopt("x", "xname", "The expression to be used as x axis", "EXPR");
    opts.optopt("y", "yname", "The expression to be used as y axis", "EXPR");
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
    println!("{}", env!("VERSION"));
    Ok(())
}

fn parse_args(opts: &getopts::Options) -> Result<Plotter> {
    debug!("Time info: init = {}", get_current_time_micros());

    let matches = opts.parse(std::env::args().skip(1))?;

    if matches.opt_present("h") {
        print_help(std::env::args().next().unwrap().as_str(), &opts);
        std::process::exit(0);
    }
    if matches.opt_present("v") {
        print_version()?;
        std::process::exit(0);
    }

    let input = matches
        .opt_str("i")
        .ok_or_else(|| anyhow!("Input file (-i) missing"))?;
    let opseq_str = matches
        .opt_str("e")
        .ok_or_else(|| anyhow!("Operator sequence (-e) missing"))?;
    let xexpr = match matches.opt_str("x") {
        Some(x) => x,
        None => "".to_string(),
    };
    let yexpr = match matches.opt_str("y") {
        Some(y) => y,
        None => "".to_string(),
    };
    let output = match matches.opt_str("o") {
        Some(f) => f,
        None => ".".to_string(),
    };
    let preserve = matches.opt_present("p");

    let ifmt = match matches.opt_str("f") {
        Some(f) => f,
        None => "csv".to_string(),
    };
    if &ifmt != "lnk" {
        if &xexpr == "" || &yexpr == "" {
            return Err(anyhow!(
                "x (-x) and y (-y) expressions are required for non-lnk format"
            ));
        }
    }

    let ofmt = match matches.opt_str("F") {
        Some(f) => f,
        None => "csv".to_string(),
    };

    let ihdr = matches.opt_present("H");

    let ifmt = DataSheetFormat::new(&ifmt, ihdr, &opseq_str)?;
    let ofmt = DataSheetFormat::new(&ofmt, true, &opseq_str)?;

    let gpcmd = if let Some(cmd) = matches.opt_str("g") {
        GnuplotCommand::from_additional_cmd(&cmd)
    } else if let Some(file) = matches.opt_str("G") {
        GnuplotCommand::from_file(&file)?
    } else {
        GnuplotCommand::from_additional_cmd("")
    };

    let plotter = Plotter::new(
        &input, &opseq_str, &xexpr, &yexpr, ifmt, ofmt, &output, gpcmd,
        preserve,
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

    let opt = build_opt();
    let mut plotter = match parse_args(&opt) {
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
