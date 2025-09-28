# Spreadsheet Plotter

Spreadsheet Plotter (`sp`) is a linux command-line tool that takes spreadsheets as input, manipulates the data mathematically to produce a data series and plots it with `gnuplot`. With customized behavior of saving intermidiate results and processing the data, `sp` also supports performing pure mathematical transformation on spreadsheets, quickly re-plotting data without processing the original spreadsheet again, and customizing the apperance of output plots. 

The workflow of `sp` is defined by "operator sequences", which could be represented with a string. Each single alphabet in the string is an operator that causes `sp` to manipulate the data or dump outputs, operators may have comma-separated arguments (numbers only) that directly follows them. `sp` iterates through the operator sequence and executes each of them. 

`sp` focuses solely on exactly one data series, and is designed to be usable without any GUI support (it prints plots directly onto the terminal window!). To further extend its functionalities, Multi-Spreadsheet Plotter (`msp`) acts as a frontend of `sp` that invokes it to generate multiple data series and produces complex `.pdf` plot files or show plots on GUI screens. The style of the generated plot is highly customizable: the `gnuplot` command could be altered with various command line options or even completely replaced with a external file; `msp` itself is also capable of printing the `gnuplot` command it would use for users to derive their own `gnuplot` scripts.

## Build

`sp` and `msp` could be built with a single `cargo build --release` command.

To install this package, simply add the output of `readlink -f ./target/release/` to your `$PATH`.

- Runtime dependencies

    Command | Dependencies
    --------|----
    `sp`    | `gnuplot`, `mlr` (>= 6.0)
    `msp`   | `ps2pdf`, `sp` (provided by this project)

## Quick Examples of `sp`

We first offer a quick reference to `sp` here by showing its functionalities with examples. Note that all programs in this project supports `-h` option for printing usage messages.

### Plotting a scatter plot

```
sp -i input.csv -e "P" -x '$1' -y '$2'
```

This command reads `input.csv` and plots the data as a scatter plot with the 2nd column as y axis and the 1st column as x axis. The plot would be directly printed (operator `P`) onto the terminal.

```
cat input.csv | sp -e "P" -x '$1' -y '$2'
```

If the input file is not specified, `sp` would read from `stdin`.


### Customizing the appearance of plots

```
sp -i input.csv -e "P" -x '$1' -y '$2' -g "set xrange [0:1000]"
```

`sp` supports customizing the appearance of plots with the `-g` option. The argument of `-g` is a string that would be directly passed to `gnuplot` and executed before the `plot` command.

Sometimes, specifying the gnuplot command in the command line is not convenient. Also, `-g` could not control the `plot` command. Therefore, `-g` lacks the ability of controlling the plot type, generating multiple plots, etc. In this case, `sp` also supports reading gnuplot commands from a file. The file would be specified with the `-G` option. For example, `-G "1.gp"` would execute `gnuplot` with `1.gp` instead. Note that when using `-G`, use the `input_file`, `xaxis`, `yaxis` macros as plot input, like the example below:

```gnuplot
plot input_file using xaxis:yaxis with points
```

### Replot

```
sp -r -g 'set xrange [0:1000]'
```

`sp` stores the spreadsheet data used in the previous plot command in a special temporary file. To conveniently re-plot the data with a different plot command, `sp` provides the `-r` option. When `-r` is provided, `sp` simply checks for existence of such temporary file and re-plot the data with the provided `gnuplot` command (via `-g` or `-G`).

### Pre-processing data

```
sp -i input.csv -e "P" -x '$1' -y '${column_name} * 2 + 3'
```

`sp` supports data pre-processing by invoking `mlr` on the input data. Any valid `mlr` expression could be used as the x and y values, and `sp` uses the following expression to perform preprocessing:

```
mlr filter 'print ([xexpr]).",".([yexpr]) ;false'
```

During the pre-processing, `sp` first checks for simple expressions that could be handled with lower overhead: `sp` could detect simple column references (covers common usage like `$name` or `${name}` or `$[[[index]]]`) and pure mathematical expressions that do not involve any column references. If both of the axis are simple expressions, `sp` would not call `mlr` to process the input data (for mathematical expressions, we still use `mlr` as a calculator, but the expression is evaluated only once) and will directly read from the source file. 

### Plotting transformed data

```
sp -i input.csv -e "id1000P" -x '$1' -y '$2'
```

Slightly different from the first example, this command first computes __integral__ (operator `i`) of the original data and then computes __derivation__ on a smooth window of 1000 (operator `d`) of the integral. Finally, it plots the result onto the terminal.

Consider the case where we store a network trace in `input.csv` with two columns, the 1st column is timestamps in microseconds and the 2nd column is the size of packets received at the corresponding time. The example above would transform the original <time, packet size> pairs into <time, amount of received data> pairs by computing integral, and then produce the <time, throughput> pair by computing derivation (smoothed out to a 1ms time window).

### Store and use intermediate results 

```
sp -i input.csv --ocprefix "sp-" -e "iCd1000CP" -x '$1' -y '$2'
```

Again, this example is only slightly different from the last one. The only difference in the operation sequence is the `C` operators that causes `sp` to store intermediate results. In this example, before moving on to compute the derivation, `sp` firstly dumps intermediate results to a cache file; also, before plotting, `sp` dumps the final result to a cache file. Here the cache file contains both information about current run and the spreadsheet data. The former is a `TOML` snippet and the latter is in the specified output format (discussed in the next part). The two parts in the file are separated with a _human-readable_ separator. Here we note that the `C` operator is allowed only if the input file is not `stdin`.

When generating cache files, `sp` uses the command line argument `--ocprefix` to specify the prefix of the cache file names. If not specified, `sp` would use `sp-` as the prefix. The generated cache files would be named as `${cache-prefix}[op].spds`, where `[op]` is the operator sequence needed for generating this file. 

With the help of `splnk`, `sp` is also capable of quickly re-plotting as shown in the example below:

```
sp -i sp-i.spds -f lnk -e "id1000cP"
```

In this example, we would like to generate a CDF (operator `c`) plot --- take the network throughput case as an example, this time we will generate the distribution of network throughput instead of the raw time series.

To achieve this, `sp` first reads the `-f` (input format) and `-i` option to know that we are providing a link file. Note that `-f` option defaults to `csv`, so `-f csv` is not needed in previous examples. `sp` checks the operator sequence to find out whether the cached file contains intermediate results of current operator sequence. If so, `sp` will start processing from it. Otherwise, `sp` reads the original file (whose name is recorded in the cache file) and start from the beginning. We also note that when using cache files, the axis expressions are ignored.

### Performing mathematical transformation

```
sp -i input.csv -F csv -e "id1000O" -x '$1' -y '$2'
```

In some cases, we may only want to perform pure mathematical transformation on spreadsheets and generate input data for other tools. To achieve this, we only need to change the final operator from `P` to `O`, which prints the result datasheet to the terminal. Here the output format is specified with the `-F` option. The default output format is also `csv`, so `-F csv` is not mandatory in this example.

## Quick Examples of `msp` 

We offer a quick reference to `msp` here by showing its functionalities with examples. Note that `msp` uses default option values extensively. Make sure to run `msp -h` to check the default values!

Note: we recommend reading [`msp` Plot Style & Data Series](#msp-plot-style--data-series) first to get familiar with the data series specification.

### Plotting onto a GUI window or a PDF file

```
msp ',x=$date,y=$cost' -i balance.csv
```

This command would call `sp` to retrieve the `$date` and `$cost` columns from `balance.csv` and generate datasheet files, then call `gnuplot` to plot the resulting data onto a GUI window using the `x11` terminal.

```
msp ',x=$date,y=$cost' -i balance.csv --term postscript --gpout balance.pdf
```

Alternatively, by changing the terminal type (`--term`), `msp` could also produce PDF files. When producing files, `--gpout` option should be specified to indicate the _final_ output file name. Note that the `postscript` terminal in `gnuplot` produces postscript files. However, in `msp`, the output would be redirected to `ps2pdf` to directly generate the final PDF file.

### Producing different types of plots with manipulated data

```
msp ',x=$date,y=$cost,op=d0,type=linespoints,style=lw 5' -i balance.csv
```

Different from the last example, this command invokes `sp` to apply derivation operator `d0` on the `$cost` column to produce the trend of `$cost`, and then plots the resulting data as lines and points, with line width set to 5.

### Plotting multiple data series

```
msp ',x=$date,y=$cost,op=d0,type=linespoints,title=Alice' \
    ',x=$date,y=$cost,op=d0,type=linespoints,title=Bob' \
    -i balance.alice.csv \
    -i balance.bob.csv
```

Now we are beginning to run into something really different. In this example, `msp` invokes `sp` concurrently to process two input files `balance.alice.csv` and `balance.bob.csv` to calculate the trend of cost for both Alice and Bob. Then, `msp` plots the two data series together onto the same `x11` GUI screen, with their owner's names as the legend.

### Specifying Input files and series-specific options

```
cat balance.alice.csv |
msp ',f=0,x=$date,y=$jul_cost,t=linespoints,l=Alice,s=lc red' \
    ',x=$date,y=$jul_cost,t=linespoints,l=Bob,s=lc blue' \
    ',f=0,x=$date,y=$jul_cost,o=d0,t=linespoints,s=lc red,a=12' \
    ',x=$date,y=$jul_cost,o=d0,t=linespoints,s=lc blue,a=12' \
    -i balance.bob.csv
```

Despite implicitly inferred in most common cases, the input file of each data series could also be explicitly specified, either by the `--file-index` option that overwrites the inferring logic, or by the `file` key in the data series specification. Here the first data series comes from STDIN, whose index number is `0`. Then, the second data series uses the first file specified with `-i` (index `1`) because the default `--file-index` value is `+1`, which indicates that this data series should use the next file index. Then, the third data series resets input index to 0, and again, the last data series leverages the default value to set its input index to `1`.

Next, let's consider the meaning of the data. We are plotting information of two different people, Alice and Bob. Therefore, we should use the same style for the two data series of the same person. Also, the unit of cost and the derivation of cost is not the same, indicating that we should not use a unified y axis for both types of data. Therefore, we specify `style=lc red` and `style=lc blue` for Alice and Bob, respectively. Moreover, we use `axis=12` for derivation data to have them plotted on the y2 axis (12 for x1y2).

### Applying global options

```
cat balance.alice.csv |
msp ',f=0,l=Alice,s=lc red' \
    ',l=Bob,s=lc blue' \
    ',f=0,o=d0,s=lc red,a=12' \
    ',o=d0,s=lc blue,a=12' \
    -i balance.bob.csv \
    --type linespoints --xexpr '$date' --yexpr '$jul_cost' \
    --font Helvetica,24 --xl Date --yl Cost --y2l "Derivation of Cost"
```

`msp` supports customizing default value of all options in the data series specification. This example plots exactly the same data as the previous example. However, the common parts in the data series specification are now replaced by command line options. Also, in this example we also specified various options to ensure the plot uses a pretty font and has appropriate x and y labels.

### Using references in data series specification

```
cat balance.alice.csv |
msp ',f=0,l=Alice,s=lc red' \
    ',l=Bob,s=lc blue' \
    ',f=0,o=d0,rs=+-2,a=12' \
    ',o=d0,rs=2,a=12' \
    -i balance.bob.csv \
    --type linespoints --xexpr '$date' --yexpr '$jul_cost' \
    --font Helvetica,24 --xl Date --yl Cost --y2l "Derivation of Cost"
```

In last example, we greatly reduced the length of the data series specification by using default values. However, the value of `style` is still long, and rewriting it for multiple times may introduce typos. In this example, we use `r[key]` (reference keys) to retrieve value from _previously-seen_ keys. In data series #3, we use the reference `+-2` to refer to the `style` value of data series #(3 - 2); in data series #4, we use the reference `2` to refer to the `style` value of data series #2. Here we note that combining absolute and relative references could make the command confusing, and the recommended practice is to use only one type of reference for one key. We also note that `r[key]` are not real keys, so they do not have default values (thus you could not specify them with command line options!), and `rfile` is illegal, since `file` is already a reference.

### Preparing datasheet files and gnuplot command

```
msp -d (other options)
```

The `-d` option of `msp` causes `msp` to perform a dry-run that does not plot anything. Instead, it would invoke `sp` to generate datasheet files as specified by the other options, and print the gnuplot command it would use otherwise to the terminal. This option acts as a debug measure that allows the user to check the gnuplot command manually, and is also available for generating inputs of larger projects (e.g. a LaTeX project).

## Details

### Operator sequence

`sp` manipulates the input spreadsheet in one of the two ways: 1) _Preprocessing_ that retrieves the x and y columns from the input file according to the `-x` and `-y` options and invokes `mlr` to perform calculation if necessary; 2) _Applying operator sequence_ that performs various transformations on the x and y columns or outputs them in various forms.

`sp` supports two kinds of operators, transform operators (lower case alphabets) and dump operators (upper case alphabets): 

- Transform operators 

    These operators manipulates the input table and generate a new table as output. As indicated by their name, they transforms the input data into a new form. Unlike the preprocessing step that operates on rows, _i.e._ separated data records, transform operators are applied on a series of data covering the whole table.

    `sp` supports the following transform operators:

    - `c`: Cumulative distribution function

        For table `(x, y)`, This operator computes the CDF of `y` and produces table `(y, cdf(y))`. The column names in the new table are `y` and `CDF`.

    - `d`: Derivation

        This operator accepts an argument `window` which specifies the _minimum_ window size for computing derivation. When `window == 0`, raw derivation value is computed. When specifying a non-zero window value, derivation values are generated each time x value increases by at least `window`.

        For table `(x, y)`, This operator computes the derivation of `y` with respect to `x` and produces table `(x, dy/dx)`. The column names in the new table are `x` and `x:Derivation(window)`.

        __NOTE__: This operator automatically sorts the table by x value, and requires the x value to be unique. Otherwise, `sp` would fail.

    - `i`: Integral

        For table `(x, y)`, This operator computes the integral of `y` with respect to `x` and produces table `(x, ∫y dx)`. The column names in the new table are `x` and `x:Integral`.

        __NOTE__: This operator automatically sorts the table by x value, and requires the x value to be unique. Otherwise, `sp` would fail.

    - `m`: Merge

        This operator treats the table as a key-value store, and accumulates the value of each key into their sum. Note that this operator neither sorts the table nor assumes the input data to be sorted. Instead, it only accumulates consecutive records. For example, for the following lines:

        ```
        1, 2
        1, 3
        2, 4
        1, 2
        ```

        `sp` produces:

        ```
        1, 5
        2, 4
        1, 2
        ```

        To accumulate all records in a table, combine this operator with the `o` operator.

        For table `(x, y)`, This operator produces table `(x, ∑y)`. The column names in the new table are `x` and `x:Merge`.

    - `o`: sOrt

        This operator sorts the table by x value.

        For table `(x, y)`, This operator produces table `(x, y)`. Original column names are preserved.

    - `s`: Step (_i.e._ difference of the consecutive y values)

        Despite designed for computing difference values, this operator works on unsorted tables just like the `m` operator.
        
        For table `(x, y)`, This operator produces table `(x, dy)`. The column names in the new table are `x` and `x:Step`.

- Dump operators

    Unlike transform operators, dump operators do not modify the data table. Instead, they perform various forms of output such as plotting and printing the data.

    `sp` supports the following dump operators:

    - `C`: print current state to a Cache file

        see [Store and use intermediate results](#store-and-use-intermediate-results)

    - `O`: Output current table to the terminal

        This operator output the current table to `stdout`, and the output format is also specified with the `-F` option.

    - `p`: Plot current table

        This operator plots the current table with `gnuplot`. By default it generates a scatter plot and prints it onto the terminal. The following `gnuplot` command is used:

        ```        
        set key autotitle columnhead
        set terminal dumb size `tput cols`,`echo $(($(tput lines) - 1))`

        # user-provided gnuplot commands

        plot [file_name] using 1:2
        ```

        The user could customize the plot by using `-g` option to specify gnuplot commands that should be executed before the `plot` command.

        To further customize the plot, the user could use `-G` option to specify a gnuplot script file that completely overrides the default template. `sp` would automatically generate the macro `input_file` to point to the data sheet, so in the script files users may plot data like:

        ```
        plot input_file using 1:2 with lines
        ```

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
                       file-index = REF of data source file
                       opseq = transforms to apply on the data
                       plot-type = plot type of the data series
                       style = plotting style of the data series
                       title = title of the data series
                       xexpr = x-axis expression
                       yexpr = y-axis expression
                       rKEY = KEY's value of series[REF]
                         (rfile_index is illegal)
                   REF = (+)[num]
                     [num]: Absolute index (1-based),
                       (0 for stdin if referring to input file)
                     [+num]: Relative index (current index + num),
                   NOTE: prefix of keys is also supported (e.g. a for axis).
                   Example:
                     ,input=0 => input=stdin
                     |x=${a,}|op=c|a=21 => xexpr="${a,}", opseq="c", axis="21"
                     ,rx=1,ry=+-1 =>
                       xexpr=series[1].xexpr,
                       yexpr=series[current_index - 1].yexpr
    ...
    ```

    `msp` recognizes data series from the data series specification as shown above. A data series is represented by a `delimeter` character and `delimeter`-separated `key=value` expressions following it. The available `key`s are carefully picked to cover `sp`'s data manipulation functionality and common plotting options. 

    The following efforts are made to ensure the convenience and flexibility of `msp`:

    1. **Key-value pairs instead of value list:** To be more flexible, `msp` must support various data series-specific options to tune the behavior of both data processing and plotting. While value lists (e.g. ",1,2,3" for `,input_index=1,xexpr=2,yexpr=3`) are more concise, we found that too many options poses great difficulty for users to remember their order, and misplaced options will generate very confusing output sometimes because the value of one option may also be a valid value of another option. Inspired by Python, we use key-value pairs for specifying options to improve readability.
    
    2. **File indexes instead of file paths:** Each data series is (inevitably) associated with a data source file. However, specifying file names in the data series specification would:
    
        1) Cause the specification to be considerably longer;
        2) Prevent the user from quickly typing file names with tab completion.
        3) Force the user to type the same file name repeatedly for multiple data series referencing the same file.

        Therefore, we use command line arguments to provide file names, which automatically enables tab completion. In data series specification, user only need to write a reference, which is shorter, but not equally readable as plain names (yes, we admit). However, file references are at least better than long paths that makes a simple data series specification longer than one terminal line, we believe :) 

    3. **Default values and abbreviations:** A critical drawback of having too many options is that specifying all of them would make the data series specification extremely long. Therefore, `msp` supports using abbreviations for keys like many other commands does, provides every option a default value, and supports customizing all default values with command line options. 
    
    4. **References:** To further avoid specifying the same options for multiple data series, `msp` also supports a unified reference format (`[REF]` in the help message) for forward-referencing previous option values. Notably, `input-path` also uses the reference format, but instead of referencing the value in another data series, `input-path` references to input files. We set the default value of `input-path` to `+1` to automatically infer file indexes for the common case where each data series comes from a separated file. We note that we force relative references to be `(+)[num]` reference to "the last one" would look like `+-1`. The reason that `-1` is not used here is that for specifying the `--file-index` option, the value `-1` would be interpreted as another command line option.
    
    5. **`delimeter` instead of escaping characters:** Given that `xexpr` and `yexpr` may contain arbitrary characters, it is not feasible to use any fixed delimeters to separate the key-value pairs without escaping them. However, in practice (especially for shell programming), escaping characters is indeed a catastrophe when generating command line arguments from code or passing them through programs. Moreover, reversely-escaping characters in original arguments manually is both exhausting and error-prone (why not use another program? This introduces another layer of escaping!). Therefore, inspired by `sed`, we use user-provided delimeter to divide the key-value expressions. We have three advantages here: 
    
        1) **No escaping**, of course; 
        2) **Readability not reduced**: we introduce only one extra meaningless character to implement a extremely-simple working parser; 
        3) **Supports arbitrary in-expression characters**: remember that `rust` is UTF-8 native, and there are enough choices to ensure that `delimeter` would not present in the expressions.

- Plot style

    `msp` uses command line options to specify global settings that applies to all data series and plotting options. As for the plot style, `msp` supports two levels of customization:
    
    1. **Common use:** `msp` uses a hard-coded gnuplot template with various customizable parts such as terminal type, font and x/y range. Users may modify the default behavior of `msp` with corresponding command line options. For special behavior (e.g. `set logscale x`), users could also insert arbitrary gnuplot command just before the `plot` command via the `-g` option.
    
    2. **Advanced use:** `msp` allows users to provide a custom gnuplot script file to override the default template. Users may use the macro `ds_{input_index}` to point to the data sheets, and plot data like:
    
        ```
        plot ds_1 using 1:2 with lines
        ```

        Definition of `ds_{input_index}` macros would be generated by `msp` automatically and would be placed before the user-provided script.

    3. **Fine-tune of the default template:** `msp` provides a dry-run mode to prepare everything it needs to generate the plot. Users may use the `-d` option to prepare the data files and print the generated gnuplot command to stdout. This enables the user to check what is happening beneath `msp` and derive their own gnuplot commands from the default template (e.g. plot an additional function). 
    We also recognize this as an important measure for users to stay close with the `gnuplot` language, given the fact that convenient shorthands would easily cause us to forget the details :)
    