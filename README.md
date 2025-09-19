# Spreadsheet Plotter

Spreadsheet Plotter (`sp`) is a linux command-line tool that takes spreadsheets as input, manipulates the data mathematically and generates plots with `gnuplot`. With customized behavior of saving intermidiate results and processing the data, `sp` also supports performing pure mathematical transformation on spreadsheets, quickly re-plotting data without processing the original spreadsheet again, and customizing the apperance of output plots. 

The workflow of `sp` is focused on "operator sequences", which could be represented with a string. Each single alphabet in the string is an operator that causes `sp` to manipulate the data or dump outputs, operators may have comma-separated arguments (numbers only) that directly follows them. `sp` iterates through the operator sequence and executes each of them. 

## Build

`sp` could be built with a single `cargo build --release` command. However, to run `sp`, you must also install `gnuplot` and `mlr` (>= 6.0) on your system.

## Quick Examples

We first offer a quick reference to `sp` by showing its functionalities with examples.

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

Sometimes, specifying the gnuplot command in the command line is not convenient. Also, `-g` could not control the `plot` command. Therefore, `-g` lacks the ability of controlling the plot type, generating multiple plots, etc. In this case, `sp` also supports reading gnuplot commands from a file. The file would be specified with the `-G` option. For example, `-G "1.gp"` would execute `gnuplot` with `1.gp` instead.

### Replot

```
sp -r -g 'set xrange [0:1000]'
```

`sp` stores the spreadsheet data used in the previous plot command in a special temporary file. To conveniently re-plot the data with a different plot command, `sp` provides the `-r` option. When `-r` is provided, `sp` simply checks for existence of such temporary file and re-plot the data with the provided `gnuplot` command (via `-g` or `-G`).

### Pre-processing data

```
sp -i input.csv -e "P" -x '$1' -y '${column_name} * 2 + 3'
```

`sp` supports data pre-processing by invoking `mlr` on the input data. Any valid `mlr` expression could be used as the x and y values. During the pre-processing, `sp` first checks for simple expressions that could be handled with lower overhead: `sp` could detect simple column references (covers common usage like `$name` or `${name}`) and pure mathematical expressions that do not involve any column references. If both of the axis are simple expressions, `sp` would not call `mlr` to process the input data (for mathematical expressions, `mlr` may still be used as a calculator) and will directly read from the source file. 

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

<!-- ```
sp --icprefix "sp-" -e "id1000cP"
```

For cases where multiple cache files are available, we could use the `--icprefix` option to specify the prefix of the cache file names. `sp` would automatically find all files with the specified prefix and choose the one that best matches the provided operator sequence using longest-prefix matching. Note that dump operators (upper case) are not counted in such matching. Here we note that `sp` requires all such files to be valid cache files and their original datasheet file must be the same, or `sp` would fail. -->


### Performing mathematical transformation

```
sp -i input.csv -F csv -e "id1000O" -x '$1' -y '$2'
```

In some cases, we may only want to perform pure mathematical transformation on spreadsheets and generate input data for other tools. To achieve this, we only need to change the final operator from `P` to `O`, which prints the result datasheet to the terminal. Here the output format is specified with the `-F` option. The default output format is also `csv`, so `-F csv` is not mandatory in this example.

## Usage

### Calling interface

Invoking `sp` with `-h` option would provide a detailed help message describing its calling interface.

### Operator sequence

Operator sequence is the other way for `sp` to perform data transformation. Before processing the operator sequence, `sp` first performs preprocessing by retrieving the data source from the input file according to the `-x` and `-y` options. The preprocessing procedure would generate a table with two columns (x and y).

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
