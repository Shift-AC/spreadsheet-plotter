#[cfg(feature = "preprocess")]
mod datainput;
#[cfg(feature = "preprocess")]
mod opeseq;
#[cfg(feature = "preprocess")]
mod plainselect;

#[cfg(feature = "gnuplot")]
mod plotscript;
#[cfg(feature = "gnuplot")]
mod plotter;

#[cfg(feature = "preprocess")]
pub use datainput::DataFormat;
#[cfg(feature = "preprocess")]
pub use datainput::DataInput;
#[cfg(feature = "preprocess")]
pub use opeseq::OpSeq;
#[cfg(feature = "preprocess")]
pub use plainselect::Expr;
#[cfg(feature = "preprocess")]
pub use plainselect::PlainSelector;

#[cfg(feature = "gnuplot")]
pub use plotscript::AxisOptions;
#[cfg(feature = "gnuplot")]
pub use plotscript::Color;
#[cfg(feature = "gnuplot")]
pub use plotscript::DataSeriesOptions;
#[cfg(feature = "gnuplot")]
pub use plotscript::GnuplotTemplate;
#[cfg(feature = "gnuplot")]
pub use plotscript::LineStyle;
#[cfg(feature = "gnuplot")]
pub use plotscript::PlotType;
#[cfg(feature = "gnuplot")]
pub use plotscript::PointStyle;
#[cfg(feature = "gnuplot")]
pub use plotscript::Terminal;
#[cfg(feature = "gnuplot")]
pub use plotter::DataPoints;
#[cfg(feature = "gnuplot")]
pub use plotter::DataSeriesSource;
#[cfg(feature = "gnuplot")]
pub use plotter::Plotter;
