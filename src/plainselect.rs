use anyhow::anyhow;
use regex::{Captures, Regex};

pub struct Expr {
    raw_expr: String,
    index_pattern: Regex,
}

impl Expr {
    pub fn new(raw_expr: &str, index_mark: char) -> Self {
        let index_mark = match index_mark {
            '-' | '\\' => format!("\\{}", index_mark),
            _ => index_mark.to_string(),
        };
        Self {
            raw_expr: raw_expr.to_string(),
            index_pattern: Regex::new(&format!(r"[{}]\d+", index_mark))
                .unwrap(),
        }
    }

    /// Get a list of indexes referenced by this expression
    fn get_required_indexes(&self) -> anyhow::Result<IndexList> {
        self.index_pattern
            .find_iter(&self.raw_expr)
            .try_fold(Vec::new(), |mut acc, caps| {
                let index = &caps.as_str()[1..];
                match index.parse::<usize>() {
                    Ok(0) | Err(_) => Err(anyhow!(
                        "Invalid index {} at char {}",
                        index,
                        caps.range().start
                    )),
                    Ok(val) => {
                        acc.push(val);
                        Ok(acc)
                    }
                }
            })
            .map(|acc| IndexList {
                indexes: acc,
                prefix: "col".to_string(),
            })
    }

    fn to_sql(&self, index_list: &IndexList) -> String {
        let escaped = self.raw_expr.replace("\"", "\"\"");
        self.index_pattern
            .replace_all(&escaped, |caps: &Captures| {
                let index = caps[0][1..].parse::<usize>().unwrap();
                format!(
                    "COLUMNS(getvariable('{}_{}'))",
                    index_list.prefix, index
                )
            })
            .to_string()
    }
}

pub struct IndexList {
    indexes: Vec<usize>,
    prefix: String,
}

impl Default for IndexList {
    fn default() -> Self {
        Self::new("col")
    }
}

impl IndexList {
    pub fn new(prefix: impl AsRef<str>) -> Self {
        Self {
            indexes: Vec::new(),
            prefix: prefix.as_ref().to_string(),
        }
    }

    pub fn merge(&mut self, other: Self) {
        self.indexes.extend(other.indexes);
    }

    pub fn simplify(&mut self) {
        self.indexes.sort();
        self.indexes.dedup();
    }

    fn generate_index_variables(&self, src_table: &str) -> String {
        self.indexes
            .iter()
            .map(|i| format!("SET VARIABLE {}_{} = (SELECT name FROM pragma_table_info('{}') WHERE cid = {});\n", self.prefix, i, src_table, i - 1))
            .collect::<Vec<_>>()
            .join("")
    }

    fn generate_clean(&self) -> String {
        format!(
            "DROP MACRO get_col_name;\n{}",
            self.indexes
                .iter()
                .map(|i| format!("RESET VARIABLE {}_{};\n", self.prefix, i))
                .collect::<Vec<_>>()
                .join("")
        )
    }

    fn generate_preamble(&self, src_table: &str) -> String {
        format!(
            "CREATE MACRO get_col_name(x integer) AS (SELECT name FROM pragma_table_info('{}') WHERE cid = x);\n{}",
            src_table,
            self.generate_index_variables(src_table)
        )
    }
}

pub struct PlainSelector {
    xexpr: Expr,
    yexpr: Expr,
    pre_filter: Option<Expr>,
    pre_index_list: IndexList,
    post_filter: Option<Expr>,
    post_index_list: IndexList,
}

impl PlainSelector {
    pub fn new(
        xexpr: Expr,
        yexpr: Expr,
        pre_filter: Option<Expr>,
        post_filter: Option<Expr>,
    ) -> anyhow::Result<Self> {
        let mut pre_index_list = IndexList::default();
        pre_index_list.merge(xexpr.get_required_indexes()?);
        pre_index_list.merge(yexpr.get_required_indexes()?);
        if let Some(ref filter) = pre_filter {
            pre_index_list.merge(filter.get_required_indexes()?);
        }
        pre_index_list.simplify();
        let mut post_index_list = IndexList::default();
        if let Some(ref filter) = post_filter {
            post_index_list.merge(filter.get_required_indexes()?);
        }
        post_index_list.simplify();
        Ok(Self {
            xexpr,
            yexpr,
            pre_filter,
            post_filter,
            pre_index_list,
            post_index_list,
        })
    }

    pub fn to_preprocess_sql(
        &self,
        src_table: &str,
        dst_table: &str,
    ) -> String {
        let query = format!(
            "CREATE TABLE {} AS SELECT {} AS x, {} AS y FROM {}{};\n",
            dst_table,
            self.xexpr.to_sql(&self.pre_index_list),
            self.yexpr.to_sql(&self.pre_index_list),
            src_table,
            if let Some(ref filter) = self.pre_filter {
                format!(" WHERE {}", filter.to_sql(&self.pre_index_list))
            } else {
                "".to_string()
            }
        );

        let cleanup = format!(
            "DROP TABLE {};\n{}",
            src_table,
            self.pre_index_list.generate_clean()
        );

        format!(
            "{}{}{}",
            self.pre_index_list.generate_preamble(src_table),
            query,
            cleanup
        )
    }

    pub fn to_postprocess_sql(&self, src_table: &str) -> String {
        format!(
            "SELECT * FROM {}{};\n",
            src_table,
            if let Some(ref filter) = self.post_filter {
                format!(" WHERE {}", filter.to_sql(&self.post_index_list))
            } else {
                "".to_string()
            }
        )
    }
}
