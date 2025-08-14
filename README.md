# Spreadsheet Plotter

Spreadsheet Plotter (`sp`) is a linux command-line tool that takes spreadsheets as input, manipulates the data mathematically and generates plots with `gnuplot`. With customized behavior of saving intermidiate results and generating plots, `sp` also supports performing pure mathematical transformation on spreadsheets, quickly re-plotting data without processing the original spreadsheet again, and customizing the apperance of output plots. 

The workflow of `sp` is focused on "operator sequences", which could be represented with a string. Each single alphabet in the string is an operator that causes `sp` to manipulate the data or dump outputs, operators may have comma-separated arguments (numbers only) that directly follows them. `sp` iterates through the operator sequence and executes each of them. 

## Quick Examples

We first offer a quick reference to `sp` by showing its functionalities with examples.

### Plotting a scatter plot

```
sp -i input.csv -e "P" -x '#1' -y '#2'
```

This command reads `input.csv` and plots the data as a scatter plot with the 2nd column as y axis and the 1st column as x axis. The plot would be directly printed (operator `P`) onto the terminal.

### Customizing the appearance of plots

```
sp -i input.csv -e "P" -x '#1' -y '#2' -g "set xrange [0:1000]"
```

`sp` supports customizing the appearance of plots with the `-g` option. The argument of `-g` is a string that would be directly passed to `gnuplot` and executed before the `plot` command.

Sometimes, specifying the gnuplot command in the command line is not convenient. Also, `-g` could control the `plot` command. Therefore, `-g` lacks the ability of controlling the plot type, generating multiple plots, etc. In this case, `sp` also supports reading gnuplot commands from a file. The file would be specified with the `-G` option. For example, `-G "1.gp"` would execute `gnuplot` with `1.gp` instead.

### Using alternative column names

```
sp -i input.csv -e "P" -x '#a' -y '#B'
```

In some cases, the column numbers in the spreadsheet are hard to remember. For example, Microsoft Excel uses column names `A`, `B`, `C`, etc. to represent the 1st, 2nd, 3rd columns, respectively. In this case, `sp` also supports using column names as axis labels. Note that `sp`'s column names are case-insensitive.

```
sp -i input.csv -e "P" -H -x '@timestamp@' -y '@size\@packet@'
```

Sometimes, the column names in the spreadsheet are not very descriptive. In this case, `sp` also supports using column headers as column names. For example, the command above would plot the data with `timestamp` as x axis and `size` as y axis. `sp` differentiates column headers and column names by their prefixes. Column headers are quoted with `@`, while column names start with `#`. Also note that `\` is used to escape any characters that follows it, thus `@` in column names becomes `\@`, while `\` becomes `\\`.

For `sp` to recognize column headers, we must also specify the `-H` option for `sp` to know about the presence of column headers in the input file. Note that if `-H` is used, the column headers must be unique, or `sp` would fail.

### Plotting transformed data

```
sp -i input.csv -e "id1000P" -x '#1' -y '#2'
```

Slightly different from the first example, this command first computes __integral__ (operator `i`) of the original data and then computes __derivation__ on a smooth window of 1000 (operator `d`) of the integral. Finally, it plots the result onto the terminal.

Consider the case where we store a network trace in `input.csv` with two columns, the 1st column is timestamps in microseconds and the 2nd column is the size of packets received at the corresponding time. The example above would transform the original <time, packet size> pairs into <time, amount of received data> pairs by computing integral, and then produce the <time, throughput> pair by computing derivation (smoothed out to a 1ms time window).

### Pre-processing data

```
sp -i input.csv -e "iP" -x '#1' -y '#2 ^ 0.5 * (@time_len@ / @size@)'
```

In addition to using the values from a single column, `sp` also supports computing x and y values from the original data rows. `sp` supports basic arithmetic operators like `+`, `-`, `*` and `/`. Power (`^`) and residual (`%`) operations are also available. Note here that `sp` treats all field value and instant number in the expression as `f64`s, and residual operations are based on that: for example, `3 % 2.4` evaluates to `0.6`. Another important point to remember is that `sp` does _NOT_ support non-finite numbers and would fail instantly upon encountering one, so be careful about divisions and the possibility of overflowing!

### Store intermediate results as spreadsheets and re-plotting

```
sp -i input.csv -o . -e "iCd1000CP" -x '#1' -y '#2'
```

Again, this example is only slightly different from the last one. The only difference in the operation sequence is the `C` operators that causes `sp` to store _current_ intermediate results. In this example, before moving on to compute the derivation, `sp` firstly dumps the <time, amount of received data> to a spreadsheet file; also, before plotting, `sp` dumps the final result to a spreadsheet file.

You may have noticed the `-o` option. It is used to specify the directory that the output files would be stored. If not specified, the current directory would be used. The name of generated spreadsheet files would be named as `[op].spds`, where `[op]` is the operator sequence needed for generating this file. If at least one output file was generated, `sp` would also generate a text file, `splnk`, which records the path of the original datasheet file and the output directory.

With the help of `splnk`, `sp` is also capable of quickly re-plotting as shown in the next example:

```
sp -i splnk -f lnk -o . -e "id1000cP"
```

In this example, we would like to generate a CDF (operator `c`) plot --- take the network throughput case as an example, this time we will generate the distribution of network throughput instead of the raw time series.

To achieve this, `sp` first reads the `-f` (input format) option to know that we are providing a link file. Note that `-f` option defaults to `csv`, so `-f csv` is not needed in previous examples. Subsequently, it find out the path of the original datasheet file and the output directory from `splnk`. Then, it compares its operator sequence against the name of available `*.spds` files to find out the _best_ intermediate result file to use --- `id1000.spds` would be preferred over `i.spds` in this example. In this way, `sp` is able to reuse intermediate results to accelerate further data transformation. Note here that `sp` reads original x and y expressions from `splnk` so `-x` and `-y` options are not necessary.

In addition to data transformation, intermediate results could also be directly re-used to generate plots with different appearance. To achieve this, simply store the intermediate results and invoke `sp` with a different `-g`/`-G` argument later.

### Performing mathematical transformation

```
sp -i input.csv -F csv -e "id1000O" -x '#1' -y '#2'
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

    - `C`: output current table to a Cache file

        This operator dumps the current table to a cache file named `[output_dir]/[opseq].spds`. The format of the cache file is specified with the `-F` option. Here `[opseq]` is the operator sequence executed before current `C` operator and `[output_dir]` is specified with the `-o` option. The output files could be used conveniently by other programs.

        In addition to the cache files, `sp` also creates a text file named `splnk` in the output directory when the first `C` operator is executed. As demonstrated in the Quick Examples part, `splnk` contains information for `sp` to reload the execution status of previous sessions. 

        The most important feature of `splnk` is that it enables `sp` to automatically match the operator sequence with the cache files and find the best one. Consider the case where some previous `sp` sessions operated on the operator sequence `iCd0CcC`, and now we would like to execute the operator sequence `id0s`. The `splnk` file generated in the previous session would record that 3 cache files are available: `iCd0Cc.spds`, `iCd0.spds` and `i.spds`. `sp` would automatically find the best one, which is `iCd0.spds`, and load it. The cache loading procedure is as follows: 

        > 1. Ignore all dump operators in both current operator sequence and the `splnk` file.
        > 2. Find the longest operator sequence in the `splnk` file that is a prefix of current operator sequence.
        > 3. Load the corresponding cache file
        > 4. Find the last operator in current operator sequence that matched the operator sequence in the `splnk` file.
        > 5. Ignore such operator and all operators before it in current operator sequence.

        In this way, `sp` could perform matching even if the operator sequence contains different sequences of dump operators.

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
