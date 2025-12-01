# Spreadsheet Plotter

Spreadsheet Plotter (`sp`) is a linux command-line toolbox that takes spreadsheets as input, performs simple mathematical transformations on them, and produces plots with customizable apperance.
Despite designed as a plotter, `sp` also aims at being a convenient shortcut of many common operations on numeric data, given the fact that raw data often need to be pre-processed before being plotted.

The `sp` toolbox offers two main tools:

- `sp`: A command-line tool that takes a single spreadsheet as input, prepares dataset file for plotting by performing mathematical transformations on it, and plots the dataset directly onto the terminal. `sp` is command line-native: it provides minimum plotting functionalities in strictly-limited scenarios such as shells on mobile phones, routers or servers in production, etc.

- `msp`: A frontend of `sp` that invokes it to generate multiple data series and produces complex `.pdf` plot files or show plots on GUI screens. `msp` targets more complex (but common!) scenarios where users need to generate ready-for-use plot files with a single command or preview them.

The `sp` toolbox relies on two major building blocks: an SQL engine for data manipulation and `gnuplot` for plotting. For maximized portability, the `sp` tool was designed in the sense that not only the `sp` binary, but also its dependencies (e.g. `duckdb`, `gnuplot`) should be statically linkable. For `sp` itself, we use [`musl`](http://musl.libc.org/) as its C standard library and by default `sp` compiles to statically-linked binaries. For SQL engine, `sp` uses [`duckdb`](https://duckdb.org/), an SQLite implementation with no external dependencies and provides [`musl`](http://musl.libc.org/)-based binaries for major CPU achitectures. As for `gnuplot`, configuring `gnuplot` with the following command would generate a minimum statically-linked `gnuplot` binary with basic functionalities needed by `sp` for plotting to the terminal:

```bash
# Run this command in gnuplot source directory, tested with gnuplot 6.0
CC='musl-gcc -static' CXX='musl-gcc -static' ./configure --disable-plugins --disable-wxwidgets --without-qt --without-lua --without-wx --without-cairo --without-tektronix
```

## Build

`sp` and `msp` could be built with a single `cargo build --release` command.

To install this package, simply add the output of `readlink -f ./target/x86_64-unknown-linux-musl/release/` to your `$PATH`.

- Runtime dependencies (need to be in `$PATH`)

    Command | Dependencies
    --------|----
    `sp`    | `sh`, `gnuplot`, `duckdb` (>= 1.4)
    `msp`   | `ps2pdf`, `sp` (provided by this project)

## Workflow

`sp`'s complete workflow involves 4 steps: 

1. Pre-process the input file to generate intermediate table.

    Firstly, `sp` reads the input file as a table backed by the input file, then applies customizable `SELECT` statement to the table to generate an in-memory intermediate table. The source table is dropped afterwards.

2. Apply an user-defined "operator sequence" to the intermediate table. Operators are common mathematical transformations that could be applied to the data, such as computing CDF from raw points, computing derivative/integral, smoothing, etc.

    Each operator takes an in-memory table as input and outputs an in-memory temporary table. Specifically, Operators are implemented as `WITH` clause in SQL, where each operator is translated into a subquery.

3. Generate the final dataset file by applying customizable filter to the intermediate table.

    Such operation is implemented with a `SELECT` statement with a customizable `WHERE` clause. The final output would be redirected to a special file in the system temporary directory.

4. Generate the `gnuplot` source file and plot it with `gnuplot`.

The user may choose to execute only a part of the workflow. For example, it is possible to have `sp` print the dataset to `stdout`, or do nothing but print the SQL command that would be executed.

`msp`, instead, leverages `sp` as a data processing tool to generate multiple data series, and then generates `gnuplot` source file for itself to plot the data series.

## Quick Examples of `sp`

We first offer a quick reference to `sp` here by showing its functionalities with examples. Note that all programs in this project supports `-h` option for printing usage messages.

### Plotting a scatter plot

```
sp -i input.csv -x 'name' -y 'age'
```

This command reads `input.csv` and plots the data as a scatter plot with the `age` column as y axis and the `name` column as x axis. The plot would be directly printed (operator `P`) onto the terminal. In this example, `sp` invokes the following SQL query to retrieve the data:

```sql
SELECT name, age FROM 'input.csv'
```

If the input file is not specified, `sp` would read from `stdin`.

```
cat input.csv | sp -x 'name' -y 'age'
```

### Using different input formats

Theoretically, `sp` supports any input format that `duckdb` supports. However, `duckdb` relies on file extension name to infer the file format. By default, `sp` directly passes the file name to `duckdb`. However, there exists cases that input files does not have correct extension names and in such cases we may use the `--format` option to specify the extension name:

```
sp --format json -i input.csv -x 'name' -y 'age'
```

The `--format` option defaults to `auto`, which means `sp` would let `duckdb` infer the file format. The exception is when the input file is read from `stdin`, in which case `sp` would assume the file format is `csv`, and this is why the previous example works. Additionally, for typical datasheet files, we could use the option `--header` to control how `duckdb` interprets the first row. Here `true`/`false` forces `duckdb` to use/not use the first row as column header, and `auto` (default value) allows `duckdb` to automatically infer from file content. Note that `--header` must be used with `--format csv` or `--format xlsx`.

### Plotting a scatter plot using column indexes

```
sp -i input.csv -x '$1' -y '$2'
```

Although column names are efficient to use in many cases, there does exist datasheet files with no headers and in this case, `sp` also supports using column indexes to specify the x and y axes. Due to the fact that the SQL-based databases does not have built-in support of column indexes, `sp` would retrieve the table info first to get a list of column names, and then replace column indexes in the expressions with such names. Specifically, `sp` searches for all occurences of `[$]\d+` and tries to replace them. To stay compatible with the corner case where the character `$` is used in column names, we could also use the `--index-mark` option to specify a single character as the indicator of column indexes. For example, the command above is equal to:

```
sp -i input-csv --index-mark '#' -x '#1' -y '#2'
```

and `sp` would search for `[#]\d+` instead. 

### Customizing the appearance of plots

```
sp -i input.csv -x '$1' -y '$2' -g "set xrange [0:1000]"
```

`sp` supports customizing the appearance of plots with the `-g` option. The argument of `-g` is a string that would be directly passed to `gnuplot` and executed before the `plot` command.

Sometimes, specifying the gnuplot command in the command line is not convenient. Also, `-g` could not control the `plot` command. Therefore, `-g` lacks the ability of controlling the plot type, generating multiple plots, etc. In this case, `sp` also supports reading gnuplot commands from a file. The file could be specified with the `-G` option. For example, `-G "1.gp"` would execute `gnuplot` with `1.gp` instead. Note that when using `-G`, use the `input_file`, `xaxis`, `yaxis` macros as plot input, like the example below:

```gnuplot
plot input_file using xaxis:yaxis with points
```

`sp` would generate definition for the macros and pass them as the prefix of the final `gnuplot` source file.

### Replot

```
sp -r -g 'set xrange [0:1000]'
```

`sp` stores the spreadsheet data used in the previous plot command in a special temporary file. To conveniently re-plot the data with a different plot command, `sp` provides the `-r` option. When `-r` is provided, `sp` simply checks for existence of such temporary file and re-plot the data with the provided `gnuplot` command (via `-g` or `-G`).

### Pre-processing and Post-processing

```
sp -i input.csv -x '$1' -y 'sqrt(income) + 3'
```

Previously we are only using a single column as the x/y axises. However, `sp` also supports using a SQL expression as the value of the axises. The SQL expression would be directly used in a `SELECT` statement, so all scalar functions, operators, and window functions are also supported. 

```
sp -i input.csv --if 'income > 0' -x '$1' -y 'sqrt(income) + 3'
```

Sometimes, we would like to filter out some rows of the input data. `sp` supports this by providing the `--if` option. The argument of `--if` is a SQL expression to be used as the `WHERE` clause in the `SELECT` statement. 

```
sp -i input.csv --if 'income > 0' --of '$2 > 100' -x '$1' -y 'sqrt(income) + 3'
```

Also, `sp` supports filtering the final dataset with the `--of` option. Similarly, the argument of `--of` is a SQL expression to be used as the `WHERE` clause in the `SELECT` statement. Given that the column names are unspecified in the dataset, we could use `$1` to refer to the x axis and `$2` to refer to the y axis.

### Plotting transformed data

```
sp -i input.csv -e "id1000" -x '$1' -y '$2'
```

Slightly different from previous examples, this command first computes __integral__ (operator `i`) of the original data and then computes __derivation__ on a smooth window of ±1000 (operator `d`) of the integral. Finally, it plots the result onto the terminal.

Consider the case where we store a network trace in `input.csv` with two columns, the 1st column is timestamps in microseconds and the 2nd column is the size of packets received at the corresponding time. The example above would transform the original <time, packet size> pairs into <time, amount of received data> pairs by computing integral, and then produce the <time, throughput> pair by computing derivation (smoothed out to a 1ms time window).

### Dumping dataset/SQL command

```
sp -i input.csv -e "id1000" -x '$1' -y '$2' --mode dump
```

In some cases, we may simply intend to manipulate spreadsheets and generate input data for other tools. To achieve this, we need the `--mode` argument. The default value of `--mode` is `plot`, which would plot the data onto the terminal. However, with `--mode dump`, `sp` would dump the transformed data (as CSV data) to the terminal instead. We may also use `--mode dry-run` to let `sp` do nothing but print the SQL query that it would execute.

## Quick Examples of `msp` 

We offer a quick reference to `msp` here by showing its functionalities with examples. Note that `msp` uses default option values extensively. Make sure to run `msp -h` to check the default values!

Note: we recommend reading [`msp` Plot Style & Data Series](#msp-plot-style--data-series) first to get familiar with the data series specification.

### Plotting onto a GUI window or a PDF file

```
msp ',x=date,y=cost' -i balance.csv
```

This command would call `sp` to retrieve the `date` and `cost` columns from `balance.csv` and generate datasheet files, then call `gnuplot` to plot the resulting data onto a GUI window using the `x11` terminal.

```
msp ',x=date,y=cost' -i balance.csv --term postscript --gpout balance.pdf
```

Alternatively, by changing the terminal type (`--term`), `msp` could also produce PDF files. When producing files, `--gpout` option should be specified to indicate the _final_ output file name. Note that the `postscript` terminal in `gnuplot` produces postscript files. However, in `msp`, the output would be redirected to `ps2pdf` to directly generate the final PDF file.

### Producing different types of plots with manipulated data

```
msp ',x=date,y=cost,op=d,plot=linespoints,style=lw 5' -i balance.csv
```

Different from the last example, this command invokes `sp` to apply derivation operator `d0` on the `cost` column to produce the trend of `cost`, and then plots the resulting data as lines and points, with line width set to 5.

### Plotting multiple data series

```
msp ',x=date,y=cost,op=d,plot=linespoints,title=Alice' \
    ',x=date,y=cost,op=d,plot=linespoints,title=Bob' \
    -i balance.alice.csv \
    -i balance.bob.csv
```

Now we are beginning to run into something really different. In this example, `msp` invokes `sp` concurrently to process two input files `balance.alice.csv` and `balance.bob.csv` to calculate the trend of cost for both Alice and Bob. Then, `msp` plots the two data series together onto the same `x11` GUI screen, with their owner's names as the legend.

### Specifying Input files and series-specific options

```
cat balance.alice.csv |
msp ',f=0,x=date,y=$jul_cost,p=linespoints,l=Alice,s=lc red' \
    ',x=date,y=$jul_cost,p=linespoints,l=Bob,s=lc blue' \
    ',f=0,x=date,y=$jul_cost,o=d,p=linespoints,s=lc red,a=12' \
    ',x=date,y=$jul_cost,o=d,p=linespoints,s=lc blue,a=12' \
    -i balance.bob.csv
```

Despite implicitly inferred in most common cases, the input file of each data series could also be explicitly specified, either by the `--file` option that overwrites the inferring logic, or by the `file` key in the data series specification. Here the firs t data series comes from STDIN, whose index number is `0`. Then, the second data series uses the first file specified with `-i` (index `1`) because the default `--file` value is `+1`, which indicates that this data series should use the next file index. Then, the third data series resets input index to 0, and again, the last data series leverages the default value to set its input index to `1`.

Next, let's consider the meaning of the data. We are plotting information of two different people, Alice and Bob. Therefore, we should use the same style for the two data series of the same person. Also, the unit of cost and the derivation of cost is not the same, indicating that we should not use a unified y axis for both types of data. Therefore, we specify `style=lc red` and `style=lc blue` for Alice and Bob, respectively. Moreover, we use `axis=12` for derivation data to have them plotted on the y2 axis (12 for x1y2).

### Applying global options

```
cat balance.alice.csv |
msp ',f=0,l=Alice,s=lc red' \
    ',l=Bob,s=lc blue' \
    ',f=0,o=d,s=lc red,a=12' \
    ',o=d,s=lc blue,a=12' \
    -i balance.bob.csv \
    --plot linespoints --xexpr '$date' --yexpr '$jul_cost' \
    --font Helvetica,24 --xl Date --yl Cost --y2l "Derivation of Cost"
```

`msp` supports customizing default value of all options in the data series specification. This example plots exactly the same data as the previous example. However, the common parts in the data series specification are now replaced by command line options. Also, in this example we also specified various options to ensure the plot uses a pretty font and has appropriate x and y labels.

### Using references in data series specification

```
cat balance.alice.csv |
msp ',f=0,l=Alice,s=lc red' \
    ',l=Bob,s=lc blue' \
    ',f=0,o=d,rs=-2,a=12' \
    ',o=d,rs=2,a=12' \
    -i balance.bob.csv \
    --plot linespoints --xexpr '$date' --yexpr '$jul_cost' \
    --font Helvetica,24 --xl Date --yl Cost --y2l "Derivation of Cost"
```

In last example, we greatly reduced the length of the data series specification by using default values. However, the value of `style` is still long, and rewriting it for multiple times may introduce typos. In this example, we use `r[key]` (reference keys) to retrieve value from _previously-seen_ keys. In data series #3, we use the reference `-2` to refer to the `style` value of data series #(3 - 2); in data series #4, we use the reference `2` to refer to the `style` value of data series #2. Here we note that combining absolute and relative references could make the command confusing, and the recommended practice is to use only one type of reference for one key. We also note that `r[key]` are not real keys, so they do not have default values (thus you could not specify them with command line options!), and `rfile` is illegal, since `file` is already a reference.

### Preparing datasheet files and gnuplot command

```
msp -d (other options)
```

The `-d` option of `msp` causes `msp` to perform a dry-run that does not plot anything. Instead, it would invoke `sp` to generate datasheet files as specified by the other options, and print the gnuplot command it would use otherwise to the terminal. This option acts as a debug measure that allows the user to check the gnuplot command manually, and is also available for generating inputs of larger projects (e.g. a LaTeX project).

## Details

### Operator sequence

In `sp`, the operator sequence is a sequence of transforms that could be translated into pre-defined SQL queries, as listed below:

- `a<window>`: Average on a smooth window

    For table `(x, y)`, This operator computes the average of `y` on a customizable window and produces table `(x, avg(y))`. 

    - Specifying the window

        The window is specified by two numbers written as `left_window,right_window`. With such a window, `sp` takes all records with y value in the range `[x - left_window, x + right_window]` into consideration. The window could also be written as one number, `window`, or even an empty string. The case with only one number is the abbreviation of `window,window`, and the empty string is the abbreviation of `0.0,0.0`.

- `c`: Cumulative distribution function

    For table `(x, y)`, This operator computes the CDF of `y`.

- `d<window>`: Derivative

    For table `(x, y)`, This operator computes dy/dx on the specified window. `sp` uses the first record and the last record in the window for computation. When the window is `0.0,0.0`, `sp` instead uses current record and the previous record for computation instead.

- `f`: Filter finite values

    For table `(x, y)`, This operator filters out all records with infinite or NaN values in `y`.

- `i`: Integral

    For table `(x, y)`, This operator computes the integral of `y` with respect to `x`.

- `m`: Merged sum

    For table `(x, y)`, This operator accumulates the `y` value of each distinct `x` value into their sum.

- `o`: Order by x value

    This operator sorts the table by x value.

- `s`: Step (_i.e._ difference of the consecutive y values)
    
    For table `(x, y)`, This operator computes the difference of the consecutive y values.

- `u`: Preserve unique records

    For table `(x, y)`, This operator filters out all records with duplicate `x` value, preserving only the first record with each distinct `x` value.

### `msp` Plot Style & Data Series

`msp` is designed for a convenient short hand of both `gnuplot` and `sp` that plots multiple data series onto a single plot. As for the term "convenient", we require `msp` to be convenient enough to be called with solely command-line arguments (instead of introducing another scripting language) and flexible enough to cover most common plot types and styles. `msp` achieves this goal by breaking the plotting options into data series-specific options and global options, and specify calling interfaces for each of them:

- Data Series

    ```bash
    $ msp -h
    ...
      <SERIES>...  SERIES = ([d]key=value)...
                     d = single character to be used as delimiter
                     keys:
                       axis = axises to plot on ("12" for x1y2)
                       file = REF of data source file
                       ifilter = input filter expression
                       ofilter = output filter expression
                       opseq = transforms to apply on the data
                       plot = plot type of the data series
                       style = plotting style of the data series
                       title = title of the data series
                       xexpr = x-axis expression
                       yexpr = y-axis expression
                       rKEY = KEY's value of series[REF]
                         (rfile is illegal)
                   REF = (+|-)?[num]
                     [num]: Absolute index (1-based),
                       (0 for stdin if referring to input file)
                     (+|-)[num]: Relative index (current index +/- num),
                   NOTE: prefix of keys is also supported (e.g. a for axis).
                   Example:
                     ,file=0 => read from stdin
                     |x=$1|op=c|a=21 => xexpr="$1", opseq="c", axis="21"
                     ,rx=1,ry=-1 =>
                       xexpr=series[1].xexpr,
                       yexpr=previous_series.yexpr
    ...
    ```

    `msp` recognizes data series from the data series specification as shown above. A data series is represented by a `delimeter` character and `delimeter`-separated `key=value` expressions following it. The available `key`s are carefully picked to cover `sp`'s data manipulation functionality and common plotting options. 

    The following efforts are made to ensure the convenience and flexibility of `msp`:

    1. **Key-value pairs instead of value list:** To be more flexible, `msp` must support various data series-specific options to tune the behavior of both data processing and plotting. While value lists (e.g. ",1,2,3" for `,file=1,xexpr=2,yexpr=3`) are more concise, we found that too many options poses great difficulty for users to remember their order, and misplaced options will generate very confusing output sometimes because the value of one option may also be a valid value of another option. Therefore, we use key-value pairs for specifying options to improve readability.
    
    2. **File indexes instead of file paths:** Each data series is (inevitably) associated with a data source file. However, specifying file names in the data series specification would:
    
        1) Cause the specification to be considerably longer;
        2) Prevent the user from quickly typing file names with tab completion.
        3) Force the user to type the same file name repeatedly for multiple data series referencing the same file.

        Therefore, we use command line arguments to provide file names, which automatically enables tab completion. In data series specification, user only need to write a reference, which is shorter, but not equally readable as plain names (yes, we admit). However, file references are at least better than long paths that makes a simple data series specification longer than one terminal line, we believe :) 

    3. **Default values and abbreviations:** A critical drawback of having too many options is that specifying all of them would make the data series specification extremely long. Therefore, `msp` supports using abbreviations for keys like many other commands does, provides every option a default value, and supports customizing all default values with command line options. 
    
    4. **References:** To further avoid specifying the same options for multiple data series, `msp` also supports a unified reference format (`[REF]` in the help message) for forward-referencing previous option values. Notably, `file` also uses the reference format, but instead of referencing the value in another data series, `file` references to input files. We set the default value of `file` to `+1` to automatically infer file indexes for the common case where each data series comes from a separated file.
    
    5. **`delimeter` instead of escaping characters:** Given that `xexpr` and `yexpr` may contain arbitrary characters, it is not feasible to use any fixed delimeters to separate the key-value pairs without escaping them. However, in practice (especially for shell programming), escaping characters is indeed a catastrophe when generating command line arguments from code or passing them through programs. Moreover, reversely-escaping characters in original arguments manually is both exhausting and error-prone (why not use another program? This introduces another layer of escaping!). Therefore, inspired by `sed`, we use user-provided delimeter to divide the key-value expressions. We have three advantages here: 
    
        1) **No escaping**, of course; 
        2) **Readability not reduced**: we introduce only one extra meaningless character to implement a extremely-simple working parser; 
        3) **Supports arbitrary in-expression characters**: remember that Rust is UTF-8 native, and there are enough choices to ensure that `delimeter` would not present in the expressions.

- Plot style

    `msp` uses command line options to specify global settings that applies to all data series and plotting options. As for the plot style, `msp` supports two levels of customization:
    
    1. **Common use:** `msp` uses a hard-coded gnuplot template with various customizable parts such as terminal type, font and x/y range. Users may modify the default behavior of `msp` with corresponding command line options. For special behavior (e.g. `set logscale x`), users could also insert arbitrary gnuplot command just before the `plot` command via the `-g` option.
    
    2. **Advanced use:** `msp` allows users to provide a custom gnuplot script file to override the default template. Users may use the macro `ds_{file}` to point to the data sheets, and plot data like:
    
        ```
        plot ds_1 using 1:2 with lines
        ```

        Definition of `ds_{file}` macros would be generated by `msp` automatically and would be placed before the user-provided script.

    3. **Fine-tune of the default template:** `msp` provides a dry-run mode to prepare everything it needs to generate the plot. Users may use the `-d` option to prepare the data files and print the generated gnuplot command to stdout. This enables the user to check what is happening beneath `msp` and derive their own gnuplot commands from the default template (e.g. plot an additional function). 
    We also recognize this as an important measure for users to stay close with the `gnuplot` language, given the fact that convenient shorthands would easily cause us to forget the details :)
    