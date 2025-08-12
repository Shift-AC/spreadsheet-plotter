# Spreadsheet Plotter

Spreadsheet Plotter (`sp`) is a linux command-line tool that takes spreadsheets as input, manipulates the data mathematically and generates plots with `gnuplot`. With customized behavior of saving intermidiate results and generating plots, `sp` also supports performing pure mathematical transformation on spreadsheets, quickly re-plotting data without processing the original spreadsheet again, and customizing the apperance of output plots. 

The workflow of `sp` is focused on "operator sequences", which could be represented with a string. Each single alphabet in the string is an operator that causes `sp` to manipulate the data or dump outputs, operators may have comma-separated arguments (numbers only) that directly follows them. `sp` iterates through the operator sequence and executes each of them. 

## Examples of usage

We first offer a quick reference to `sp` by showing its functionalities with examples.

### Plotting a scatter plot

```
sp -i input.csv -t "P" -x 1 -y 2
```

This command reads `input.csv` and plots the data as a scatter plot with the 2nd column as y axis and the 1st column as x axis. The plot would be directly printed (operator `P`) onto the terminal.

### Customizing the appearance of plots

```
sp -i input.csv -t "P" -x 1 -y 2 -g "set xrange [0:1000]"
```

`sp` supports customizing the appearance of plots with the `-g` option. The argument of `-g` is a string that would be directly passed to `gnuplot` and executed before the `plot` command.

Sometimes, specifying the gnuplot command in the command line is not convenient. In this case, `sp` also supports reading gnuplot commands from a file. The file would be specified with the `-G` option. For example, `-G "gnuplot.gp"` would read the gnuplot commands from `gnuplot.gp` instead.

### Using alternative column names

```
sp -i input.csv -t "P" -x a -y b
```

In some cases, the column numbers in the spreadsheet are hard to remember. For example, Microsoft Excel uses column names `A`, `B`, `C`, etc. to represent the 1st, 2nd, 3rd columns, respectively. In this case, `sp` also supports using column names as axis labels. Note that `sp`'s column names are case-insensitive.

```
sp -i input.csv -t "P" -H -X timestamp -Y size
```

Sometimes, the column names in the spreadsheet are not very descriptive. In this case, `sp` also supports using column headers as column names. For example, the command above would plot the data with `timestamp` as x axis and `size` as y axis. For `-X` and `-Y` to work, we must also specify the `-H` option for `sp` to know about the presence of the column header. Note that if `-X` or `-Y` is used, the column header must be unique, or `sp` would fail.

### Plotting transformed data

```
sp -i input.csv -t "id1000P" -x 1 -y 2
```

Slightly different from the first example, this command first computes __integral__ (operator `i`) of the original data and then computes __derivation__ on a smooth window of 1000 (operator `d`) of the integral. Finally, it plots the result onto the terminal.

Consider the case where we store a network trace in `input.csv` with two columns, the 1st column is timestamps in microseconds and the 2nd column is the size of packets received at the corresponding time. The example above would transform the original <time, packet size> pairs into <time, amount of received data> pairs by computing integral, and then produce the <time, throughput> pair by computing derivation (smoothed out to a 1ms time window).

### Store intermediate results as spreadsheets and re-plotting

```
sp -i input.csv -o . -t "iCd1000CP" -x 1 -y 2
```

Again, this example is only slightly different from the last one. The only difference in the operation sequence is the `C` operators that causes `sp` to store _current_ intermediate results. In this example, before moving on to compute the derivation, `sp` firstly dumps the <time, amount of received data> to a spreadsheet file; also, before plotting, `sp` dumps the final result to a spreadsheet file.

You may have noticed the `-o` option. It is used to specify the directory that the output files would be stored. If not specified, the current directory would be used. The name of generated spreadsheet files would be named as `[op].spds`, where `[op]` is the operator sequence needed for generating this file. If at least one output file was generated, `sp` would also generate a text file, `splnk`, which records the path of the original datasheet file and the output directory.

With the help of `splink`, `sp` is also capable of quickly re-plotting as shown in the next example:

```
sp -i splnk -f lnk -o . -t "id1000cP" -x 1 -y 2
```

In this example, we would like to generate a CDF (operator `c`) plot --- take the network throughput case as an example, this time we will generate the distribution of network throughput instead of the raw time series.

To achieve this, `sp` first reads the `-f` (input format) option to know that we are passing a link file. Note that `-f` option defaults to `csv`, so `-f csv` is not needed in previous examples. Subsequently, it find out the path of the original datasheet file and the output directory from `splnk`. Then, it compares its operator sequence against the name of available `*.spds` files to find out the _best_ intermediate result file to use --- `id1000.spds` would be preferred over `i.spds` in this example. In this way, `sp` is able to reuse intermediate results to accelerate further data transformation.

Besides complex data transformation, intermediate results could also be directly re-used to generate plots with different appearance. To achieve this, simply store the intermediate results and invoke `sp` with a different `-g`/`-G` argument later.

### Performing mathematical transformation

```
sp -i input.csv -F csv -t "id1000O" -x 1 -y 2
```

In some cases, we may only want to perform pure mathematical transformation on spreadsheets and generate input data for other tools. To achieve this, we only need to change the final operator from `P` to `O`, which prints the result datasheet to the terminal. Here the output format is specified with the `-F` option. The default output format is also `csv`, so `-F csv` is not mandatory in this example.
