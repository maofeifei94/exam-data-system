//! 流程引擎：把「可编辑的原子操作符流程」编译成 SQL，并交给 SQLite 执行。
//!
//! 每个步骤都会生成一段命名 CTE（s0、s1、s2 ...），最终拼成一条完整 SQL：
//!   WITH s0 AS (SELECT "c0" AS "身份证号", ... FROM "data_xxx"),
//!        s1 AS (SELECT *, ROUND(("笔试成绩" * 0.6) + ("面试成绩" * 0.4), 2) AS "总成绩" FROM s0),
//!        s2 AS (SELECT *, RANK() OVER (ORDER BY "总成绩" DESC) AS "排名" FROM s1)
//!   SELECT * FROM s2 ORDER BY "总成绩" DESC
//! 这条 SQL 会写进运行日志，用户可以直观看到流程做了什么。

pub mod expr;
pub mod operators;

use crate::db::{self, new_id, now};
use crate::error::{AppError, AppResult};
use crate::models::{
    DatasetColumn, FlowStep, RunLogEntry, RunResult, StepData, StepFlow, ValidateError,
    ValidateResult,
};
use crate::store;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;

const PREVIEW_ROWS: i64 = 200;
/// 节点数据预览单次最多返回的行数，防止误传超大 limit 拖垮接口。
const MAX_PREVIEW_ROWS: i64 = 2000;

struct Compiler<'a> {
    conn: &'a Connection,
    columns: Vec<DatasetColumn>,
    ctes: Vec<String>,
    cur: String,
    next: usize,
    order_by: Option<String>,
    limit: Option<i64>,
    output_name: Option<String>,
    log: Vec<RunLogEntry>,
}

fn q(name: &str) -> String {
    expr::quote_column(name)
}

fn str_param(params: &Value, key: &str) -> String {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn num_param(params: &Value, key: &str) -> Option<i64> {
    params.get(key).and_then(|v| match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    })
}

fn bool_param(params: &Value, key: &str, default: bool) -> bool {
    params.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn list_param(params: &Value, key: &str) -> Vec<String> {
    params
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn defaulted(value: String, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

fn f64_param(params: &Value, key: &str) -> Option<f64> {
    params.get(key).and_then(|v| match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    })
}

/// 读取多级排序配置：`keys: [{field, order}]`；为空时回退到旧的 `by` + `order`。
fn sorts_param(params: &Value) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(arr) = params.get("keys").and_then(|v| v.as_array()) {
        for item in arr {
            let field = item
                .get("field")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if field.is_empty() {
                continue;
            }
            let order = match item.get("order").and_then(|v| v.as_str()) {
                Some("asc") => "ASC",
                _ => "DESC",
            };
            out.push((field, order.to_string()));
        }
    }
    if out.is_empty() {
        let by = str_param(params, "by");
        if !by.is_empty() {
            let order = if str_param(params, "order") == "asc" {
                "ASC"
            } else {
                "DESC"
            };
            out.push((by, order.to_string()));
        }
    }
    out
}

struct AggSpec {
    field: String,
    func: String,
    out: String,
    decimals: i64,
}

fn aggregations_param(params: &Value) -> Vec<AggSpec> {
    let mut out = Vec::new();
    if let Some(arr) = params.get("aggregations").and_then(|v| v.as_array()) {
        for item in arr {
            let func = item
                .get("func")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if func.is_empty() {
                continue;
            }
            out.push(AggSpec {
                field: item
                    .get("field")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                func,
                out: item
                    .get("out")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                decimals: item.get("decimals").and_then(|v| v.as_i64()).unwrap_or(2),
            });
        }
    }
    out
}

/// 把任意 SQL 值转成文本；整数值不带 `.0`，避免考场号 1 显示成 1.0。
fn text_sql(var: &str) -> String {
    format!(
        "(CASE WHEN TYPEOF({var}) IN ('integer', 'real') AND {var} = CAST({var} AS INTEGER) \
         THEN CAST(CAST({var} AS INTEGER) AS TEXT) ELSE CAST({var} AS TEXT) END)"
    )
}

/// 条件赋值的取值：数字 / `[字段]` / 带引号的表达式按公式编译，其余按普通文字处理。
fn value_or_expr(raw: &str, cols: &[String]) -> Result<String, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok("NULL".to_string());
    }
    let quoted = text.starts_with('\'') && text.ends_with('\'');
    if text.starts_with('[') || quoted || text.parse::<f64>().is_ok() {
        expr::compile(text, cols)
    } else {
        Ok(format!("'{}'", text.replace('\'', "''")))
    }
}

/// 规划「关联数据表」的输出列：返回（右表字段名, 输出字段名, 类型）。
///
/// 重名时自动追加 `_2`、`_3`；配置了前缀则统一加前缀。
fn join_output_plan(
    left_names: &[String],
    right: &[DatasetColumn],
    requested: &[String],
    prefix: &str,
) -> Result<Vec<(String, String, String)>, String> {
    let mut picked: Vec<&DatasetColumn> = Vec::new();
    if requested.is_empty() {
        picked.extend(right.iter());
    } else {
        for name in requested {
            let Some(col) = right.iter().find(|c| &c.name == name) else {
                return Err(format!("关联表中没有字段 [{name}]"));
            };
            if !picked.iter().any(|c| c.name == col.name) {
                picked.push(col);
            }
        }
    }
    let mut used: Vec<String> = left_names.to_vec();
    let mut out = Vec::new();
    for col in picked {
        let mut name = format!("{prefix}{}", col.name);
        if used.contains(&name) {
            let mut n = 2;
            while used.contains(&format!("{name}_{n}")) {
                n += 1;
            }
            name = format!("{name}_{n}");
        }
        used.push(name.clone());
        out.push((col.name.clone(), name, col.dtype.clone()));
    }
    Ok(out)
}

fn agg_label(func: &str, field: &str) -> String {
    match func {
        "count" => "数量".to_string(),
        "count_distinct" => "去重数量".to_string(),
        "sum" => "合计".to_string(),
        "avg" => "平均值".to_string(),
        "min" => "最小值".to_string(),
        "max" => "最大值".to_string(),
        "stddev" => "标准差".to_string(),
        "variance" => "方差".to_string(),
        "group_concat" => format!("{field}汇总"),
        _ => format!("{func}({field})"),
    }
}

impl<'a> Compiler<'a> {
    fn new(conn: &'a Connection, dataset_id: &str) -> AppResult<Self> {
        let columns = store::get_columns(conn, dataset_id)?;
        if columns.is_empty() {
            return Err(AppError::bad_request("数据集没有任何字段"));
        }
        // 流程表达式会用到的业务函数（身份证解析、金额大写等）按连接注册。
        crate::db::register_functions(conn)
            .map_err(|e| AppError::internal_with_context("注册流程函数失败", e))?;
        let tname = crate::db::table_name(dataset_id)?;
        let items: Vec<String> = columns
            .iter()
            .enumerate()
            .map(|(i, c)| format!("\"c{i}\" AS {}", q(&c.name)))
            .collect();
        let cte = format!("s0 AS (SELECT {} FROM \"{}\")", items.join(", "), tname);
        Ok(Compiler {
            conn,
            columns,
            ctes: vec![cte],
            cur: "s0".to_string(),
            next: 1,
            order_by: None,
            limit: None,
            output_name: None,
            log: Vec::new(),
        })
    }

    /// 把当前 CTE 链 + 一段查询语句拼成可执行 SQL。
    fn with_ctes(&self, body: &str) -> String {
        format!("WITH {} {}", self.ctes.join(", "), body)
    }

    fn final_sql(&self) -> String {
        let mut s = format!("SELECT * FROM {}", self.cur);
        if let Some(o) = &self.order_by {
            s.push_str(&format!(" ORDER BY {o}"));
        }
        if let Some(n) = self.limit {
            s.push_str(&format!(" LIMIT {n}"));
        }
        self.with_ctes(&s)
    }

    fn count_current(&self) -> Result<i64, String> {
        let sql = self.with_ctes(&format!("SELECT COUNT(*) FROM {}", self.cur));
        self.conn
            .query_row(&sql, [], |r| r.get::<_, i64>(0))
            .map_err(|e| format!("统计当前步骤行数失败: {e}"))
    }

    fn push_cte(&mut self, body: String) -> String {
        let name = format!("s{}", self.next);
        self.next += 1;
        let full = format!("{name} AS ({body})");
        self.ctes.push(full.clone());
        self.cur = name;
        full
    }

    fn project(
        &self,
        from: &str,
        replace: &[(String, String)],
        appends: &[(String, String)],
    ) -> String {
        let mut items: Vec<String> = Vec::new();
        for c in &self.columns {
            let qq = q(&c.name);
            if let Some((_, e)) = replace.iter().find(|(n, _)| n == &c.name) {
                items.push(format!("{e} AS {qq}"));
            } else {
                items.push(format!("{qq} AS {qq}"));
            }
        }
        for (n, e) in appends {
            items.push(format!("{e} AS {}", q(n)));
        }
        format!("SELECT {} FROM {}", items.join(", "), from)
    }

    fn has_column(&self, name: &str) -> bool {
        self.columns.iter().any(|c| c.name == name)
    }

    fn add_column(&mut self, col: DatasetColumn) {
        self.columns.push(col);
    }

    fn rename_column(&mut self, from: &str, to: &str) {
        if let Some(c) = self.columns.iter_mut().find(|c| c.name == from) {
            c.name = to.to_string();
        }
    }

    fn log_entry(
        &self,
        step_index: i64,
        step: &FlowStep,
        status: &str,
        rows: i64,
        sql: Option<String>,
        message: Option<String>,
    ) -> RunLogEntry {
        RunLogEntry {
            step_index,
            operator: step.operator.clone(),
            label: if step.label.is_empty() {
                operators::label_of(&step.operator)
            } else {
                step.label.clone()
            },
            status: status.to_string(),
            rows,
            sql,
            message,
        }
    }

    fn step(&mut self, step_index: i64, step: &FlowStep) -> Result<RunLogEntry, String> {
        let params = if step.params.is_null() {
            Value::Object(Default::default())
        } else {
            step.params.clone()
        };
        match step.operator.as_str() {
            "require_fields" => self.op_require_fields(step_index, step, &params),
            "unique_check" => self.op_unique_check(step_index, step, &params),
            "map_columns" => self.op_map_columns(step_index, step, &params),
            "rename" => self.op_rename(step_index, step, &params),
            "select" => self.op_select(step_index, step, &params),
            "cast" => self.op_cast(step_index, step, &params),
            "fill_null" => self.op_fill_null(step_index, step, &params),
            "compute" => self.op_compute(step_index, step, &params),
            "rank" => self.op_rank(step_index, step, &params),
            "filter" => self.op_filter(step_index, step, &params),
            "dedupe" => self.op_dedupe(step_index, step, &params),
            "sort" => self.op_sort(step_index, step, &params),
            "limit" => self.op_limit(step_index, step, &params),
            "random_number" => self.op_random_number(step_index, step, &params),
            "random_rows" => self.op_random_rows(step_index, step, &params),
            "group_aggregate" => self.op_group_aggregate(step_index, step, &params),
            "round_number" => self.op_round_number(step_index, step, &params),
            "text_concat" => self.op_text_concat(step_index, step, &params),
            "text_substr" => self.op_text_substr(step_index, step, &params),
            "text_pad" => self.op_text_pad(step_index, step, &params),
            "text_replace" => self.op_text_replace(step_index, step, &params),
            "conditional" => self.op_conditional(step_index, step, &params),
            "date_calc" => self.op_date_calc(step_index, step, &params),
            "parse_id_card" => self.op_parse_id_card(step_index, step, &params),
            "amount_upper" => self.op_amount_upper(step_index, step, &params),
            "percentage" => self.op_percentage(step_index, step, &params),
            "tax_calc" => self.op_tax_calc(step_index, step, &params),
            "budget_variance" => self.op_budget_variance(step_index, step, &params),
            "loan_payment" => self.op_loan_payment(step_index, step, &params),
            "reconcile" => self.op_reconcile(step_index, step, &params),
            "depreciation" => self.op_depreciation(step_index, step, &params),
            "allocate" => self.op_allocate(step_index, step, &params),
            "data_profile" => self.op_data_profile(step_index, step, &params),
            "not_null_check" => self.op_not_null_check(step_index, step, &params),
            "column_compare" => self.op_column_compare(step_index, step, &params),
            "format_check" => self.op_format_check(step_index, step, &params),
            "range_check" => self.op_range_check(step_index, step, &params),
            "shuffle" => self.op_shuffle(step_index, step, &params),
            "assign_seats" => self.op_assign_seats(step_index, step, &params),
            "sum_check" => self.op_sum_check(step_index, step, &params),
            "balance_check" => self.op_balance_check(step_index, step, &params),
            "split_column" => self.op_split_column(step_index, step, &params),
            "join_dataset" => self.op_join_dataset(step_index, step, &params),
            "append_dataset" => self.op_append_dataset(step_index, step, &params),
            "value_map" => self.op_value_map(step_index, step, &params),
            "text_clean" => self.op_text_clean(step_index, step, &params),
            "sequence" => self.op_sequence(step_index, step, &params),
            "multi_column_stat" => self.op_multi_column_stat(step_index, step, &params),
            "cumulative" => self.op_cumulative(step_index, step, &params),
            "window_compare" => self.op_window_compare(step_index, step, &params),
            "aging_bucket" => self.op_aging_bucket(step_index, step, &params),
            "grade_level" => self.op_grade_level(step_index, step, &params),
            "standard_score" => self.op_standard_score(step_index, step, &params),
            "score_summary" => self.op_score_summary(step_index, step, &params),
            "score_segment" => self.op_score_segment(step_index, step, &params),
            "pivot" => self.op_pivot(step_index, step, &params),
            "output" => self.op_output(step_index, step, &params),
            other => Err(format!("不支持的操作符: {other}")),
        }
    }

    fn op_require_fields(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let fields = list_param(p, "fields");
        if fields.is_empty() {
            return Err("必需字段列表为空".to_string());
        }
        let missing: Vec<String> = fields
            .iter()
            .filter(|f| !self.has_column(f))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "缺少必需字段：{}。当前字段：{}",
                missing.join("、"),
                expr::preview_columns(
                    &self
                        .columns
                        .iter()
                        .map(|c| c.name.clone())
                        .collect::<Vec<_>>()
                )
            ));
        }
        if !bool_param(p, "allow_extra", true) {
            let extra: Vec<String> = self
                .columns
                .iter()
                .filter(|c| !fields.contains(&c.name))
                .map(|c| c.name.clone())
                .collect();
            if !extra.is_empty() {
                return Err(format!("存在未声明的字段：{}", extra.join("、")));
            }
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            None,
            Some(format!("必要字段齐全：{}", fields.join("、"))),
        ))
    }

    fn op_unique_check(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let cols = list_param(p, "columns");
        if cols.is_empty() {
            return Err("唯一性校验字段为空".to_string());
        }
        for c in &cols {
            if !self.has_column(c) {
                return Err(format!("字段 [{c}] 不存在"));
            }
        }
        let keys = cols.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ");
        let inner = format!(
            "SELECT COUNT(*) FROM (SELECT {keys} FROM {} GROUP BY {keys} HAVING COUNT(*) > 1)",
            self.cur
        );
        let sql = self.with_ctes(&inner);
        let dup: i64 = self
            .conn
            .query_row(&sql, [], |r| r.get(0))
            .map_err(|e| format!("唯一性校验失败: {e}"))?;
        let on_error = str_param(p, "on_error");
        if dup > 0 && on_error != "warn" {
            return Err(format!(
                "字段 {} 存在 {dup} 组重复数据，流程终止",
                cols.join("、")
            ));
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(inner),
            Some(if dup > 0 {
                format!("警告：字段 {} 有 {dup} 组重复", cols.join("、"))
            } else {
                format!("字段 {} 无重复", cols.join("、"))
            }),
        ))
    }

    fn op_map_columns(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let use_global = bool_param(p, "apply_global_alias", true);
        let mut plan: Vec<(String, String)> = Vec::new();
        if use_global {
            let mut stmt = self
                .conn
                .prepare("SELECT alias, standard FROM alias_mappings")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(|e| e.to_string())?;
            for r in rows.flatten() {
                plan.push(r);
            }
        }
        if let Some(arr) = p.get("mappings").and_then(|v| v.as_array()) {
            for item in arr {
                let from = item
                    .get("from")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let to = item
                    .get("to")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if from.is_empty() || to.is_empty() || from == to {
                    continue;
                }
                plan.retain(|(a, _)| a != &from);
                plan.push((from, to));
            }
        }

        let mut renames: Vec<(String, String)> = Vec::new();
        for c in &self.columns {
            if let Some((_, to)) = plan.iter().find(|(a, _)| a == &c.name) {
                if to != &c.name {
                    renames.push((c.name.clone(), to.clone()));
                }
            }
        }
        if renames.is_empty() {
            return Ok(self.log_entry(
                i,
                step,
                "ok",
                self.count_current()?,
                None,
                Some("没有需要映射的字段".to_string()),
            ));
        }
        // 校验改名后不产生重名
        let mut names: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        for (from, to) in &renames {
            if let Some(pos) = names.iter().position(|n| n == from) {
                names[pos] = to.clone();
            }
        }
        if store::unique_names(&names) != names {
            return Err(format!(
                "映射后出现重名字段：{}",
                renames
                    .iter()
                    .map(|(_, to)| to.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            ));
        }
        let items: Vec<String> = self
            .columns
            .iter()
            .map(|c| {
                let new = renames
                    .iter()
                    .find(|(f, _)| f == &c.name)
                    .map(|(_, t)| t.clone())
                    .unwrap_or_else(|| c.name.clone());
                format!("{} AS {}", q(&c.name), q(&new))
            })
            .collect();
        let body = format!("SELECT {} FROM {}", items.join(", "), self.cur);
        let cte = self.push_cte(body);
        for (from, to) in &renames {
            self.rename_column(from, to);
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!(
                "已映射 {} 个字段：{}",
                renames.len(),
                renames
                    .iter()
                    .map(|(f, t)| format!("{f}→{t}"))
                    .collect::<Vec<_>>()
                    .join("、")
            )),
        ))
    }

    fn op_rename(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let from = str_param(p, "from");
        let to = str_param(p, "to");
        if from.is_empty() || to.is_empty() {
            return Err("原字段与新字段名都不能为空".to_string());
        }
        if !self.has_column(&from) {
            return Err(format!("字段 [{from}] 不存在"));
        }
        if self.has_column(&to) {
            return Err(format!("字段 [{to}] 已存在，不能重名"));
        }
        let items: Vec<String> = self
            .columns
            .iter()
            .map(|c| {
                let new = if c.name == from {
                    to.clone()
                } else {
                    c.name.clone()
                };
                format!("{} AS {}", q(&c.name), q(&new))
            })
            .collect();
        let body = format!("SELECT {} FROM {}", items.join(", "), self.cur);
        let cte = self.push_cte(body.clone());
        self.rename_column(&from, &to);
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("{from} → {to}")),
        ))
    }

    fn op_select(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let cols = list_param(p, "columns");
        if cols.is_empty() {
            return Err("保留字段列表为空".to_string());
        }
        for c in &cols {
            if !self.has_column(c) {
                return Err(format!("字段 [{c}] 不存在"));
            }
        }
        let items: Vec<String> = cols
            .iter()
            .map(|c| format!("{} AS {}", q(c), q(c)))
            .collect();
        let body = format!("SELECT {} FROM {}", items.join(", "), self.cur);
        let cte = self.push_cte(body.clone());
        self.columns.retain(|c| cols.contains(&c.name));
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("保留 {} 个字段", cols.len())),
        ))
    }

    fn op_cast(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let to = str_param(p, "to");
        let decimals = num_param(p, "decimals").unwrap_or(2);
        let on_error = str_param(p, "on_error");
        let base = match to.as_str() {
            "text" => format!("CAST({} AS TEXT)", q(&col)),
            "integer" => format!("CAST({} AS INTEGER)", q(&col)),
            _ => {
                let c = format!("CAST({} AS REAL)", q(&col));
                if decimals >= 0 {
                    format!("ROUND({c}, {decimals})")
                } else {
                    c
                }
            }
        };
        let e = if to == "text" {
            base
        } else if on_error == "fail" {
            let guard = expr::numeric_guard_sql(&col);
            let bad_sql = self.with_ctes(&format!(
                "SELECT COUNT(*) FROM {} WHERE CAST({} AS TEXT) IS NOT NULL AND TRIM(CAST({} AS TEXT)) <> '' AND NOT ({guard})",
                self.cur,
                q(&col),
                q(&col)
            ));
            let bad: i64 = self
                .conn
                .query_row(&bad_sql, [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if bad > 0 {
                return Err(format!("字段 [{col}] 有 {bad} 行不是合法数字，流程终止"));
            }
            base
        } else {
            let guard = expr::numeric_guard_sql(&col);
            format!("CASE WHEN {guard} THEN {base} ELSE NULL END")
        };
        let body = self.project(&self.cur.clone(), &[(col.clone(), e)], &[]);
        let cte = self.push_cte(body.clone());
        if let Some(c) = self.columns.iter_mut().find(|c| c.name == col) {
            c.dtype = if to == "text" {
                "text".to_string()
            } else {
                "number".to_string()
            };
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("[{col}] → {to}")),
        ))
    }

    fn op_fill_null(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let value = str_param(p, "value");
        let e = format!(
            "CASE WHEN {} IS NULL OR TRIM(CAST({} AS TEXT)) = '' THEN '{}' ELSE {} END",
            q(&col),
            q(&col),
            value.replace('\'', "''"),
            q(&col)
        );
        let body = self.project(&self.cur.clone(), &[(col.clone(), e)], &[]);
        let cte = self.push_cte(body.clone());
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("空值填充：[{col}] → “{value}”")),
        ))
    }

    fn op_compute(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let out = str_param(p, "out");
        let raw = str_param(p, "expr");
        if out.is_empty() {
            return Err("新字段名不能为空".to_string());
        }
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let cols: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        let sql_expr = expr::compile(&raw, &cols)?;
        let decimals = num_param(p, "decimals").unwrap_or(2);
        let e = if decimals >= 0 {
            format!("ROUND(({sql_expr}), {decimals})")
        } else {
            sql_expr
        };
        let body = format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur);
        let cte = self.push_cte(body.clone());
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "number".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("新增字段 [{out}] = {raw}")),
        ))
    }

    fn op_rank(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let by = str_param(p, "by");
        if !self.has_column(&by) {
            return Err(format!("字段 [{by}] 不存在"));
        }
        let out = {
            let o = str_param(p, "out");
            if o.is_empty() {
                "排名".to_string()
            } else {
                o
            }
        };
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let method = match str_param(p, "method").as_str() {
            "dense_rank" => "DENSE_RANK()",
            "row_number" => "ROW_NUMBER()",
            _ => "RANK()",
        };
        let order = if str_param(p, "order") == "asc" {
            "ASC"
        } else {
            "DESC"
        };
        let parts = list_param(p, "partition_by");
        for c in &parts {
            if !self.has_column(c) {
                return Err(format!("分组字段 [{c}] 不存在"));
            }
        }
        let partition = if parts.is_empty() {
            String::new()
        } else {
            format!(
                "PARTITION BY {} ",
                parts.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")
            )
        };
        let e = format!("{method} OVER ({partition}ORDER BY {} {order})", q(&by));
        let body = format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur);
        let cte = self.push_cte(body.clone());
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "number".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!(
                "按 [{by}] {order} 生成 [{out}]{}",
                if parts.is_empty() {
                    String::new()
                } else {
                    format!("（分组：{}）", parts.join("、"))
                }
            )),
        ))
    }

    fn op_filter(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let raw = str_param(p, "expr");
        let cols: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        let cond = expr::compile(&raw, &cols)?;
        let body = format!("SELECT * FROM {} WHERE ({cond})", self.cur);
        let cte = self.push_cte(body.clone());
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("筛选条件：{raw}")),
        ))
    }

    fn op_dedupe(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let cols = list_param(p, "columns");
        if cols.is_empty() {
            return Err("去重依据字段为空".to_string());
        }
        for c in &cols {
            if !self.has_column(c) {
                return Err(format!("字段 [{c}] 不存在"));
            }
        }
        let order_by = str_param(p, "order_by");
        if !order_by.is_empty() && !self.has_column(&order_by) {
            return Err(format!("字段 [{order_by}] 不存在"));
        }
        let order = if str_param(p, "order") == "asc" {
            "ASC"
        } else {
            "DESC"
        };
        let window_order = if order_by.is_empty() {
            String::new()
        } else {
            format!("ORDER BY {} {order} ", q(&order_by))
        };
        let inner = self.project(
            &self.cur.clone(),
            &[],
            &[(
                "__rn".to_string(),
                format!(
                    "ROW_NUMBER() OVER (PARTITION BY {} {} )",
                    cols.iter().map(|c| q(c)).collect::<Vec<_>>().join(", "),
                    window_order.trim()
                ),
            )],
        );
        let keep = self
            .columns
            .iter()
            .map(|c| q(&c.name))
            .collect::<Vec<_>>()
            .join(", ");
        let body = format!("SELECT {keep} FROM ({inner}) WHERE \"__rn\" = 1");
        let cte = self.push_cte(body.clone());
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("按 {} 去重", cols.join("、"))),
        ))
    }

    fn op_sort(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let keys = sorts_param(p);
        if keys.is_empty() {
            return Err("请至少设置一个排序字段".to_string());
        }
        for (field, _) in &keys {
            if !self.has_column(field) {
                return Err(format!("字段 [{field}] 不存在"));
            }
        }
        self.order_by = Some(
            keys.iter()
                .map(|(f, o)| format!("{} {o}", q(f)))
                .collect::<Vec<_>>()
                .join(", "),
        );
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            None,
            Some(format!(
                "结果排序：{}",
                keys.iter()
                    .map(|(f, o)| format!("[{f}] {}", if o == "ASC" { "升序" } else { "降序" }))
                    .collect::<Vec<_>>()
                    .join(" → ")
            )),
        ))
    }

    fn op_limit(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let n = num_param(p, "n").unwrap_or(100);
        self.limit = Some(n.max(0));
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            None,
            Some(format!("只输出前 {n} 行")),
        ))
    }

    fn op_random_number(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let out = str_param(p, "out");
        if out.is_empty() {
            return Err("新字段名不能为空".to_string());
        }
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let mut lo = f64_param(p, "min").unwrap_or(0.0);
        let mut hi = f64_param(p, "max").unwrap_or(100.0);
        if lo > hi {
            std::mem::swap(&mut lo, &mut hi);
        }
        let decimals = num_param(p, "decimals").unwrap_or(2).max(0);
        let e = if str_param(p, "mode") == "decimal" {
            format!(
                "ROUND((({lo}) + (ABS(RANDOM() % 1000000) * 1.0 / 1000000.0) * (({hi}) - ({lo}))), {decimals})"
            )
        } else {
            let span = (hi.floor() - lo.ceil() + 1.0).max(1.0) as i64;
            format!("(ABS(RANDOM() % {span}) + {})", lo.ceil() as i64)
        };
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "number".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("新增随机数字段 [{out}]（{lo} ~ {hi}）")),
        ))
    }

    fn op_random_rows(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let mode = str_param(p, "mode");
        // SQLite 的 RANDOM() 在 ORDER BY 中即随机顺序；抽样再叠加 LIMIT。
        self.order_by = Some("RANDOM()".to_string());
        if mode == "sample" {
            let n = num_param(p, "n").unwrap_or(10).max(0);
            self.limit = Some(n);
            Ok(self.log_entry(
                i,
                step,
                "ok",
                self.count_current()?,
                None,
                Some(format!("随机抽取 {n} 行")),
            ))
        } else {
            Ok(self.log_entry(
                i,
                step,
                "ok",
                self.count_current()?,
                None,
                Some("已随机打乱行顺序".to_string()),
            ))
        }
    }

    fn op_group_aggregate(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let groups = list_param(p, "group_by");
        for g in &groups {
            if !self.has_column(g) {
                return Err(format!("分组字段 [{g}] 不存在"));
            }
        }
        let aggs = aggregations_param(p);
        if aggs.is_empty() {
            return Err("请至少添加一个汇总项".to_string());
        }
        const FUNCS: &[&str] = &[
            "count",
            "count_distinct",
            "sum",
            "avg",
            "min",
            "max",
            "group_concat",
            "stddev",
            "variance",
        ];
        let mut items: Vec<String> = groups
            .iter()
            .map(|g| format!("{} AS {}", q(g), q(g)))
            .collect();
        let mut names: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        let mut new_cols: Vec<DatasetColumn> = Vec::new();
        let mut summaries: Vec<String> = Vec::new();
        for agg in &aggs {
            if !FUNCS.contains(&agg.func.as_str()) {
                return Err(format!("不支持的汇总方式：{}", agg.func));
            }
            let needs_field = agg.func != "count";
            if needs_field && agg.field.is_empty() {
                return Err(format!("{} 汇总必须选择字段", agg.func));
            }
            if needs_field && !self.has_column(&agg.field) {
                return Err(format!("汇总字段 [{}] 不存在", agg.field));
            }
            let mut out = if agg.out.is_empty() {
                agg_label(&agg.func, &agg.field)
            } else {
                agg.out.clone()
            };
            if names.contains(&out) {
                let mut k = 2;
                while names.contains(&format!("{out}{k}")) {
                    k += 1;
                }
                out = format!("{out}{k}");
            }
            let f = q(&agg.field);
            let expr = match agg.func.as_str() {
                "count" => "COUNT(*)".to_string(),
                "count_distinct" => format!("COUNT(DISTINCT {f})"),
                "sum" => {
                    let e = format!("SUM({f})");
                    if agg.decimals >= 0 {
                        format!("ROUND({e}, {})", agg.decimals)
                    } else {
                        e
                    }
                }
                "avg" => format!("ROUND(AVG({f}), {})", agg.decimals.max(0)),
                "min" => format!("MIN({f})"),
                "max" => format!("MAX({f})"),
                "group_concat" => format!("GROUP_CONCAT({f})"),
                "stddev" => format!(
                    "ROUND(SQRT(MAX(0, AVG({f} * {f}) - AVG({f}) * AVG({f}))), {})",
                    agg.decimals.max(0)
                ),
                _ => format!(
                    "ROUND(MAX(0, AVG({f} * {f}) - AVG({f}) * AVG({f})), {})",
                    agg.decimals.max(0)
                ),
            };
            items.push(format!("{expr} AS {}", q(&out)));
            names.push(out.clone());
            let dtype = match agg.func.as_str() {
                "group_concat" => "text",
                _ => "number",
            };
            new_cols.push(DatasetColumn {
                name: out.clone(),
                dtype: dtype.to_string(),
            });
            summaries.push(format!(
                "{}({}) → [{out}]",
                agg.func,
                if agg.field.is_empty() {
                    "*"
                } else {
                    &agg.field
                }
            ));
        }
        let body = if groups.is_empty() {
            format!("SELECT {} FROM {}", items.join(", "), self.cur)
        } else {
            format!(
                "SELECT {} FROM {} GROUP BY {}",
                items.join(", "),
                self.cur,
                groups.iter().map(|g| q(g)).collect::<Vec<_>>().join(", ")
            )
        };
        let cte = self.push_cte(body);
        let mut cols: Vec<DatasetColumn> = groups
            .iter()
            .map(|g| DatasetColumn {
                name: g.clone(),
                dtype: self
                    .columns
                    .iter()
                    .find(|c| &c.name == g)
                    .map(|c| c.dtype.clone())
                    .unwrap_or_else(|| "text".to_string()),
            })
            .collect();
        cols.extend(new_cols);
        self.columns = cols;
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!(
                "分组汇总{}：{}",
                if groups.is_empty() {
                    String::new()
                } else {
                    format!("（按 {}）", groups.join("、"))
                },
                summaries.join("，")
            )),
        ))
    }

    fn op_round_number(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let out = {
            let o = str_param(p, "out");
            if o.is_empty() {
                col.clone()
            } else {
                o
            }
        };
        if out != col && self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let decimals = num_param(p, "decimals").unwrap_or(2).max(0);
        let f = q(&col);
        let e = match str_param(p, "mode").as_str() {
            "floor" => format!(
                "(CAST({f} AS INTEGER) - CASE WHEN {f} < 0 AND {f} <> CAST({f} AS INTEGER) THEN 1 ELSE 0 END)"
            ),
            "ceil" => format!(
                "(CAST({f} AS INTEGER) + CASE WHEN {f} > 0 AND {f} <> CAST({f} AS INTEGER) THEN 1 ELSE 0 END)"
            ),
            _ => format!("ROUND({f}, {decimals})"),
        };
        let cte = if out == col {
            let body = self.project(&self.cur.clone(), &[(col.clone(), e)], &[]);
            self.push_cte(body)
        } else {
            let body = self.project(&self.cur.clone(), &[], &[(out.clone(), e)]);
            self.add_column(DatasetColumn {
                name: out.clone(),
                dtype: "number".to_string(),
            });
            self.push_cte(body)
        };
        if let Some(c) = self.columns.iter_mut().find(|c| c.name == col) {
            c.dtype = "number".to_string();
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("数值取整：[{col}] → [{out}]")),
        ))
    }

    fn op_text_concat(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let cols = list_param(p, "columns");
        if cols.is_empty() {
            return Err("请至少选择一个参与拼接的字段".to_string());
        }
        for c in &cols {
            if !self.has_column(c) {
                return Err(format!("字段 [{c}] 不存在"));
            }
        }
        let out = str_param(p, "out");
        if out.is_empty() {
            return Err("新字段名不能为空".to_string());
        }
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let sep = str_param(p, "separator").replace('\'', "''");
        let parts: Vec<String> = cols
            .iter()
            .map(|c| format!("COALESCE({}, '')", text_sql(&q(c))))
            .collect();
        let e = parts.join(&format!(" || '{sep}' || "));
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "text".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("拼接 {} → [{out}]", cols.join(" + "))),
        ))
    }

    fn op_text_substr(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let out = str_param(p, "out");
        if out.is_empty() {
            return Err("新字段名不能为空".to_string());
        }
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let start = num_param(p, "start").unwrap_or(1).max(1);
        let length = num_param(p, "length").unwrap_or(0);
        let e = if length > 0 {
            format!("SUBSTR({}, {start}, {length})", text_sql(&q(&col)))
        } else {
            format!("SUBSTR({}, {start})", text_sql(&q(&col)))
        };
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "text".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("截取 [{col}] → [{out}]")),
        ))
    }

    fn op_text_pad(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let out = {
            let o = str_param(p, "out");
            if o.is_empty() {
                col.clone()
            } else {
                o
            }
        };
        if out != col && self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let width = num_param(p, "width").unwrap_or(0).clamp(0, 128);
        let mut pad = str_param(p, "pad");
        if pad.is_empty() {
            pad = "0".to_string();
        }
        pad = pad.replace('\'', "''");
        let filler = pad.repeat(width.max(1) as usize);
        let f = q(&col);
        let cast = text_sql(&f);
        let len = format!("MAX(0, {width} - LENGTH(COALESCE({cast}, '')))");
        let value = format!("COALESCE({cast}, '')");
        let e = if str_param(p, "side") == "right" {
            format!("({value} || SUBSTR('{filler}', 1, {len}))")
        } else {
            format!("(SUBSTR('{filler}', 1, {len}) || {value})")
        };
        let cte = if out == col {
            let body = self.project(&self.cur.clone(), &[(col.clone(), e)], &[]);
            self.push_cte(body)
        } else {
            let body = self.project(&self.cur.clone(), &[], &[(out.clone(), e)]);
            self.push_cte(body)
        };
        if out != col {
            self.add_column(DatasetColumn {
                name: out.clone(),
                dtype: "text".to_string(),
            });
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("补位到 {width} 位：[{col}] → [{out}]")),
        ))
    }

    fn op_text_replace(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let find = str_param(p, "find");
        if find.is_empty() {
            return Err("查找内容不能为空".to_string());
        }
        let out = {
            let o = str_param(p, "out");
            if o.is_empty() {
                col.clone()
            } else {
                o
            }
        };
        if out != col && self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let replace = str_param(p, "replace");
        let e = format!(
            "REPLACE(COALESCE({}, ''), '{}', '{}')",
            text_sql(&q(&col)),
            find.replace('\'', "''"),
            replace.replace('\'', "''")
        );
        let cte = if out == col {
            let body = self.project(&self.cur.clone(), &[(col.clone(), e)], &[]);
            self.push_cte(body)
        } else {
            let body = self.project(&self.cur.clone(), &[], &[(out.clone(), e)]);
            self.push_cte(body)
        };
        if out != col {
            self.add_column(DatasetColumn {
                name: out.clone(),
                dtype: "text".to_string(),
            });
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("文本替换：[{col}] 中的「{find}」→「{replace}」")),
        ))
    }

    fn op_conditional(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let out = str_param(p, "out");
        if out.is_empty() {
            return Err("新字段名不能为空".to_string());
        }
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let cols: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        let cond = expr::compile(&str_param(p, "condition"), &cols)?;
        let then = value_or_expr(&str_param(p, "then"), &cols)?;
        let otherwise = value_or_expr(&str_param(p, "else"), &cols)?;
        let e = format!("CASE WHEN ({cond}) THEN ({then}) ELSE ({otherwise}) END");
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "text".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("按条件生成 [{out}]")),
        ))
    }

    fn op_date_calc(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let out = str_param(p, "out");
        if out.is_empty() {
            return Err("新字段名不能为空".to_string());
        }
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let mode = str_param(p, "mode");
        let norm = format!("norm_date({})", q(&col));
        let (e, dtype, desc) = match mode.as_str() {
            "add_days" => {
                let days = num_param(p, "days").unwrap_or(0);
                (
                    format!("DATE({norm}, '{days} days')"),
                    "text",
                    format!("日期加减 {days} 天"),
                )
            }
            "diff_days" => {
                let other = str_param(p, "other");
                if other.is_empty() || !self.has_column(&other) {
                    return Err(format!("对比日期字段 [{other}] 不存在"));
                }
                (
                    format!(
                        "CAST(julianday({norm}) - julianday(norm_date({})) AS INTEGER)",
                        q(&other)
                    ),
                    "number",
                    format!("与 [{other}] 相差天数"),
                )
            }
            "year" => (
                format!("CAST(strftime('%Y', {norm}) AS INTEGER)"),
                "number",
                "提取年份".to_string(),
            ),
            "month" => (
                format!("CAST(strftime('%m', {norm}) AS INTEGER)"),
                "number",
                "提取月份".to_string(),
            ),
            "day" => (
                format!("CAST(strftime('%d', {norm}) AS INTEGER)"),
                "number",
                "提取日".to_string(),
            ),
            "weekday" => (
                format!(
                    "CASE CAST(strftime('%w', {norm}) AS INTEGER) \
                     WHEN 0 THEN '星期日' WHEN 1 THEN '星期一' WHEN 2 THEN '星期二' \
                     WHEN 3 THEN '星期三' WHEN 4 THEN '星期四' WHEN 5 THEN '星期五' \
                     WHEN 6 THEN '星期六' END"
                ),
                "text",
                "提取星期".to_string(),
            ),
            _ => (
                format!(
                    "CAST((julianday('now', 'localtime') - julianday({norm})) / 365.2425 AS INTEGER)"
                ),
                "number",
                "计算年龄".to_string(),
            ),
        };
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: dtype.to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("{desc} → [{out}]")),
        ))
    }

    fn op_parse_id_card(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let birth = str_param(p, "birth_out");
        let gender = str_param(p, "gender_out");
        let age = str_param(p, "age_out");
        let valid = str_param(p, "valid_out");
        if birth.is_empty() && gender.is_empty() && age.is_empty() && valid.is_empty() {
            return Err("至少需要输出一个解析结果字段".to_string());
        }
        let f = q(&col);
        let candidates: Vec<(&str, String, &str)> = vec![
            (&birth, format!("id_card_birth({f})"), "text"),
            (&gender, format!("id_card_gender({f})"), "text"),
            (
                &age,
                format!(
                    "CAST((julianday('now', 'localtime') - julianday(id_card_birth({f}))) / 365.2425 AS INTEGER)"
                ),
                "number",
            ),
            (
                &valid,
                format!(
                    "CASE WHEN TRIM(CAST({f} AS TEXT)) = '' THEN NULL \
                     WHEN id_card_valid({f}) = 1 THEN '合法' ELSE '非法' END"
                ),
                "text",
            ),
        ];
        let mut planned = self.columns.clone();
        let mut names: Vec<String> = planned.iter().map(|c| c.name.clone()).collect();
        let mut appends: Vec<(String, String)> = Vec::new();
        let mut added: Vec<String> = Vec::new();
        for (name, expr, dtype) in candidates {
            let name = name.trim();
            if name.is_empty() || names.contains(&name.to_string()) {
                continue;
            }
            appends.push((name.to_string(), expr));
            planned.push(DatasetColumn {
                name: name.to_string(),
                dtype: dtype.to_string(),
            });
            names.push(name.to_string());
            added.push(name.to_string());
        }
        if appends.is_empty() {
            return Err("解析结果字段与现有字段重名，请换一个名字".to_string());
        }
        if str_param(p, "on_error") == "fail" {
            let sql = self.with_ctes(&format!(
                "SELECT COUNT(*) FROM {} WHERE TRIM(COALESCE(CAST({f} AS TEXT), '')) <> '' AND id_card_valid({f}) = 0",
                self.cur
            ));
            let bad: i64 = self
                .conn
                .query_row(&sql, [], |r| r.get(0))
                .map_err(|e| format!("身份证校验失败: {e}"))?;
            if bad > 0 {
                return Err(format!("有 {bad} 行身份证号校验位不合法，流程终止"));
            }
        }
        let mut items: Vec<String> = planned
            .iter()
            .filter(|c| self.has_column(&c.name))
            .map(|c| format!("{} AS {}", q(&c.name), q(&c.name)))
            .collect();
        for (name, expr) in &appends {
            items.push(format!("{expr} AS {}", q(name)));
        }
        let cte = self.push_cte(format!("SELECT {} FROM {}", items.join(", "), self.cur));
        self.columns = planned;
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("身份证解析：新增 {} 个字段", added.len())),
        ))
    }

    fn op_amount_upper(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let out = {
            let o = str_param(p, "out");
            if o.is_empty() {
                "金额大写".to_string()
            } else {
                o
            }
        };
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let e = format!("rmb_upper(CAST({} AS REAL))", q(&col));
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "text".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("金额大写：[{col}] → [{out}]")),
        ))
    }

    fn op_percentage(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let out = {
            let o = str_param(p, "out");
            if o.is_empty() {
                "占比".to_string()
            } else {
                o
            }
        };
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let parts = list_param(p, "partition_by");
        for c in &parts {
            if !self.has_column(c) {
                return Err(format!("分组字段 [{c}] 不存在"));
            }
        }
        let decimals = num_param(p, "decimals").unwrap_or(2).max(0);
        let partition = if parts.is_empty() {
            String::new()
        } else {
            format!(
                "PARTITION BY {} ",
                parts.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")
            )
        };
        let f = q(&col);
        let e = format!("ROUND({f} * 100.0 / NULLIF(SUM({f}) OVER ({partition}), 0), {decimals})");
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "number".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("计算 [{col}] 占比 → [{out}]")),
        ))
    }

    fn op_tax_calc(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let rate = f64_param(p, "rate").ok_or_else(|| "税率不能为空".to_string())?;
        if !(0.0..=1.0).contains(&rate) {
            return Err("税率应在 0~1 之间（如 13% 填 0.13）".to_string());
        }
        let mode = {
            let m = str_param(p, "mode");
            if m.is_empty() {
                "exclusive".to_string()
            } else {
                m
            }
        };
        if mode != "exclusive" && mode != "inclusive" {
            return Err(format!("未知的金额口径: {mode}"));
        }
        let decimals = num_param(p, "decimals").unwrap_or(2).max(0);
        let net_out = defaulted(str_param(p, "net_out"), "不含税金额");
        let tax_out = defaulted(str_param(p, "tax_out"), "税额");
        let gross_out = defaulted(str_param(p, "gross_out"), "含税金额");
        for out in [&net_out, &tax_out, &gross_out] {
            if self.has_column(out) {
                return Err(format!("字段 [{out}] 已存在"));
            }
        }
        if net_out == tax_out || net_out == gross_out || tax_out == gross_out {
            return Err("三个输出字段名不能相同".to_string());
        }
        let f = format!("CAST({} AS REAL)", q(&col));
        let net_expr = if mode == "inclusive" {
            format!("ROUND({f} / (1.0 + {rate}), {decimals})")
        } else {
            format!("ROUND({f}, {decimals})")
        };
        let tax_expr = if mode == "inclusive" {
            format!("ROUND({f}, {decimals}) - {net_expr}")
        } else {
            format!("ROUND({f} * {rate}, {decimals})")
        };
        let gross_expr = if mode == "inclusive" {
            format!("ROUND({f}, {decimals})")
        } else {
            format!("{net_expr} + {tax_expr}")
        };
        let cte = self.push_cte(format!(
            "SELECT *, {net_expr} AS {}, {tax_expr} AS {}, {gross_expr} AS {} FROM {}",
            q(&net_out),
            q(&tax_out),
            q(&gross_out),
            self.cur
        ));
        for out in [&net_out, &tax_out, &gross_out] {
            self.add_column(DatasetColumn {
                name: out.clone(),
                dtype: "number".to_string(),
            });
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!(
                "税额计算：[{col}] 按 {:.2}% {}",
                rate * 100.0,
                if mode == "inclusive" {
                    "价内税拆分"
                } else {
                    "价外税拆分"
                }
            )),
        ))
    }

    fn op_budget_variance(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let budget = str_param(p, "budget_column");
        let actual = str_param(p, "actual_column");
        for col in [&budget, &actual] {
            if !self.has_column(col) {
                return Err(format!("字段 [{col}] 不存在"));
            }
        }
        if budget == actual {
            return Err("预算金额与实际金额不能是同一字段".to_string());
        }
        let groups = list_param(p, "group_by");
        for g in &groups {
            if !self.has_column(g) {
                return Err(format!("分组字段 [{g}] 不存在"));
            }
        }
        let decimals = num_param(p, "decimals").unwrap_or(2).max(0);
        let warning = f64_param(p, "warning_rate").unwrap_or(100.0);
        let status_out = defaulted(str_param(p, "status_out"), "预算状态");
        if groups.iter().any(|g| g == &status_out) {
            return Err(format!("状态字段 [{status_out}] 与分组字段重名"));
        }
        let b = format!("COALESCE(SUM(CAST({} AS REAL)), 0)", q(&budget));
        let a = format!("COALESCE(SUM(CAST({} AS REAL)), 0)", q(&actual));
        let rate_expr = format!("ROUND({a} * 100.0 / NULLIF({b}, 0), {decimals})");
        let mut items: Vec<String> = groups
            .iter()
            .map(|g| format!("{} AS {}", q(g), q(g)))
            .collect();
        items.push(format!("{b} AS {}", q("预算数")));
        items.push(format!("{a} AS {}", q("实际数")));
        items.push(format!("{rate_expr} AS {}", q("执行率")));
        items.push(format!("ROUND({a} - {b}, {decimals}) AS {}", q("差额")));
        items.push(format!(
            "CASE WHEN {b} = 0 THEN '无预算' WHEN {rate_expr} > {warning} THEN '超支' ELSE '正常' END AS {}",
            q(&status_out)
        ));
        let group_by = if groups.is_empty() {
            String::new()
        } else {
            format!(
                " GROUP BY {}",
                groups.iter().map(|g| q(g)).collect::<Vec<_>>().join(", ")
            )
        };
        let cte = self.push_cte(format!(
            "SELECT {} FROM {} WHERE {} IS NOT NULL OR {} IS NOT NULL{group_by}",
            items.join(", "),
            self.cur,
            q(&budget),
            q(&actual)
        ));
        let mut cols: Vec<DatasetColumn> = groups
            .iter()
            .map(|g| DatasetColumn {
                name: g.clone(),
                dtype: self
                    .columns
                    .iter()
                    .find(|c| &c.name == g)
                    .map(|c| c.dtype.clone())
                    .unwrap_or_else(|| "text".to_string()),
            })
            .collect();
        for name in ["预算数", "实际数", "执行率", "差额"] {
            cols.push(DatasetColumn {
                name: name.to_string(),
                dtype: "number".to_string(),
            });
        }
        cols.push(DatasetColumn {
            name: status_out.clone(),
            dtype: "text".to_string(),
        });
        self.columns = cols;
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("预算执行分析：[{budget}] / [{actual}]")),
        ))
    }

    fn op_loan_payment(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let principal = str_param(p, "principal");
        if !self.has_column(&principal) {
            return Err(format!("字段 [{principal}] 不存在"));
        }
        let annual_rate =
            f64_param(p, "annual_rate").ok_or_else(|| "年利率不能为空".to_string())?;
        if !(0.0..=1.0).contains(&annual_rate) {
            return Err("年利率应在 0~1 之间（如 4.9% 填 0.049）".to_string());
        }
        let years = num_param(p, "years").unwrap_or(10);
        if years <= 0 {
            return Err("期限必须大于 0 年".to_string());
        }
        let months = years * 12;
        let decimals = num_param(p, "decimals").unwrap_or(2).max(0);
        let payment_out = defaulted(str_param(p, "payment_out"), "月供");
        let interest_out = defaulted(str_param(p, "interest_out"), "总利息");
        let total_out = defaulted(str_param(p, "total_out"), "还款总额");
        for out in [&payment_out, &interest_out, &total_out] {
            if self.has_column(out) {
                return Err(format!("字段 [{out}] 已存在"));
            }
        }
        if payment_out == interest_out || payment_out == total_out || interest_out == total_out {
            return Err("三个输出字段名不能相同".to_string());
        }
        let principal_expr = format!("CAST({} AS REAL)", q(&principal));
        let raw_payment = if annual_rate == 0.0 {
            format!("({principal_expr}) / {months}")
        } else {
            let rate = annual_rate / 12.0;
            let growth = format!("power(1.0 + ({rate}), {months})");
            format!("({principal_expr}) * ({rate}) * {growth} / ({growth} - 1.0)")
        };
        let payment = format!("ROUND({raw_payment}, {decimals})");
        let total = format!("ROUND(({payment}) * {months}, {decimals})");
        let interest = format!("ROUND(({total}) - ({principal_expr}), {decimals})");
        let cte = self.push_cte(format!(
            "SELECT *, {payment} AS {}, {interest} AS {}, {total} AS {} FROM {}",
            q(&payment_out),
            q(&interest_out),
            q(&total_out),
            self.cur
        ));
        for out in [&payment_out, &interest_out, &total_out] {
            self.add_column(DatasetColumn {
                name: out.clone(),
                dtype: "number".to_string(),
            });
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!(
                "等额本息：{years} 年 · 年利率 {:.2}%",
                annual_rate * 100.0
            )),
        ))
    }

    fn op_reconcile(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let right_id = str_param(p, "dataset_id");
        if right_id.is_empty() {
            return Err("请选择对方数据集".to_string());
        }
        let left_key = str_param(p, "left_key");
        let left_amount = str_param(p, "left_amount");
        for col in [&left_key, &left_amount] {
            if !self.has_column(col) {
                return Err(format!("字段 [{col}] 不存在"));
            }
        }
        let right_cols = store::get_columns(self.conn, &right_id)
            .map_err(|e| format!("读取对方数据集字段失败: {e}"))?;
        if right_cols.is_empty() {
            return Err("对方数据集没有字段".to_string());
        }
        let right_key = str_param(p, "right_key");
        let right_amount = str_param(p, "right_amount");
        let right_key_idx = right_cols
            .iter()
            .position(|c| c.name == right_key)
            .ok_or_else(|| format!("对方数据集没有字段 [{right_key}]"))?;
        let right_amount_idx = right_cols
            .iter()
            .position(|c| c.name == right_amount)
            .ok_or_else(|| format!("对方数据集没有字段 [{right_amount}]"))?;
        let table =
            crate::db::table_name(&right_id).map_err(|e| format!("对方数据集不可用: {e}"))?;
        let tolerance = f64_param(p, "tolerance").unwrap_or(0.01).max(0.0);
        let mode = {
            let m = str_param(p, "mode");
            if m.is_empty() {
                "diff".to_string()
            } else {
                m
            }
        };
        if mode != "diff" && mode != "all" {
            return Err(format!("未知的输出范围: {mode}"));
        }
        let l_key = format!("CAST({} AS TEXT)", q(&left_key));
        let l_amount = format!("CAST({} AS REAL)", q(&left_amount));
        let r_key = format!("CAST(\"c{right_key_idx}\" AS TEXT)");
        let r_amount = format!("CAST(\"c{right_amount_idx}\" AS REAL)");
        let left_set = format!(
            "(SELECT {l_key} AS \"__k__\", COALESCE(SUM({l_amount}), 0) AS \"__amt__\" FROM {} WHERE {l_key} IS NOT NULL AND {l_key} <> '' GROUP BY {l_key})",
            self.cur
        );
        let right_set = format!(
            "(SELECT {r_key} AS \"__k__\", COALESCE(SUM({r_amount}), 0) AS \"__amt__\" FROM \"{table}\" WHERE {r_key} IS NOT NULL AND {r_key} <> '' GROUP BY {r_key})"
        );
        let diff = "ROUND(l.\"__amt__\" - r.\"__amt__\", 2)";
        let body = format!(
            "SELECT l.\"__k__\" AS {}, l.\"__amt__\" AS {}, r.\"__amt__\" AS {}, {diff} AS {}, \
             CASE WHEN r.\"__k__\" IS NULL THEN '本方独有' \
                  WHEN ABS(l.\"__amt__\" - r.\"__amt__\") <= {tolerance} THEN '一致' \
                  ELSE '金额不符' END AS {} \
             FROM {left_set} l LEFT JOIN {right_set} r ON l.\"__k__\" = r.\"__k__\" \
             UNION ALL \
             SELECT r.\"__k__\" AS {}, 0 AS {}, r.\"__amt__\" AS {}, ROUND(0 - r.\"__amt__\", 2) AS {}, '对方独有' AS {} \
             FROM {right_set} r LEFT JOIN {left_set} l ON l.\"__k__\" = r.\"__k__\" \
             WHERE l.\"__k__\" IS NULL",
            q("对账键"),
            q("本方金额"),
            q("对方金额"),
            q("差异"),
            q("对账结果"),
            q("对账键"),
            q("本方金额"),
            q("对方金额"),
            q("差异"),
            q("对账结果"),
        );
        let body = if mode == "diff" {
            format!("SELECT * FROM ({body}) WHERE {} <> '一致'", q("对账结果"))
        } else {
            body
        };
        let cte = self.push_cte(body);
        self.columns = vec![
            DatasetColumn {
                name: "对账键".to_string(),
                dtype: "text".to_string(),
            },
            DatasetColumn {
                name: "本方金额".to_string(),
                dtype: "number".to_string(),
            },
            DatasetColumn {
                name: "对方金额".to_string(),
                dtype: "number".to_string(),
            },
            DatasetColumn {
                name: "差异".to_string(),
                dtype: "number".to_string(),
            },
            DatasetColumn {
                name: "对账结果".to_string(),
                dtype: "text".to_string(),
            },
        ];
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!(
                "对账核对：[{left_key}] / [{right_key}]{}",
                if mode == "diff" {
                    "（仅输出差异）"
                } else {
                    ""
                }
            )),
        ))
    }

    fn op_depreciation(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let original = str_param(p, "original");
        if !self.has_column(&original) {
            return Err(format!("字段 [{original}] 不存在"));
        }
        let months_field = str_param(p, "months");
        if !months_field.is_empty() && !self.has_column(&months_field) {
            return Err(format!("已计提月数字段 [{months_field}] 不存在"));
        }
        let rate = f64_param(p, "salvage_rate")
            .unwrap_or(5.0)
            .clamp(0.0, 100.0);
        let years = f64_param(p, "years").unwrap_or(5.0);
        if years <= 0.0 {
            return Err("使用年限必须大于 0".to_string());
        }
        let mut outs: Vec<String> = Vec::new();
        for (key, default) in [
            ("monthly_out", "月折旧额"),
            ("accumulated_out", "累计折旧"),
            ("net_out", "固定资产净值"),
        ] {
            let name = {
                let v = str_param(p, key);
                if v.is_empty() {
                    default.to_string()
                } else {
                    v
                }
            };
            if self.has_column(&name) || outs.contains(&name) {
                return Err(format!("字段 [{name}] 已存在"));
            }
            outs.push(name);
        }
        let orig = format!("CAST({} AS REAL)", q(&original));
        let depreciable = format!("({orig} * (1.0 - {rate} / 100.0))");
        let monthly = format!("ROUND({depreciable} / {years} / 12.0, 2)");
        let months = if months_field.is_empty() {
            "0".to_string()
        } else {
            format!(
                "CAST(COALESCE(CAST({} AS REAL), 0) AS INTEGER)",
                q(&months_field)
            )
        };
        let accumulated = format!("ROUND(MIN({depreciable}, ({monthly}) * ({months})), 2)");
        let net = format!("ROUND({orig} - ({accumulated}), 2)");
        let cte = self.push_cte(format!(
            "SELECT *, {monthly} AS {}, {accumulated} AS {}, {net} AS {} FROM {}",
            q(&outs[0]),
            q(&outs[1]),
            q(&outs[2]),
            self.cur
        ));
        for name in &outs {
            self.add_column(DatasetColumn {
                name: name.clone(),
                dtype: "number".to_string(),
            });
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("直线法折旧：残值率 {rate}% / 年限 {years} 年")),
        ))
    }

    fn op_allocate(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let amount = str_param(p, "amount");
        let weight = str_param(p, "weight");
        for c in [&amount, &weight] {
            if !self.has_column(c) {
                return Err(format!("字段 [{c}] 不存在"));
            }
        }
        let out = str_param(p, "out");
        if out.is_empty() {
            return Err("分摊结果字段名不能为空".to_string());
        }
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let parts = list_param(p, "partition_by");
        for c in &parts {
            if !self.has_column(c) {
                return Err(format!("分组字段 [{c}] 不存在"));
            }
        }
        let decimals = num_param(p, "decimals").unwrap_or(2).max(0);
        let partition = if parts.is_empty() {
            String::new()
        } else {
            format!(
                "PARTITION BY {}",
                parts.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")
            )
        };
        let e = format!(
            "ROUND(CAST({amount} AS REAL) * CAST({weight} AS REAL) / NULLIF(SUM(CAST({weight} AS REAL)) OVER ({partition}), 0), {decimals})",
            amount = q(&amount),
            weight = q(&weight)
        );
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "number".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("按 [{weight}] 分摊 [{amount}] → [{out}]")),
        ))
    }

    fn op_data_profile(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let requested = list_param(p, "columns");
        let cols: Vec<DatasetColumn> = if requested.is_empty() {
            self.columns.clone()
        } else {
            let mut out: Vec<DatasetColumn> = Vec::new();
            for name in &requested {
                let Some(col) = self.columns.iter().find(|c| &c.name == name) else {
                    return Err(format!("字段 [{name}] 不存在"));
                };
                out.push(col.clone());
            }
            out
        };
        if cols.is_empty() {
            return Err("没有可统计的字段".to_string());
        }
        let mut selects: Vec<String> = Vec::new();
        for col in &cols {
            let f = q(&col.name);
            let avg = if col.dtype == "number" {
                format!("ROUND(AVG(CAST({f} AS REAL)), 2)")
            } else {
                "NULL".to_string()
            };
            selects.push(format!(
                "SELECT '{}' AS \"字段名\",
                        SUM(CASE WHEN {f} IS NOT NULL AND TRIM(CAST({f} AS TEXT)) <> '' THEN 1 ELSE 0 END) AS \"非空数\",
                        SUM(CASE WHEN {f} IS NULL OR TRIM(CAST({f} AS TEXT)) = '' THEN 1 ELSE 0 END) AS \"空值数\",
                        COUNT(DISTINCT {f}) AS \"去重数\",
                        MIN({f}) AS \"最小值\",
                        MAX({f}) AS \"最大值\",
                        {avg} AS \"平均值\"
                 FROM {}",
                col.name.replace('\'', "''"),
                self.cur
            ));
        }
        let cte = self.push_cte(selects.join(" UNION ALL "));
        self.columns = vec![
            DatasetColumn {
                name: "字段名".into(),
                dtype: "text".into(),
            },
            DatasetColumn {
                name: "非空数".into(),
                dtype: "number".into(),
            },
            DatasetColumn {
                name: "空值数".into(),
                dtype: "number".into(),
            },
            DatasetColumn {
                name: "去重数".into(),
                dtype: "number".into(),
            },
            DatasetColumn {
                name: "最小值".into(),
                dtype: "text".into(),
            },
            DatasetColumn {
                name: "最大值".into(),
                dtype: "text".into(),
            },
            DatasetColumn {
                name: "平均值".into(),
                dtype: "number".into(),
            },
        ];
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("已统计 {} 个字段", cols.len())),
        ))
    }

    fn op_not_null_check(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let cols = list_param(p, "columns");
        if cols.is_empty() {
            return Err("请至少选择一个必须非空的字段".to_string());
        }
        let mut conds: Vec<String> = Vec::new();
        for c in &cols {
            if !self.has_column(c) {
                return Err(format!("字段 [{c}] 不存在"));
            }
            let f = q(c);
            conds.push(format!("({f} IS NULL OR TRIM(CAST({f} AS TEXT)) = '')"));
        }
        let sql = self.with_ctes(&format!(
            "SELECT COUNT(*) FROM {} WHERE {}",
            self.cur,
            conds.join(" OR ")
        ));
        let bad: i64 = self
            .conn
            .query_row(&sql, [], |r| r.get(0))
            .map_err(|e| format!("空值校验失败: {e}"))?;
        if bad > 0 && str_param(p, "on_error") == "fail" {
            return Err(format!("有 {bad} 行存在空值（{}）", cols.join("、")));
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            None,
            Some(if bad == 0 {
                format!("字段 {} 均无空值", cols.join("、"))
            } else {
                format!("有 {bad} 行存在空值，已按警告继续")
            }),
        ))
    }

    fn op_sum_check(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let total = str_param(p, "total");
        let parts = list_param(p, "parts");
        if total.is_empty() {
            return Err("合计字段不能为空".to_string());
        }
        if parts.is_empty() {
            return Err("明细字段不能为空".to_string());
        }
        for col in std::iter::once(&total).chain(parts.iter()) {
            if !self.has_column(col) {
                return Err(format!("字段 [{col}] 不存在"));
            }
        }
        let tolerance = f64_param(p, "tolerance").unwrap_or(0.01).max(0.0);
        let sum = parts
            .iter()
            .map(|c| format!("COALESCE(CAST({} AS REAL), 0)", q(c)))
            .collect::<Vec<_>>()
            .join(" + ");
        let cond = format!(
            "ABS(COALESCE(CAST({} AS REAL), 0) - ({sum})) > {tolerance}",
            q(&total)
        );
        let sql = self.with_ctes(&format!("SELECT COUNT(*) FROM {} WHERE {cond}", self.cur));
        let bad: i64 = self
            .conn
            .query_row(&sql, [], |r| r.get(0))
            .map_err(|e| format!("合计一致性校验失败: {e}"))?;
        if bad > 0 && str_param(p, "on_error") != "warn" {
            return Err(format!(
                "有 {bad} 行合计 [{total}] 与明细之和（{}）不一致",
                parts.join(" + ")
            ));
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            None,
            Some(if bad == 0 {
                format!("合计 [{total}] 与明细之和一致")
            } else {
                format!("有 {bad} 行合计不一致，已按警告继续")
            }),
        ))
    }

    fn op_column_compare(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let left = str_param(p, "left");
        let right = str_param(p, "right");
        for c in [&left, &right] {
            if !self.has_column(c) {
                return Err(format!("字段 [{c}] 不存在"));
            }
        }
        let mut cond = format!(
            "TRIM(CAST({} AS TEXT)) <> TRIM(CAST({} AS TEXT))",
            q(&left),
            q(&right)
        );
        if bool_param(p, "ignore_blank", true) {
            cond = format!(
                "TRIM(COALESCE(CAST({} AS TEXT), '')) <> '' AND TRIM(COALESCE(CAST({} AS TEXT), '')) <> '' AND {cond}",
                q(&left),
                q(&right)
            );
        }
        let sql = self.with_ctes(&format!("SELECT COUNT(*) FROM {} WHERE {cond}", self.cur));
        let bad: i64 = self
            .conn
            .query_row(&sql, [], |r| r.get(0))
            .map_err(|e| format!("一致性校验失败: {e}"))?;
        if bad > 0 && str_param(p, "on_error") == "fail" {
            return Err(format!("[{left}] 与 [{right}] 有 {bad} 行内容不一致"));
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            None,
            Some(if bad == 0 {
                format!("[{left}] 与 [{right}] 完全一致")
            } else {
                format!("有 {bad} 行不一致，已按警告继续")
            }),
        ))
    }

    fn op_format_check(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let (func, label) = match str_param(p, "mode").as_str() {
            "id_card" => ("id_card_valid", "身份证号"),
            "email" => ("is_email", "邮箱"),
            "bank_card" => ("is_bank_card", "银行卡号"),
            _ => ("is_mobile", "手机号"),
        };
        let f = q(&col);
        let sql = self.with_ctes(&format!(
            "SELECT COUNT(*) FROM {} WHERE TRIM(COALESCE(CAST({f} AS TEXT), '')) <> '' AND {func}({f}) = 0",
            self.cur
        ));
        let bad: i64 = self
            .conn
            .query_row(&sql, [], |r| r.get(0))
            .map_err(|e| format!("格式校验失败: {e}"))?;
        if bad > 0 && str_param(p, "on_error") == "fail" {
            return Err(format!("字段 [{col}] 有 {bad} 行不是合法的{label}"));
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            None,
            Some(if bad == 0 {
                format!("字段 [{col}] 格式全部合法（{label}）")
            } else {
                format!("字段 [{col}] 有 {bad} 行{label}格式不合法，已按警告继续")
            }),
        ))
    }

    fn op_assign_seats(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let room_out = {
            let v = str_param(p, "room_out");
            if v.is_empty() {
                "考场号".to_string()
            } else {
                v
            }
        };
        let seat_out = {
            let v = str_param(p, "seat_out");
            if v.is_empty() {
                "座位号".to_string()
            } else {
                v
            }
        };
        if room_out == seat_out {
            return Err("考场号与座位号字段名不能相同".to_string());
        }
        for out in [&room_out, &seat_out] {
            if self.has_column(out) {
                return Err(format!("字段 [{out}] 已存在"));
            }
        }
        let per_room = num_param(p, "per_room").unwrap_or(30);
        if per_room <= 0 {
            return Err("每考场人数必须大于 0".to_string());
        }
        let start_room = num_param(p, "start_room").unwrap_or(1);
        let order_by = str_param(p, "order_by");
        if !order_by.is_empty() && !self.has_column(&order_by) {
            return Err(format!("排序字段 [{order_by}] 不存在"));
        }
        let order = if order_by.is_empty() {
            "ORDER BY (SELECT NULL)".to_string()
        } else {
            format!("ORDER BY {}", q(&order_by))
        };
        let rn = format!("ROW_NUMBER() OVER ({order})");
        let room_expr = format!("(CAST(({rn} - 1) / {per_room} AS INTEGER) + {start_room})");
        let seat_expr = format!("((({rn} - 1) % {per_room}) + 1)");
        let cte = self.push_cte(format!(
            "SELECT *, {room_expr} AS {}, {seat_expr} AS {} FROM {}",
            q(&room_out),
            q(&seat_out),
            self.cur
        ));
        self.add_column(DatasetColumn {
            name: room_out.clone(),
            dtype: "number".to_string(),
        });
        self.add_column(DatasetColumn {
            name: seat_out.clone(),
            dtype: "number".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!(
                "已编排：每考场 {per_room} 人，起始考场 {start_room}"
            )),
        ))
    }

    fn op_shuffle(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let groups = list_param(p, "group_by");
        for g in &groups {
            if !self.has_column(g) {
                return Err(format!("分组字段 [{g}] 不存在"));
            }
        }
        let out = defaulted(str_param(p, "out"), "随机序号");
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let (expr, order) = if groups.is_empty() {
            (
                format!("ROW_NUMBER() OVER (ORDER BY RANDOM()) AS {}", q(&out)),
                format!("{} ASC", q(&out)),
            )
        } else {
            let partition = groups.iter().map(|g| q(g)).collect::<Vec<_>>().join(", ");
            (
                format!(
                    "ROW_NUMBER() OVER (PARTITION BY {partition} ORDER BY RANDOM()) AS {}",
                    q(&out)
                ),
                format!(
                    "{} ASC, {} ASC",
                    q(&out),
                    groups.iter().map(|g| q(g)).collect::<Vec<_>>().join(", ")
                ),
            )
        };
        let cte = self.push_cte(format!("SELECT *, {expr} FROM {}", self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "number".to_string(),
        });
        self.order_by = Some(order);
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(if groups.is_empty() {
                format!("全表随机打散 → [{out}]")
            } else {
                format!(
                    "按 {} 分组随机穿插 → [{out}]",
                    groups
                        .iter()
                        .map(|g| format!("[{g}]"))
                        .collect::<Vec<_>>()
                        .join("、")
                )
            }),
        ))
    }

    fn op_range_check(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let min = f64_param(p, "min");
        let max = f64_param(p, "max");
        if min.is_none() && max.is_none() {
            return Err("请至少设置最小值或最大值".to_string());
        }
        let f = format!("CAST({} AS REAL)", q(&col));
        let mut conds: Vec<String> = Vec::new();
        if let Some(v) = min {
            conds.push(format!("{f} < {v}"));
        }
        if let Some(v) = max {
            conds.push(format!("{f} > {v}"));
        }
        let sql = self.with_ctes(&format!(
            "SELECT COUNT(*) FROM {} WHERE {} IS NOT NULL AND ({})",
            self.cur,
            q(&col),
            conds.join(" OR ")
        ));
        let bad: i64 = self
            .conn
            .query_row(&sql, [], |r| r.get(0))
            .map_err(|e| format!("范围校验失败: {e}"))?;
        let range = format!(
            "{} ~ {}",
            min.map(|v| v.to_string()).unwrap_or_else(|| "不限".into()),
            max.map(|v| v.to_string()).unwrap_or_else(|| "不限".into())
        );
        if bad > 0 && str_param(p, "on_error") == "fail" {
            return Err(format!("字段 [{col}] 有 {bad} 行超出允许范围（{range}）"));
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            None,
            Some(if bad == 0 {
                format!("字段 [{col}] 全部在范围 {range} 内")
            } else {
                format!("字段 [{col}] 有 {bad} 行超出范围 {range}，已按警告继续")
            }),
        ))
    }

    fn op_balance_check(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let voucher = str_param(p, "voucher");
        let debit = str_param(p, "debit");
        let credit = str_param(p, "credit");
        for c in [&voucher, &debit, &credit] {
            if !self.has_column(c) {
                return Err(format!("字段 [{c}] 不存在"));
            }
        }
        let tolerance = f64_param(p, "tolerance").unwrap_or(0.01).abs();
        let sql = self.with_ctes(&format!(
            "SELECT COUNT(*) FROM (
                SELECT {} FROM {} GROUP BY {}
                HAVING ABS(SUM(COALESCE(CAST({} AS REAL), 0)) - SUM(COALESCE(CAST({} AS REAL), 0))) > {tolerance}
             )",
            q(&voucher),
            self.cur,
            q(&voucher),
            q(&debit),
            q(&credit)
        ));
        let bad: i64 = self
            .conn
            .query_row(&sql, [], |r| r.get(0))
            .map_err(|e| format!("借贷平衡校验失败: {e}"))?;
        if bad > 0 && str_param(p, "on_error") == "fail" {
            return Err(format!("有 {bad} 张凭证借贷不平衡（容差 {tolerance}）"));
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            None,
            Some(if bad == 0 {
                "所有凭证借贷平衡".to_string()
            } else {
                format!("有 {bad} 张凭证借贷不平衡，已按警告继续")
            }),
        ))
    }

    fn op_split_column(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let delimiter = {
            let d = str_param(p, "delimiter");
            if d.is_empty() {
                ",".to_string()
            } else {
                d
            }
        };
        let parts = num_param(p, "max_parts").unwrap_or(2).clamp(1, 20);
        let given: Vec<String> = {
            let raw = str_param(p, "names");
            if raw.is_empty() {
                Vec::new()
            } else {
                raw.split([',', '，'])
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
        };
        let mut names: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        let mut exprs: Vec<String> = Vec::new();
        let mut new_cols: Vec<DatasetColumn> = Vec::new();
        for k in 1..=parts {
            let mut out = given
                .get((k - 1) as usize)
                .cloned()
                .unwrap_or_else(|| format!("{col}_{k}"));
            if names.contains(&out) {
                let mut n = 2;
                while names.contains(&format!("{out}_{n}")) {
                    n += 1;
                }
                out = format!("{out}_{n}");
            }
            let expr = format!(
                "split_part(CAST({} AS TEXT), '{}', {k})",
                q(&col),
                delimiter.replace('\'', "''")
            );
            exprs.push(format!("{expr} AS {}", q(&out)));
            new_cols.push(DatasetColumn {
                name: out.clone(),
                dtype: "text".to_string(),
            });
            names.push(out);
        }
        let cte = self.push_cte(format!("SELECT *, {} FROM {}", exprs.join(", "), self.cur));
        self.columns.extend(new_cols);
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("按「{delimiter}」拆分 [{col}]，新增 {parts} 列")),
        ))
    }

    fn op_append_dataset(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let right_id = str_param(p, "dataset_id");
        if right_id.is_empty() {
            return Err("请选择要追加的数据集".to_string());
        }
        let right_cols = store::get_columns(self.conn, &right_id)
            .map_err(|e| format!("读取追加数据集字段失败: {e}"))?;
        if right_cols.is_empty() {
            return Err("追加数据集没有字段".to_string());
        }
        let table =
            crate::db::table_name(&right_id).map_err(|e| format!("追加数据集不可用: {e}"))?;
        let items: Vec<String> = self
            .columns
            .iter()
            .map(
                |col| match right_cols.iter().position(|c| c.name == col.name) {
                    Some(idx) => format!("\"c{idx}\""),
                    None => "NULL".to_string(),
                },
            )
            .collect();
        let union = if bool_param(p, "dedupe", false) {
            "UNION"
        } else {
            "UNION ALL"
        };
        let body = format!(
            "SELECT * FROM {} {union} SELECT {} FROM \"{}\"",
            self.cur,
            items.join(", "),
            table
        );
        let before = self.count_current()?;
        let cte = self.push_cte(body);
        let total = self.count_current()?;
        Ok(self.log_entry(
            i,
            step,
            "ok",
            total,
            Some(cte),
            Some(format!("追加 {} 行（合并后 {total} 行）", total - before)),
        ))
    }

    fn op_join_dataset(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let right_id = str_param(p, "dataset_id");
        if right_id.is_empty() {
            return Err("请选择关联数据集".to_string());
        }
        let left_field = str_param(p, "left_field");
        if !self.has_column(&left_field) {
            return Err(format!("字段 [{left_field}] 不存在"));
        }
        let right_cols = store::get_columns(self.conn, &right_id)
            .map_err(|e| format!("读取关联数据集字段失败: {e}"))?;
        if right_cols.is_empty() {
            return Err("关联数据集没有字段".to_string());
        }
        let right_field = str_param(p, "right_field");
        if !right_cols.iter().any(|c| c.name == right_field) {
            return Err(format!("关联表中没有字段 [{right_field}]"));
        }
        let requested = list_param(p, "fields");
        let prefix = str_param(p, "prefix");
        let plan = join_output_plan(
            &self
                .columns
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>(),
            &right_cols,
            &requested,
            &prefix,
        )?;
        let table =
            crate::db::table_name(&right_id).map_err(|e| format!("关联数据集不可用: {e}"))?;
        let mut right_items: Vec<String> = Vec::new();
        for (idx, col) in right_cols.iter().enumerate() {
            if col.name == right_field {
                right_items.push(format!("\"c{idx}\" AS \"__join_key__\""));
            }
        }
        for (name, _, _) in &plan {
            if let Some(idx) = right_cols.iter().position(|c| &c.name == name) {
                right_items.push(format!("\"c{idx}\" AS {}", q(name)));
            }
        }
        let mut items: Vec<String> = vec!["l.*".to_string()];
        for (name, out, _) in &plan {
            items.push(format!("r.{} AS {}", q(name), q(out)));
        }
        let join = if str_param(p, "join_type") == "inner" {
            "INNER JOIN"
        } else {
            "LEFT JOIN"
        };
        let body = format!(
            "SELECT {} FROM {} l {} (SELECT {} FROM \"{}\") r ON CAST(l.{} AS TEXT) = CAST(r.\"__join_key__\" AS TEXT)",
            items.join(", "),
            self.cur,
            join,
            right_items.join(", "),
            table,
            q(&left_field)
        );
        let cte = self.push_cte(body);
        for (_, out, dtype) in &plan {
            self.add_column(DatasetColumn {
                name: out.clone(),
                dtype: dtype.clone(),
            });
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("关联数据集，新增 {} 个字段", plan.len())),
        ))
    }

    fn op_value_map(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let mut cases: Vec<String> = Vec::new();
        if let Some(arr) = p.get("mappings").and_then(|v| v.as_array()) {
            for item in arr {
                let from = item
                    .get("from")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let to = item.get("to").and_then(|v| v.as_str()).unwrap_or("").trim();
                if from.is_empty() {
                    continue;
                }
                cases.push(format!(
                    "WHEN CAST({} AS TEXT) = '{}' THEN '{}'",
                    q(&col),
                    from.replace('\'', "''"),
                    to.replace('\'', "''")
                ));
            }
        }
        if cases.is_empty() {
            return Err("请至少添加一条值映射".to_string());
        }
        let out = {
            let o = str_param(p, "out");
            if o.is_empty() {
                col.clone()
            } else {
                o
            }
        };
        if out != col && self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let fallback = if bool_param(p, "keep_unmatched", true) {
            text_sql(&q(&col))
        } else {
            "NULL".to_string()
        };
        let expr = format!("CASE {} ELSE {fallback} END", cases.join(" "));
        let cte = if out == col {
            let body = self.project(&self.cur.clone(), &[(col.clone(), expr)], &[]);
            self.push_cte(body)
        } else {
            let body = self.project(&self.cur.clone(), &[], &[(out.clone(), expr)]);
            self.push_cte(body)
        };
        if out != col {
            self.add_column(DatasetColumn {
                name: out.clone(),
                dtype: "text".to_string(),
            });
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("值映射：[{col}] → [{out}]")),
        ))
    }

    fn op_text_clean(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let out = {
            let o = str_param(p, "out");
            if o.is_empty() {
                col.clone()
            } else {
                o
            }
        };
        if out != col && self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let f = q(&col);
        let raw = format!("CAST({f} AS TEXT)");
        let (expr, desc) = match str_param(p, "mode").as_str() {
            "remove_spaces" => (
                format!(
                    "REPLACE(REPLACE(REPLACE(TRIM({raw}), ' ', ''), CHAR(9), ''), CHAR(10), '')"
                ),
                "去除空格与换行",
            ),
            "upper" => (format!("UPPER({raw})"), "转为大写"),
            "lower" => (format!("LOWER({raw})"), "转为小写"),
            "full_to_half" => (format!("full_to_half({raw})"), "全角转半角"),
            _ => (format!("TRIM({raw})"), "去除首尾空白"),
        };
        let cte = if out == col {
            let body = self.project(&self.cur.clone(), &[(col.clone(), expr)], &[]);
            self.push_cte(body)
        } else {
            let body = self.project(&self.cur.clone(), &[], &[(out.clone(), expr)]);
            self.push_cte(body)
        };
        if out != col {
            self.add_column(DatasetColumn {
                name: out.clone(),
                dtype: "text".to_string(),
            });
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("{desc}：[{col}] → [{out}]")),
        ))
    }

    fn op_sequence(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let out = {
            let o = str_param(p, "out");
            if o.is_empty() {
                "序号".to_string()
            } else {
                o
            }
        };
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let start = num_param(p, "start").unwrap_or(1);
        let parts = list_param(p, "partition_by");
        for c in &parts {
            if !self.has_column(c) {
                return Err(format!("分组字段 [{c}] 不存在"));
            }
        }
        let order_by = str_param(p, "order_by");
        if !order_by.is_empty() && !self.has_column(&order_by) {
            return Err(format!("排序字段 [{order_by}] 不存在"));
        }
        let partition = if parts.is_empty() {
            String::new()
        } else {
            format!(
                "PARTITION BY {} ",
                parts.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")
            )
        };
        let order = if order_by.is_empty() {
            "ORDER BY (SELECT NULL)".to_string()
        } else {
            format!("ORDER BY {}", q(&order_by))
        };
        let e = format!("ROW_NUMBER() OVER ({partition}{order}) + ({start} - 1)");
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "number".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("生成序号字段 [{out}]")),
        ))
    }

    fn op_multi_column_stat(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let cols = list_param(p, "columns");
        if cols.is_empty() {
            return Err("请至少选择一个参与计算的字段".to_string());
        }
        for c in &cols {
            if !self.has_column(c) {
                return Err(format!("字段 [{c}] 不存在"));
            }
        }
        let out = str_param(p, "out");
        if out.is_empty() {
            return Err("输出字段名不能为空".to_string());
        }
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let decimals = num_param(p, "decimals").unwrap_or(2).max(0);
        let vals: Vec<String> = cols
            .iter()
            .map(|c| format!("CAST({} AS REAL)", q(c)))
            .collect();
        let e = match str_param(p, "mode").as_str() {
            "avg" => format!(
                "ROUND(({}) / NULLIF(({}), 0), {decimals})",
                vals.iter()
                    .map(|v| format!("COALESCE({v}, 0)"))
                    .collect::<Vec<_>>()
                    .join(" + "),
                cols.iter()
                    .map(|c| format!("(CASE WHEN {} IS NULL THEN 0 ELSE 1 END)", q(c)))
                    .collect::<Vec<_>>()
                    .join(" + ")
            ),
            "count" => format!(
                "({})",
                cols.iter()
                    .map(|c| format!("(CASE WHEN {} IS NULL THEN 0 ELSE 1 END)", q(c)))
                    .collect::<Vec<_>>()
                    .join(" + ")
            ),
            "max" | "min" => {
                let func = if str_param(p, "mode") == "min" {
                    "MIN"
                } else {
                    "MAX"
                };
                let parts: Vec<String> = cols
                    .iter()
                    .map(|c| format!("COALESCE(CAST({} AS REAL), 0)", q(c)))
                    .collect();
                if parts.len() == 1 {
                    parts[0].clone()
                } else {
                    format!("{func}({})", parts.join(", "))
                }
            }
            _ => format!(
                "ROUND({}, {decimals})",
                vals.iter()
                    .map(|v| format!("COALESCE({v}, 0)"))
                    .collect::<Vec<_>>()
                    .join(" + ")
            ),
        };
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "number".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("多列统计 {} → [{out}]", cols.join("、"))),
        ))
    }

    fn op_cumulative(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let out = str_param(p, "out");
        if out.is_empty() {
            return Err("输出字段名不能为空".to_string());
        }
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let order_by = str_param(p, "order_by");
        if !order_by.is_empty() && !self.has_column(&order_by) {
            return Err(format!("排序字段 [{order_by}] 不存在"));
        }
        let parts = list_param(p, "partition_by");
        for c in &parts {
            if !self.has_column(c) {
                return Err(format!("分组字段 [{c}] 不存在"));
            }
        }
        let decimals = num_param(p, "decimals").unwrap_or(2).max(0);
        let partition = if parts.is_empty() {
            String::new()
        } else {
            format!(
                "PARTITION BY {} ",
                parts.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")
            )
        };
        let order = if order_by.is_empty() {
            "ORDER BY (SELECT NULL)".to_string()
        } else {
            format!("ORDER BY {}", q(&order_by))
        };
        let e = format!(
            "ROUND(SUM(CAST({} AS REAL)) OVER ({partition}{order} ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW), {decimals})",
            q(&col)
        );
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "number".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("累计求和：[{col}] → [{out}]")),
        ))
    }

    fn op_window_compare(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        let order_by = str_param(p, "order_by");
        for c in [&col, &order_by] {
            if !self.has_column(c) {
                return Err(format!("字段 [{c}] 不存在"));
            }
        }
        let out = str_param(p, "out");
        if out.is_empty() {
            return Err("输出字段名不能为空".to_string());
        }
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let parts = list_param(p, "partition_by");
        for c in &parts {
            if !self.has_column(c) {
                return Err(format!("分组字段 [{c}] 不存在"));
            }
        }
        let decimals = num_param(p, "decimals").unwrap_or(2).max(0);
        let periods = num_param(p, "periods").unwrap_or(1).max(1);
        let partition = if parts.is_empty() {
            String::new()
        } else {
            format!(
                "PARTITION BY {} ",
                parts.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")
            )
        };
        let f = format!("CAST({} AS REAL)", q(&col));
        let prev = format!(
            "LAG({f}, {periods}) OVER ({partition}ORDER BY {})",
            q(&order_by)
        );
        let e = if str_param(p, "mode") == "diff" {
            format!("ROUND({f} - ({prev}), {decimals})")
        } else {
            format!("ROUND(({f} - ({prev})) * 100.0 / NULLIF(ABS({prev}), 0), {decimals})")
        };
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "number".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("同期比较：[{col}] → [{out}]")),
        ))
    }

    fn op_aging_bucket(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let out = {
            let o = str_param(p, "out");
            if o.is_empty() {
                "账龄".to_string()
            } else {
                o
            }
        };
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let mode = if str_param(p, "mode") == "days" {
            "days"
        } else {
            "date"
        };
        let buckets = {
            let b = str_param(p, "buckets");
            if b.is_empty() {
                "30,60,90,180".to_string()
            } else {
                b
            }
        };
        let e = format!(
            "aging_bucket(CAST({} AS TEXT), '{mode}', '{}')",
            q(&col),
            buckets.replace('\'', "''")
        );
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "text".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("账龄分段 → [{out}]")),
        ))
    }

    fn op_grade_level(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let mut cuts: Vec<f64> = str_param(p, "cutoffs")
            .split([',', '，'])
            .filter_map(|s| s.trim().parse::<f64>().ok())
            .collect();
        if cuts.is_empty() {
            return Err("请至少设置一个分数阈值".to_string());
        }
        cuts.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let mut labels: Vec<String> = str_param(p, "labels")
            .split([',', '，'])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        while labels.len() < cuts.len() + 1 {
            labels.push(format!("等级{}", labels.len() + 1));
        }
        let out = {
            let o = str_param(p, "out");
            if o.is_empty() {
                "等级".to_string()
            } else {
                o
            }
        };
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let mut arms: Vec<String> = Vec::new();
        for (idx, cut) in cuts.iter().enumerate() {
            arms.push(format!(
                "WHEN CAST({} AS REAL) >= {cut} THEN '{}'",
                q(&col),
                labels[idx].replace('\'', "''")
            ));
        }
        let last = labels[cuts.len()].replace('\'', "''");
        let e = format!(
            "CASE WHEN {} IS NULL THEN NULL {} ELSE '{last}' END",
            q(&col),
            arms.join(" ")
        );
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "text".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("等级划分：[{col}] → [{out}]")),
        ))
    }

    fn op_standard_score(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let out = {
            let o = str_param(p, "out");
            if o.is_empty() {
                "标准分".to_string()
            } else {
                o
            }
        };
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let parts = list_param(p, "partition_by");
        for c in &parts {
            if !self.has_column(c) {
                return Err(format!("分组字段 [{c}] 不存在"));
            }
        }
        let decimals = num_param(p, "decimals").unwrap_or(2).max(0);
        let f = format!("CAST({} AS REAL)", q(&col));
        let partition = if parts.is_empty() {
            String::new()
        } else {
            format!(
                "PARTITION BY {} ",
                parts.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")
            )
        };
        let avg = format!("AVG({f}) OVER ({partition})");
        let std = format!("SQRT(MAX(0, AVG({f} * {f}) OVER ({partition}) - ({avg}) * ({avg})))");
        let z = format!("(({f}) - ({avg})) / NULLIF(({std}), 0)");
        let e = match str_param(p, "mode").as_str() {
            "t" => format!("ROUND(50 + 10 * ({z}), {decimals})"),
            "percentile" => {
                format!("ROUND(PERCENT_RANK() OVER ({partition}ORDER BY {f}) * 100, {decimals})")
            }
            _ => format!("ROUND({z}, {decimals})"),
        };
        let cte = self.push_cte(format!("SELECT *, {e} AS {} FROM {}", q(&out), self.cur));
        self.add_column(DatasetColumn {
            name: out.clone(),
            dtype: "number".to_string(),
        });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("标准分转换：[{col}] → [{out}]")),
        ))
    }

    fn op_score_summary(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let groups = list_param(p, "group_by");
        for g in &groups {
            if !self.has_column(g) {
                return Err(format!("分组字段 [{g}] 不存在"));
            }
        }
        let decimals = num_param(p, "decimals").unwrap_or(2).max(0);
        let pass = f64_param(p, "pass_score").unwrap_or(60.0);
        let excellent = f64_param(p, "excellent_score").unwrap_or(85.0);
        let f = format!("CAST({} AS REAL)", q(&col));
        let mut items: Vec<String> = groups
            .iter()
            .map(|g| format!("{} AS {}", q(g), q(g)))
            .collect();
        let summary: Vec<(&str, String)> = vec![
            ("人数", format!("COUNT({f})")),
            ("平均分", format!("ROUND(AVG({f}), {decimals})")),
            ("最高分", format!("MAX({f})")),
            ("最低分", format!("MIN({f})")),
            (
                "及格率",
                format!(
                    "ROUND(SUM(CASE WHEN {f} >= {pass} THEN 1 ELSE 0 END) * 100.0 / NULLIF(COUNT({f}), 0), {decimals})"
                ),
            ),
            (
                "优秀率",
                format!(
                    "ROUND(SUM(CASE WHEN {f} >= {excellent} THEN 1 ELSE 0 END) * 100.0 / NULLIF(COUNT({f}), 0), {decimals})"
                ),
            ),
            (
                "标准差",
                format!(
                    "ROUND(SQRT(MAX(0, AVG({f} * {f}) - AVG({f}) * AVG({f}))), {decimals})"
                ),
            ),
        ];
        for (name, expr) in &summary {
            items.push(format!("{expr} AS {}", q(name)));
        }
        let group_by = if groups.is_empty() {
            String::new()
        } else {
            format!(
                " GROUP BY {}",
                groups.iter().map(|g| q(g)).collect::<Vec<_>>().join(", ")
            )
        };
        let cte = self.push_cte(format!(
            "SELECT {} FROM {} WHERE {} IS NOT NULL{group_by}",
            items.join(", "),
            self.cur,
            q(&col)
        ));
        let mut cols: Vec<DatasetColumn> = groups
            .iter()
            .map(|g| DatasetColumn {
                name: g.clone(),
                dtype: self
                    .columns
                    .iter()
                    .find(|c| &c.name == g)
                    .map(|c| c.dtype.clone())
                    .unwrap_or_else(|| "text".to_string()),
            })
            .collect();
        cols.extend(summary.iter().map(|(name, _)| DatasetColumn {
            name: name.to_string(),
            dtype: "number".to_string(),
        }));
        self.columns = cols;
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("成绩统计汇总：[{col}]")),
        ))
    }

    fn op_score_segment(
        &mut self,
        i: i64,
        step: &FlowStep,
        p: &Value,
    ) -> Result<RunLogEntry, String> {
        let col = str_param(p, "column");
        if !self.has_column(&col) {
            return Err(format!("字段 [{col}] 不存在"));
        }
        let mut step_width = num_param(p, "step").unwrap_or(10);
        if step_width <= 0 {
            step_width = 10;
        }
        let parts = list_param(p, "partition_by");
        for c in &parts {
            if !self.has_column(c) {
                return Err(format!("分组字段 [{c}] 不存在"));
            }
        }
        let mut seg_out = str_param(p, "segment_out");
        if seg_out.is_empty() {
            seg_out = "分数段".to_string();
        }
        let mut count_out = str_param(p, "count_out");
        if count_out.is_empty() {
            count_out = "人数".to_string();
        }
        if parts.contains(&seg_out) || parts.contains(&count_out) {
            return Err("分数段/人数字段名不能与分组字段重名".to_string());
        }
        let f = format!("CAST({} AS REAL)", q(&col));
        let seg = format!("CAST(CAST(({f}) / {step_width} AS INTEGER) * {step_width} AS INTEGER)");
        let label = if step_width == 1 {
            format!("CAST(({seg}) AS TEXT) || '分'")
        } else {
            format!(
                "CAST(({seg}) AS TEXT) || '-' || CAST(({seg}) + {step_width} - 1 AS TEXT) || '分'"
            )
        };
        let group_items = if parts.is_empty() {
            "1".to_string()
        } else {
            parts
                .iter()
                .map(|c| format!("{} AS {}", q(c), q(c)))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let group_by = if parts.is_empty() {
            String::new()
        } else {
            format!(
                " GROUP BY {}, __seg",
                parts.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")
            )
        };
        let inner = format!(
            "SELECT {group_items}, {seg} AS __seg, {label} AS {} , COUNT(*) AS {} FROM {} WHERE {} IS NOT NULL{group_by}",
            q(&seg_out),
            q(&count_out),
            self.cur,
            q(&col)
        );
        let cumulative = bool_param(p, "cumulative", true);
        let mut cols: Vec<DatasetColumn> = parts
            .iter()
            .map(|c| DatasetColumn {
                name: c.clone(),
                dtype: self
                    .columns
                    .iter()
                    .find(|x| &x.name == c)
                    .map(|x| x.dtype.clone())
                    .unwrap_or_else(|| "text".to_string()),
            })
            .collect();
        cols.push(DatasetColumn {
            name: seg_out.clone(),
            dtype: "text".to_string(),
        });
        cols.push(DatasetColumn {
            name: count_out.clone(),
            dtype: "number".to_string(),
        });
        let body = if cumulative {
            let partition = if parts.is_empty() {
                String::new()
            } else {
                format!(
                    "PARTITION BY {} ",
                    parts.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")
                )
            };
            let keep = if parts.is_empty() {
                String::new()
            } else {
                format!(
                    "{}, ",
                    parts.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")
                )
            };
            cols.push(DatasetColumn {
                name: "累计人数".to_string(),
                dtype: "number".to_string(),
            });
            format!(
                "SELECT {keep}{}, {}, SUM({}) OVER ({partition}ORDER BY __seg) AS \"累计人数\" FROM ({inner})",
                q(&seg_out),
                q(&count_out),
                q(&count_out)
            )
        } else {
            format!(
                "SELECT {}{}, {} FROM ({inner})",
                if parts.is_empty() {
                    String::new()
                } else {
                    format!(
                        "{}, ",
                        parts.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")
                    )
                },
                q(&seg_out),
                q(&count_out)
            )
        };
        let cte = self.push_cte(body);
        self.columns = cols;
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("分数段统计（每 {step_width} 分一段）")),
        ))
    }

    fn op_pivot(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let groups = list_param(p, "group_by");
        let pivot = str_param(p, "pivot_column");
        let value = str_param(p, "value_column");
        for c in groups.iter().chain([&pivot, &value]) {
            if !self.has_column(c) {
                return Err(format!("字段 [{c}] 不存在"));
            }
        }
        let max_cols = num_param(p, "max_columns").unwrap_or(50).clamp(1, 200) as usize;
        let sql = self.with_ctes(&format!(
            "SELECT DISTINCT {} FROM {} WHERE {} IS NOT NULL ORDER BY {}",
            q(&pivot),
            self.cur,
            q(&pivot),
            q(&pivot)
        ));
        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("读取透视列取值失败: {e}"))?;
        let values: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();
        if values.is_empty() {
            return Err("展开字段没有可用取值".to_string());
        }
        if values.len() > max_cols {
            return Err(format!(
                "展开字段有 {} 个不同取值，超过上限 {max_cols}，请先筛选数据或调大上限",
                values.len()
            ));
        }
        let decimals = num_param(p, "decimals").unwrap_or(2).max(0);
        let agg = str_param(p, "agg");
        let mut items: Vec<String> = groups
            .iter()
            .map(|g| format!("{} AS {}", q(g), q(g)))
            .collect();
        let mut names: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        let mut new_cols: Vec<DatasetColumn> = Vec::new();
        for v in &values {
            let key = v.trim();
            let base = if key.is_empty() { "（空）" } else { key };
            let mut out = base.to_string();
            if names.contains(&out) {
                out = format!("透视_{out}");
                let mut n = 2;
                while names.contains(&format!("{out}_{n}")) {
                    n += 1;
                }
                if names.contains(&out) {
                    out = format!("{out}_{n}");
                }
            }
            let cond = format!(
                "CAST({} AS TEXT) = '{}'",
                q(&pivot),
                key.replace('\'', "''")
            );
            let val = format!("CAST({} AS REAL)", q(&value));
            let e = match agg.as_str() {
                "count" => format!("SUM(CASE WHEN {cond} THEN 1 ELSE 0 END)"),
                "avg" => format!("ROUND(AVG(CASE WHEN {cond} THEN {val} END), {decimals})"),
                "max" => format!("MAX(CASE WHEN {cond} THEN {val} END)"),
                "min" => format!("MIN(CASE WHEN {cond} THEN {val} END)"),
                _ => format!("ROUND(SUM(CASE WHEN {cond} THEN {val} ELSE 0 END), {decimals})"),
            };
            items.push(format!("{e} AS {}", q(&out)));
            names.push(out.clone());
            new_cols.push(DatasetColumn {
                name: out,
                dtype: "number".to_string(),
            });
        }
        let group_by = if groups.is_empty() {
            String::new()
        } else {
            format!(
                " GROUP BY {}",
                groups.iter().map(|g| q(g)).collect::<Vec<_>>().join(", ")
            )
        };
        let cte = self.push_cte(format!(
            "SELECT {} FROM {}{group_by}",
            items.join(", "),
            self.cur
        ));
        let mut cols: Vec<DatasetColumn> = groups
            .iter()
            .map(|g| DatasetColumn {
                name: g.clone(),
                dtype: self
                    .columns
                    .iter()
                    .find(|c| &c.name == g)
                    .map(|c| c.dtype.clone())
                    .unwrap_or_else(|| "text".to_string()),
            })
            .collect();
        cols.extend(new_cols);
        self.columns = cols;
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            Some(cte),
            Some(format!("按 [{}] 展开 {} 列", pivot, values.len())),
        ))
    }

    fn op_output(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let name = str_param(p, "name");
        let out_name = if name.is_empty() {
            "流程输出".to_string()
        } else {
            name
        };
        self.output_name = Some(out_name.clone());
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current()?,
            None,
            Some(format!("输出结果表：{out_name}")),
        ))
    }
}

/// 运行流程：编译 → 执行 → 落地为新的数据集。
///
/// 成功路径中「建结果表 + 写数据集元数据 + 写运行记录」放在同一事务内；
/// 任一步失败都会整体回滚，不会留下半成品数据集。
pub fn run_flow(conn: &mut Connection, flow_id: &str, dataset_id: &str) -> AppResult<RunResult> {
    let (flow_name, project_id, steps) = load_flow(conn, flow_id)?;
    if steps.is_empty() {
        return Err(AppError::bad_request("流程没有任何步骤"));
    }
    // 数据集必须存在
    let src_ds = store::get_dataset(conn, dataset_id)?;

    // 项目流程：必需字段必须存在于项目模板中；缺一个就拒绝运行并记录失败。
    // 全局流程沿用原有行为（不强制绑定全局字段字典）。
    let missing = if project_id.trim().is_empty() {
        Vec::new()
    } else {
        missing_required_fields(conn, &project_id, &steps)?
    };
    if !missing.is_empty() {
        let run_id = new_id();
        let message = format!("项目模板中缺少必需字段：{}", missing.join("、"));
        conn.execute(
            "INSERT INTO flow_runs (id, flow_id, dataset_id, output_dataset_id, status, message, sql_text, created_at)
             VALUES (?1, ?2, ?3, '', 'failed', ?4, '', ?5)",
            rusqlite::params![run_id, flow_id, dataset_id, message, now()],
        )?;
        return Ok(RunResult {
            run_id,
            status: "failed".to_string(),
            message: Some(message),
            log: Vec::new(),
            output_dataset_id: None,
            output: None,
        });
    }

    let mut c = Compiler::new(conn, dataset_id)?;
    let mut failed: Option<String> = None;
    for (idx, step) in steps.iter().enumerate() {
        let i = idx as i64;
        if !step.enabled {
            c.log.push(RunLogEntry {
                step_index: i,
                operator: step.operator.clone(),
                label: if step.label.is_empty() {
                    operators::label_of(&step.operator)
                } else {
                    step.label.clone()
                },
                status: "skipped".to_string(),
                rows: 0,
                sql: None,
                message: Some("已禁用，跳过".to_string()),
            });
            continue;
        }
        match c.step(i, step) {
            Ok(entry) => c.log.push(entry),
            Err(msg) => {
                c.log.push(RunLogEntry {
                    step_index: i,
                    operator: step.operator.clone(),
                    label: if step.label.is_empty() {
                        operators::label_of(&step.operator)
                    } else {
                        step.label.clone()
                    },
                    status: "failed".to_string(),
                    rows: 0,
                    sql: None,
                    message: Some(msg.clone()),
                });
                failed = Some(format!(
                    "第 {} 步「{}」失败：{msg}",
                    i + 1,
                    steps[idx].label
                ));
                break;
            }
        }
    }

    let run_id = new_id();
    let full_sql = c.final_sql();
    // 先把编译结果取出并结束对 conn 的共享借用，下面才能开启事务。
    let columns = c.columns.clone();
    let log = std::mem::take(&mut c.log);
    let out_name = c
        .output_name
        .clone()
        .unwrap_or_else(|| format!("{flow_name} 结果"));
    drop(c);

    if let Some(msg) = failed {
        conn.execute(
            "INSERT INTO flow_runs (id, flow_id, dataset_id, output_dataset_id, status, message, sql_text, created_at)
             VALUES (?1, ?2, ?3, '', 'failed', ?4, ?5, ?6)",
            rusqlite::params![run_id, flow_id, dataset_id, msg, full_sql, now()],
        )?;
        return Ok(RunResult {
            run_id,
            status: "failed".to_string(),
            message: Some(msg),
            log,
            output_dataset_id: None,
            output: None,
        });
    }

    let origin = format!("流程「{flow_name}」");
    let (out_ds, output) = db::with_tx(conn, |tx| {
        let out_ds = store::create_dataset_from_select_tx(
            tx,
            &out_name,
            "flow_output",
            &origin,
            &src_ds.project_id,
            &columns,
            &full_sql,
        )?;
        let output = store::read_rows(tx, &out_ds.id, 0, PREVIEW_ROWS)?;
        tx.execute(
            "INSERT INTO flow_runs (id, flow_id, dataset_id, output_dataset_id, status, message, sql_text, created_at)
             VALUES (?1, ?2, ?3, ?4, 'success', ?5, ?6, ?7)",
            rusqlite::params![
                run_id,
                flow_id,
                dataset_id,
                out_ds.id,
                format!("输出 {} 行 / {} 列", out_ds.row_count, out_ds.columns.len()),
                full_sql,
                now()
            ],
        )?;
        Ok((out_ds, output))
    })?;

    Ok(RunResult {
        run_id,
        status: "success".to_string(),
        message: Some(format!(
            "流程执行成功，输出 {} 行 / {} 列",
            out_ds.row_count,
            out_ds.columns.len()
        )),
        log,
        output_dataset_id: Some(out_ds.id),
        output: Some(output),
    })
}

/// 步骤的展示名：优先用户自定义标签，否则回退到操作符中文名。
fn describe_step(step: &FlowStep) -> String {
    if step.label.trim().is_empty() {
        operators::label_of(&step.operator)
    } else {
        step.label.clone()
    }
}

/// 预览流程执行到指定节点之后的数据，不落库、不改动任何数据集。
///
/// `step_index` 为 0 基下标；等于步骤总数时返回整条流程的最终输出
/// （包含 `sort` / `limit` 步骤产生的排序与截断）。这样「数据流」栏目
/// 可以逐个节点查阅中间结果，而不会像 `run_flow` 那样产生新的输出数据集。
pub fn preview_step(
    conn: &Connection,
    flow_id: &str,
    dataset_id: &str,
    step_index: i64,
    offset: i64,
    limit: i64,
) -> AppResult<StepData> {
    let (_, _, steps) = load_flow(conn, flow_id)?;
    if steps.is_empty() {
        return Err(AppError::bad_request("流程没有任何步骤"));
    }
    // 数据集必须存在；缺失时返回明确的 404，而不是一张空表。
    store::get_dataset(conn, dataset_id)?;

    let last = steps.len() as i64;
    if step_index < 0 || step_index > last {
        return Err(AppError::bad_request(format!(
            "节点序号 {step_index} 超出范围（0..{last}）"
        )));
    }

    // 节点 i 的数据 = 执行完第 i 步之后的数据；终点节点执行完全部步骤。
    let upto = (step_index + 1).min(last) as usize;
    let mut c = Compiler::new(conn, dataset_id)?;
    for (idx, step) in steps.iter().enumerate().take(upto) {
        if !step.enabled {
            continue;
        }
        c.step(idx as i64, step).map_err(|msg| {
            AppError::bad_request(format!(
                "第 {} 步「{}」执行失败：{msg}",
                idx + 1,
                describe_step(step)
            ))
        })?;
    }

    let (label, operator) = if step_index as usize == steps.len() {
        ("流程输出".to_string(), "output".to_string())
    } else {
        let step = &steps[step_index as usize];
        (describe_step(step), step.operator.clone())
    };

    // `limit` 步骤会限制最终行数，分页总数同样按该上限截断。
    let count = c
        .count_current()
        .map_err(|msg| AppError::bad_request(format!("统计节点数据失败：{msg}")))?;
    let total = match c.limit {
        Some(n) => count.min(n.max(0)),
        None => count,
    };

    let off = offset.max(0);
    let ask = limit.clamp(0, MAX_PREVIEW_ROWS);
    let take = match c.limit {
        Some(n) => (n - off).max(0).min(ask),
        None => ask,
    };

    let mut base = format!("SELECT * FROM {}", c.cur);
    if let Some(order) = &c.order_by {
        base.push_str(&format!(" ORDER BY {order}"));
    }
    let page_sql = c.with_ctes(&format!("{base} LIMIT {take} OFFSET {off}"));

    let columns = c.columns.clone();
    let ncol = columns.len();
    let mut stmt = conn.prepare(&page_sql)?;
    let mut rows = stmt.query([])?;
    let mut out: Vec<Vec<serde_json::Value>> = Vec::new();
    while let Some(row) = rows.next()? {
        let actual = row.as_ref().column_count();
        let mut vals = Vec::with_capacity(ncol);
        for i in 0..ncol {
            vals.push(if i < actual {
                store::json_value(row.get_ref(i)?)
            } else {
                serde_json::Value::Null
            });
        }
        out.push(vals);
    }

    Ok(StepData {
        step_index,
        label,
        operator,
        columns,
        rows: out,
        total,
    })
}

fn load_flow(conn: &Connection, flow_id: &str) -> AppResult<(String, String, Vec<FlowStep>)> {
    let mut stmt = conn.prepare("SELECT name, project_id, steps FROM flows WHERE id = ?1")?;
    let mut rows = stmt.query(rusqlite::params![flow_id])?;
    let row = rows
        .next()?
        .ok_or_else(|| AppError::not_found(format!("流程不存在: {flow_id}")))?;
    let name: String = row.get(0)?;
    let project_id: String = row.get(1)?;
    let raw: String = row.get(2)?;
    let steps: Vec<FlowStep> = serde_json::from_str(&raw)
        .map_err(|e| AppError::internal_with_context("流程定义 JSON 损坏", e))?;
    Ok((name, project_id, steps))
}

/// 当前作用域内可用的字段名：项目流程查项目模板，全局流程查全局标准字段。
pub(crate) fn scoped_field_names(conn: &Connection, project_id: &str) -> AppResult<Vec<String>> {
    if project_id.trim().is_empty() {
        let mut stmt = conn.prepare("SELECT name FROM standard_fields")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    } else {
        let mut stmt = conn.prepare("SELECT name FROM project_fields WHERE project_id = ?1")?;
        let rows = stmt.query_map(rusqlite::params![project_id], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

/// 流程中声明为必需、但当前作用域模板里不存在的字段（按出现顺序去重）。
pub(crate) fn missing_required_fields(
    conn: &Connection,
    project_id: &str,
    steps: &[FlowStep],
) -> AppResult<Vec<String>> {
    let names = scoped_field_names(conn, project_id)?;
    let mut missing: Vec<String> = Vec::new();
    for step in steps.iter().filter(|s| s.enabled) {
        if step.operator != "require_fields" {
            continue;
        }
        let fields = step
            .params
            .get("fields")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for field in fields {
            if !names.iter().any(|name| name == field) && !missing.iter().any(|m| m == field) {
                missing.push(field.to_string());
            }
        }
    }
    Ok(missing)
}

/// 静态校验：不执行 SQL，只根据字段名推演每一步是否可行。
///
/// 数据流模型：流程中所有「必需字段校验」步骤声明的字段并集被视为输入字段全集，
/// 未声明的字段按用户约定视为不存在；没有必需字段声明时才退回数据集实际列。
/// 每一步都会输出进入/离开时的字段快照，前端据此即时提示流程是否合理。
pub fn validate_steps(steps: &[FlowStep], columns: &[String]) -> ValidateResult {
    validate_steps_with(steps, columns, &HashMap::new())
}

/// 与 `validate_steps` 相同，但可以传入「关联数据表」算子用到的数据集字段，
/// 从而静态推导 JOIN 之后新增的字段名。
pub fn validate_steps_with(
    steps: &[FlowStep],
    columns: &[String],
    join_datasets: &HashMap<String, Vec<DatasetColumn>>,
) -> ValidateResult {
    let mut errors: Vec<ValidateError> = Vec::new();
    let mut flows: Vec<StepFlow> = Vec::new();

    // 当前数据流字段始终以真实数据集列为准；
    // 「必需字段」是校验条件，不能用它替换真实列（否则永远校验不出缺失字段）。
    let mut cols: Vec<String> = columns.to_vec();

    for (idx, step) in steps.iter().enumerate() {
        let i = idx as i64;
        let input_fields = cols.clone();
        let mut step_errors: Vec<String> = Vec::new();

        if !step.enabled {
            flows.push(StepFlow {
                step_index: i,
                input_fields,
                output_fields: cols.clone(),
                errors: step_errors,
            });
            continue;
        }

        let p = if step.params.is_null() {
            Value::Object(Default::default())
        } else {
            step.params.clone()
        };

        match step.operator.as_str() {
            "require_fields" => {
                let fields = list_param(&p, "fields");
                if fields.is_empty() {
                    step_errors.push("必需字段列表为空".into());
                }
                let missing: Vec<String> = fields
                    .iter()
                    .filter(|f| !cols.contains(f))
                    .cloned()
                    .collect();
                if !missing.is_empty() {
                    step_errors.push(format!("缺少必要字段：{}", missing.join("、")));
                }
                if !bool_param(&p, "allow_extra", true) {
                    let extra: Vec<String> = cols
                        .iter()
                        .filter(|c| !fields.contains(c))
                        .cloned()
                        .collect();
                    if !extra.is_empty() {
                        step_errors.push(format!("存在未声明字段：{}", extra.join("、")));
                    }
                }
            }
            "unique_check" | "dedupe" => {
                for c in list_param(&p, "columns") {
                    if !cols.contains(&c) {
                        step_errors.push(format!("字段 [{c}] 不存在"));
                    }
                }
                if step.operator == "dedupe" {
                    let order_by = str_param(&p, "order_by");
                    if !order_by.is_empty() && !cols.contains(&order_by) {
                        step_errors.push(format!("字段 [{order_by}] 不存在"));
                    }
                }
            }
            "map_columns" => {
                // 全局别名表只在运行时可见，这里仅推演显式写出的映射。
                if let Some(arr) = p.get("mappings").and_then(|v| v.as_array()) {
                    for item in arr {
                        let from = item
                            .get("from")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        let to = item
                            .get("to")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        if from.is_empty() || to.is_empty() || from == to {
                            continue;
                        }
                        if let Some(pos) = cols.iter().position(|c| c == &from) {
                            if cols.iter().any(|c| c == &to) {
                                step_errors.push(format!("映射目标字段 [{to}] 已存在"));
                            } else {
                                cols[pos] = to;
                            }
                        }
                    }
                }
            }
            "rename" => {
                let from = str_param(&p, "from");
                let to = str_param(&p, "to");
                if from.is_empty() || to.is_empty() {
                    step_errors.push("原字段与新字段名都不能为空".into());
                } else if !cols.contains(&from) {
                    step_errors.push(format!("字段 [{from}] 不存在"));
                } else if cols.contains(&to) {
                    step_errors.push(format!("字段 [{to}] 已存在"));
                } else if let Some(pos) = cols.iter().position(|c| c == &from) {
                    cols[pos] = to;
                }
            }
            "select" => {
                let keep = list_param(&p, "columns");
                if keep.is_empty() {
                    step_errors.push("保留字段列表为空".into());
                }
                for c in &keep {
                    if !cols.contains(c) {
                        step_errors.push(format!("字段 [{c}] 不存在"));
                    }
                }
                if !keep.is_empty() {
                    cols = keep;
                }
            }
            "cast" | "fill_null" => {
                let c = str_param(&p, "column");
                if !cols.contains(&c) {
                    step_errors.push(format!("字段 [{c}] 不存在"));
                }
            }
            "compute" => {
                let out = str_param(&p, "out");
                let raw = str_param(&p, "expr");
                if out.is_empty() {
                    step_errors.push("新字段名不能为空".into());
                } else if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else if let Err(e) = expr::compile(&raw, &cols) {
                    step_errors.push(e);
                } else {
                    cols.push(out);
                }
            }
            "rank" => {
                let by = str_param(&p, "by");
                if !cols.contains(&by) {
                    step_errors.push(format!("字段 [{by}] 不存在"));
                }
                let out = {
                    let o = str_param(&p, "out");
                    if o.is_empty() {
                        "排名".to_string()
                    } else {
                        o
                    }
                };
                if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else {
                    cols.push(out);
                }
                for c in list_param(&p, "partition_by") {
                    if !cols.contains(&c) {
                        step_errors.push(format!("分组字段 [{c}] 不存在"));
                    }
                }
            }
            "filter" => {
                let raw = str_param(&p, "expr");
                if raw.is_empty() {
                    step_errors.push("筛选条件不能为空".into());
                } else if let Err(e) = expr::compile(&raw, &cols) {
                    step_errors.push(e);
                }
            }
            "sort" => {
                let keys = sorts_param(&p);
                if keys.is_empty() {
                    step_errors.push("请至少设置一个排序字段".into());
                }
                for (field, _) in keys {
                    if !cols.contains(&field) {
                        step_errors.push(format!("字段 [{field}] 不存在"));
                    }
                }
            }
            "limit" | "output" | "random_rows" => {}
            "random_number" => {
                let out = str_param(&p, "out");
                if out.is_empty() {
                    step_errors.push("新字段名不能为空".into());
                } else if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else {
                    cols.push(out);
                }
            }
            "group_aggregate" => {
                let groups = list_param(&p, "group_by");
                for g in &groups {
                    if !cols.contains(g) {
                        step_errors.push(format!("分组字段 [{g}] 不存在"));
                    }
                }
                let aggs = aggregations_param(&p);
                if aggs.is_empty() {
                    step_errors.push("请至少添加一个汇总项".into());
                }
                let mut next: Vec<String> = groups.clone();
                for agg in &aggs {
                    const FUNCS: &[&str] = &[
                        "count",
                        "count_distinct",
                        "sum",
                        "avg",
                        "min",
                        "max",
                        "group_concat",
                        "stddev",
                        "variance",
                    ];
                    if !FUNCS.contains(&agg.func.as_str()) {
                        step_errors.push(format!("不支持的汇总方式：{}", agg.func));
                        continue;
                    }
                    if agg.func != "count" {
                        if agg.field.is_empty() {
                            step_errors.push(format!("{} 汇总必须选择字段", agg.func));
                        } else if !cols.contains(&agg.field) {
                            step_errors.push(format!("汇总字段 [{}] 不存在", agg.field));
                        }
                    }
                    let name = if agg.out.is_empty() {
                        agg_label(&agg.func, &agg.field)
                    } else {
                        agg.out.clone()
                    };
                    next.push(name);
                }
                cols = next;
            }
            "round_number" | "text_pad" | "text_replace" => {
                let col = str_param(&p, "column");
                if !cols.contains(&col) {
                    step_errors.push(format!("字段 [{col}] 不存在"));
                }
                if step.operator == "text_replace" && str_param(&p, "find").is_empty() {
                    step_errors.push("查找内容不能为空".into());
                }
                let out = {
                    let o = str_param(&p, "out");
                    if o.is_empty() {
                        col.clone()
                    } else {
                        o
                    }
                };
                if out != col && cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else if out != col {
                    cols.push(out);
                }
            }
            "text_concat" => {
                let list = list_param(&p, "columns");
                if list.is_empty() {
                    step_errors.push("请至少选择一个参与拼接的字段".into());
                }
                for c in &list {
                    if !cols.contains(c) {
                        step_errors.push(format!("字段 [{c}] 不存在"));
                    }
                }
                let out = str_param(&p, "out");
                if out.is_empty() {
                    step_errors.push("新字段名不能为空".into());
                } else if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else {
                    cols.push(out);
                }
            }
            "text_substr" => {
                let col = str_param(&p, "column");
                if !cols.contains(&col) {
                    step_errors.push(format!("字段 [{col}] 不存在"));
                }
                let out = str_param(&p, "out");
                if out.is_empty() {
                    step_errors.push("新字段名不能为空".into());
                } else if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else {
                    cols.push(out);
                }
            }
            "conditional" => {
                let out = str_param(&p, "out");
                let cond = str_param(&p, "condition");
                if out.is_empty() {
                    step_errors.push("新字段名不能为空".into());
                } else if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                }
                if cond.is_empty() {
                    step_errors.push("判断条件不能为空".into());
                } else if let Err(e) = expr::compile(&cond, &cols) {
                    step_errors.push(e);
                }
                if !out.is_empty() && !cols.contains(&out) {
                    cols.push(out);
                }
            }
            "date_calc" => {
                let col = str_param(&p, "column");
                if !cols.contains(&col) {
                    step_errors.push(format!("字段 [{col}] 不存在"));
                }
                if str_param(&p, "mode") == "diff_days" {
                    let other = str_param(&p, "other");
                    if !cols.contains(&other) {
                        step_errors.push(format!("对比日期字段 [{other}] 不存在"));
                    }
                }
                let out = str_param(&p, "out");
                if out.is_empty() {
                    step_errors.push("新字段名不能为空".into());
                } else if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else {
                    cols.push(out);
                }
            }
            "parse_id_card" => {
                let col = str_param(&p, "column");
                if !cols.contains(&col) {
                    step_errors.push(format!("字段 [{col}] 不存在"));
                }
                let outs = [
                    str_param(&p, "birth_out"),
                    str_param(&p, "gender_out"),
                    str_param(&p, "age_out"),
                    str_param(&p, "valid_out"),
                ];
                if outs.iter().all(|o| o.is_empty()) {
                    step_errors.push("至少需要输出一个解析结果字段".into());
                }
                for out in outs {
                    if !out.is_empty() && !cols.contains(&out) {
                        cols.push(out);
                    }
                }
            }
            "amount_upper" => {
                let col = str_param(&p, "column");
                if !cols.contains(&col) {
                    step_errors.push(format!("字段 [{col}] 不存在"));
                }
                let out = {
                    let o = str_param(&p, "out");
                    if o.is_empty() {
                        "金额大写".to_string()
                    } else {
                        o
                    }
                };
                if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else {
                    cols.push(out);
                }
            }
            "percentage" => {
                let col = str_param(&p, "column");
                if !cols.contains(&col) {
                    step_errors.push(format!("字段 [{col}] 不存在"));
                }
                for c in list_param(&p, "partition_by") {
                    if !cols.contains(&c) {
                        step_errors.push(format!("分组字段 [{c}] 不存在"));
                    }
                }
                let out = {
                    let o = str_param(&p, "out");
                    if o.is_empty() {
                        "占比".to_string()
                    } else {
                        o
                    }
                };
                if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else {
                    cols.push(out);
                }
            }
            "tax_calc" => {
                let col = str_param(&p, "column");
                if !cols.contains(&col) {
                    step_errors.push(format!("字段 [{col}] 不存在"));
                }
                match f64_param(&p, "rate") {
                    Some(rate) if (0.0..=1.0).contains(&rate) => {}
                    Some(_) => step_errors.push("税率应在 0~1 之间（如 13% 填 0.13）".into()),
                    None => step_errors.push("税率不能为空".into()),
                }
                let mode = str_param(&p, "mode");
                if !mode.is_empty() && mode != "exclusive" && mode != "inclusive" {
                    step_errors.push(format!("未知的金额口径: {mode}"));
                }
                let mut outs = Vec::new();
                for (key, fallback) in [
                    ("net_out", "不含税金额"),
                    ("tax_out", "税额"),
                    ("gross_out", "含税金额"),
                ] {
                    let name = defaulted(str_param(&p, key), fallback);
                    if cols.contains(&name) || outs.contains(&name) {
                        step_errors.push(format!("字段 [{name}] 已存在"));
                    } else {
                        outs.push(name);
                    }
                }
                cols.extend(outs);
            }
            "budget_variance" => {
                for (key, label) in [("budget_column", "预算金额"), ("actual_column", "实际金额")]
                {
                    let col = str_param(&p, key);
                    if !cols.contains(&col) {
                        step_errors.push(format!("{label}字段 [{col}] 不存在"));
                    }
                }
                if str_param(&p, "budget_column") == str_param(&p, "actual_column") {
                    step_errors.push("预算金额与实际金额不能是同一字段".into());
                }
                let groups = list_param(&p, "group_by");
                for g in &groups {
                    if !cols.contains(g) {
                        step_errors.push(format!("分组字段 [{g}] 不存在"));
                    }
                }
                let status = defaulted(str_param(&p, "status_out"), "预算状态");
                if groups.contains(&status) {
                    step_errors.push(format!("状态字段 [{status}] 与分组字段重名"));
                }
                let mut next = groups;
                for name in ["预算数", "实际数", "执行率", "差额"] {
                    next.push(name.to_string());
                }
                next.push(status);
                cols = next;
            }
            "loan_payment" => {
                let principal = str_param(&p, "principal");
                if !cols.contains(&principal) {
                    step_errors.push(format!("字段 [{principal}] 不存在"));
                }
                match f64_param(&p, "annual_rate") {
                    Some(rate) if (0.0..=1.0).contains(&rate) => {}
                    Some(_) => step_errors.push("年利率应在 0~1 之间（如 4.9% 填 0.049）".into()),
                    None => step_errors.push("年利率不能为空".into()),
                }
                if num_param(&p, "years").unwrap_or(10) <= 0 {
                    step_errors.push("期限必须大于 0 年".into());
                }
                let mut outs = Vec::new();
                for (key, fallback) in [
                    ("payment_out", "月供"),
                    ("interest_out", "总利息"),
                    ("total_out", "还款总额"),
                ] {
                    let name = defaulted(str_param(&p, key), fallback);
                    if cols.contains(&name) || outs.contains(&name) {
                        step_errors.push(format!("字段 [{name}] 已存在"));
                    } else {
                        outs.push(name);
                    }
                }
                cols.extend(outs);
            }
            "reconcile" => {
                for (key, label) in [("left_key", "本方关键"), ("left_amount", "本方金额")]
                {
                    let col = str_param(&p, key);
                    if !cols.contains(&col) {
                        step_errors.push(format!("{label}字段 [{col}] 不存在"));
                    }
                }
                let right_id = str_param(&p, "dataset_id");
                if right_id.is_empty() {
                    step_errors.push("请选择对方数据集".into());
                } else if let Some(right_cols) = join_datasets.get(&right_id) {
                    for (key, label) in [("right_key", "对方关键"), ("right_amount", "对方金额")]
                    {
                        let col = str_param(&p, key);
                        if !right_cols.iter().any(|c| c.name == col) {
                            step_errors.push(format!("{label}字段 [{col}] 不存在"));
                        }
                    }
                }
                match f64_param(&p, "tolerance") {
                    Some(v) if v >= 0.0 => {}
                    Some(_) => step_errors.push("允许差异不能为负数".into()),
                    None => {}
                }
                let mode = str_param(&p, "mode");
                if !mode.is_empty() && mode != "diff" && mode != "all" {
                    step_errors.push(format!("未知的输出范围: {mode}"));
                }
                cols = vec![
                    "对账键".to_string(),
                    "本方金额".to_string(),
                    "对方金额".to_string(),
                    "差异".to_string(),
                    "对账结果".to_string(),
                ];
            }
            "depreciation" => {
                let original = str_param(&p, "original");
                if !cols.contains(&original) {
                    step_errors.push(format!("字段 [{original}] 不存在"));
                }
                let months = str_param(&p, "months");
                if !months.is_empty() && !cols.contains(&months) {
                    step_errors.push(format!("已计提月数字段 [{months}] 不存在"));
                }
                if f64_param(&p, "years").unwrap_or(5.0) <= 0.0 {
                    step_errors.push("使用年限必须大于 0".into());
                }
                for (key, default) in [
                    ("monthly_out", "月折旧额"),
                    ("accumulated_out", "累计折旧"),
                    ("net_out", "固定资产净值"),
                ] {
                    let name = {
                        let v = str_param(&p, key);
                        if v.is_empty() {
                            default.to_string()
                        } else {
                            v
                        }
                    };
                    if cols.contains(&name) {
                        step_errors.push(format!("字段 [{name}] 已存在"));
                    } else {
                        cols.push(name);
                    }
                }
            }
            "allocate" => {
                for key in ["amount", "weight"] {
                    let c = str_param(&p, key);
                    if !cols.contains(&c) {
                        step_errors.push(format!("字段 [{c}] 不存在"));
                    }
                }
                for c in list_param(&p, "partition_by") {
                    if !cols.contains(&c) {
                        step_errors.push(format!("分组字段 [{c}] 不存在"));
                    }
                }
                let out = str_param(&p, "out");
                if out.is_empty() {
                    step_errors.push("分摊结果字段名不能为空".into());
                } else if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else {
                    cols.push(out);
                }
            }
            "data_profile" => {
                for c in list_param(&p, "columns") {
                    if !cols.contains(&c) {
                        step_errors.push(format!("字段 [{c}] 不存在"));
                    }
                }
                cols = vec![
                    "字段名".to_string(),
                    "非空数".to_string(),
                    "空值数".to_string(),
                    "去重数".to_string(),
                    "最小值".to_string(),
                    "最大值".to_string(),
                    "平均值".to_string(),
                ];
            }
            "not_null_check" => {
                let list = list_param(&p, "columns");
                if list.is_empty() {
                    step_errors.push("请至少选择一个必须非空的字段".into());
                }
                for c in &list {
                    if !cols.contains(c) {
                        step_errors.push(format!("字段 [{c}] 不存在"));
                    }
                }
            }
            "column_compare" => {
                for key in ["left", "right"] {
                    let c = str_param(&p, key);
                    if !cols.contains(&c) {
                        step_errors.push(format!("字段 [{c}] 不存在"));
                    }
                }
            }
            "format_check" => {
                let c = str_param(&p, "column");
                if !cols.contains(&c) {
                    step_errors.push(format!("字段 [{c}] 不存在"));
                }
            }
            "sum_check" => {
                let total = str_param(&p, "total");
                let parts = list_param(&p, "parts");
                if total.is_empty() {
                    step_errors.push("合计字段不能为空".into());
                } else if !cols.contains(&total) {
                    step_errors.push(format!("字段 [{total}] 不存在"));
                }
                if parts.is_empty() {
                    step_errors.push("明细字段不能为空".into());
                }
                for c in &parts {
                    if !cols.contains(c) {
                        step_errors.push(format!("字段 [{c}] 不存在"));
                    }
                }
            }
            "shuffle" => {
                for g in list_param(&p, "group_by") {
                    if !cols.contains(&g) {
                        step_errors.push(format!("分组字段 [{g}] 不存在"));
                    }
                }
                let out = defaulted(str_param(&p, "out"), "随机序号");
                if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else {
                    cols.push(out);
                }
            }
            "assign_seats" => {
                let room = {
                    let v = str_param(&p, "room_out");
                    if v.is_empty() {
                        "考场号".to_string()
                    } else {
                        v
                    }
                };
                let seat = {
                    let v = str_param(&p, "seat_out");
                    if v.is_empty() {
                        "座位号".to_string()
                    } else {
                        v
                    }
                };
                for out in [&room, &seat] {
                    if cols.contains(out) {
                        step_errors.push(format!("字段 [{out}] 已存在"));
                    } else {
                        cols.push(out.clone());
                    }
                }
                let order_by = str_param(&p, "order_by");
                if !order_by.is_empty() && !cols.contains(&order_by) {
                    step_errors.push(format!("排序字段 [{order_by}] 不存在"));
                }
                if num_param(&p, "per_room").unwrap_or(30) <= 0 {
                    step_errors.push("每考场人数必须大于 0".into());
                }
            }
            "range_check" => {
                let c = str_param(&p, "column");
                if !cols.contains(&c) {
                    step_errors.push(format!("字段 [{c}] 不存在"));
                }
                if f64_param(&p, "min").is_none() && f64_param(&p, "max").is_none() {
                    step_errors.push("请至少设置最小值或最大值".into());
                }
            }
            "balance_check" => {
                for key in ["voucher", "debit", "credit"] {
                    let c = str_param(&p, key);
                    if c.is_empty() {
                        step_errors.push(format!("{} 不能为空", key));
                    } else if !cols.contains(&c) {
                        step_errors.push(format!("字段 [{c}] 不存在"));
                    }
                }
            }
            "append_dataset" => {
                if str_param(&p, "dataset_id").is_empty() {
                    step_errors.push("请选择要追加的数据集".into());
                }
            }
            "join_dataset" => {
                let left = str_param(&p, "left_field");
                if !cols.contains(&left) {
                    step_errors.push(format!("字段 [{left}] 不存在"));
                }
                let dataset_id = str_param(&p, "dataset_id");
                if dataset_id.is_empty() {
                    step_errors.push("请选择关联数据集".into());
                } else if let Some(right_cols) = join_datasets.get(&dataset_id) {
                    let right_field = str_param(&p, "right_field");
                    if !right_cols.iter().any(|c| c.name == right_field) {
                        step_errors.push(format!("关联表中没有字段 [{right_field}]"));
                    }
                    match join_output_plan(
                        &cols,
                        right_cols,
                        &list_param(&p, "fields"),
                        &str_param(&p, "prefix"),
                    ) {
                        Ok(plan) => cols.extend(plan.into_iter().map(|(_, out, _)| out)),
                        Err(e) => step_errors.push(e),
                    }
                }
            }
            "split_column" => {
                let c = str_param(&p, "column");
                if !cols.contains(&c) {
                    step_errors.push(format!("字段 [{c}] 不存在"));
                }
                let parts = num_param(&p, "max_parts").unwrap_or(2);
                if !(1..=20).contains(&parts) {
                    step_errors.push("拆分段数应在 1~20 之间".into());
                }
            }
            "value_map" | "text_clean" | "aging_bucket" => {
                let c = str_param(&p, "column");
                if !cols.contains(&c) {
                    step_errors.push(format!("字段 [{c}] 不存在"));
                }
                if step.operator == "value_map" {
                    let has = p
                        .get("mappings")
                        .and_then(|v| v.as_array())
                        .map(|a| !a.is_empty())
                        .unwrap_or(false);
                    if !has {
                        step_errors.push("请至少添加一条值映射".into());
                    }
                }
                let out = {
                    let o = str_param(&p, "out");
                    if o.is_empty() {
                        c.clone()
                    } else {
                        o
                    }
                };
                if out != c && cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else if out != c {
                    cols.push(out);
                }
            }
            "sequence" => {
                let out = {
                    let o = str_param(&p, "out");
                    if o.is_empty() {
                        "序号".to_string()
                    } else {
                        o
                    }
                };
                if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else {
                    cols.push(out);
                }
                let order_by = str_param(&p, "order_by");
                if !order_by.is_empty() && !cols.contains(&order_by) {
                    step_errors.push(format!("排序字段 [{order_by}] 不存在"));
                }
                for c in list_param(&p, "partition_by") {
                    if !cols.contains(&c) {
                        step_errors.push(format!("分组字段 [{c}] 不存在"));
                    }
                }
            }
            "multi_column_stat" => {
                let list = list_param(&p, "columns");
                if list.is_empty() {
                    step_errors.push("请至少选择一个参与计算的字段".into());
                }
                for c in &list {
                    if !cols.contains(c) {
                        step_errors.push(format!("字段 [{c}] 不存在"));
                    }
                }
                let out = str_param(&p, "out");
                if out.is_empty() {
                    step_errors.push("输出字段名不能为空".into());
                } else if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else {
                    cols.push(out);
                }
            }
            "cumulative" | "window_compare" => {
                let c = str_param(&p, "column");
                if !cols.contains(&c) {
                    step_errors.push(format!("字段 [{c}] 不存在"));
                }
                let order_by = str_param(&p, "order_by");
                if step.operator == "window_compare" && order_by.is_empty() {
                    step_errors.push("请选择期间顺序字段".into());
                }
                if !order_by.is_empty() && !cols.contains(&order_by) {
                    step_errors.push(format!("排序字段 [{order_by}] 不存在"));
                }
                for part in list_param(&p, "partition_by") {
                    if !cols.contains(&part) {
                        step_errors.push(format!("分组字段 [{part}] 不存在"));
                    }
                }
                let out = str_param(&p, "out");
                if out.is_empty() {
                    step_errors.push("输出字段名不能为空".into());
                } else if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else {
                    cols.push(out);
                }
            }
            "grade_level" => {
                let c = str_param(&p, "column");
                if !cols.contains(&c) {
                    step_errors.push(format!("字段 [{c}] 不存在"));
                }
                let cuts = str_param(&p, "cutoffs");
                if cuts
                    .split([',', '，'])
                    .all(|s| s.trim().parse::<f64>().is_err())
                {
                    step_errors.push("请至少设置一个有效分数阈值".into());
                }
                let out = {
                    let o = str_param(&p, "out");
                    if o.is_empty() {
                        "等级".to_string()
                    } else {
                        o
                    }
                };
                if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else {
                    cols.push(out);
                }
            }
            "standard_score" => {
                let c = str_param(&p, "column");
                if !cols.contains(&c) {
                    step_errors.push(format!("字段 [{c}] 不存在"));
                }
                for part in list_param(&p, "partition_by") {
                    if !cols.contains(&part) {
                        step_errors.push(format!("分组字段 [{part}] 不存在"));
                    }
                }
                let out = {
                    let o = str_param(&p, "out");
                    if o.is_empty() {
                        "标准分".to_string()
                    } else {
                        o
                    }
                };
                if cols.contains(&out) {
                    step_errors.push(format!("字段 [{out}] 已存在"));
                } else {
                    cols.push(out);
                }
            }
            "score_summary" => {
                let c = str_param(&p, "column");
                if !cols.contains(&c) {
                    step_errors.push(format!("字段 [{c}] 不存在"));
                }
                let mut next = list_param(&p, "group_by");
                for g in &next {
                    if !cols.contains(g) {
                        step_errors.push(format!("分组字段 [{g}] 不存在"));
                    }
                }
                for name in [
                    "人数",
                    "平均分",
                    "最高分",
                    "最低分",
                    "及格率",
                    "优秀率",
                    "标准差",
                ] {
                    next.push(name.to_string());
                }
                cols = next;
            }
            "score_segment" => {
                let c = str_param(&p, "column");
                if !cols.contains(&c) {
                    step_errors.push(format!("字段 [{c}] 不存在"));
                }
                let parts = list_param(&p, "partition_by");
                for part in &parts {
                    if !cols.contains(part) {
                        step_errors.push(format!("分组字段 [{part}] 不存在"));
                    }
                }
                let seg = {
                    let v = str_param(&p, "segment_out");
                    if v.is_empty() {
                        "分数段".to_string()
                    } else {
                        v
                    }
                };
                let cnt = {
                    let v = str_param(&p, "count_out");
                    if v.is_empty() {
                        "人数".to_string()
                    } else {
                        v
                    }
                };
                let mut next = parts;
                next.push(seg);
                next.push(cnt);
                if bool_param(&p, "cumulative", true) {
                    next.push("累计人数".to_string());
                }
                cols = next;
            }
            "pivot" => {
                let groups = list_param(&p, "group_by");
                for c in &groups {
                    if !cols.contains(c) {
                        step_errors.push(format!("分组字段 [{c}] 不存在"));
                    }
                }
                for key in ["pivot_column", "value_column"] {
                    let c = str_param(&p, key);
                    if !cols.contains(&c) {
                        step_errors.push(format!("字段 [{c}] 不存在"));
                    }
                }
                // 展开后的列名取决于数据，静态校验只能推演分组字段。
                cols = groups;
            }
            other => step_errors.push(format!("不支持的操作符: {other}")),
        }

        for message in &step_errors {
            errors.push(ValidateError {
                step_index: i,
                message: message.clone(),
            });
        }
        flows.push(StepFlow {
            step_index: i,
            input_fields,
            output_fields: cols.clone(),
            errors: step_errors,
        });
    }

    ValidateResult {
        ok: errors.is_empty(),
        errors,
        steps: flows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DatasetColumn;
    use rusqlite::Connection;
    use serde_json::json;

    fn setup() -> (Connection, String) {
        let mut conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch(crate::db::SCHEMA).expect("schema");
        let cols = vec![
            DatasetColumn {
                name: "姓名".into(),
                dtype: "text".into(),
            },
            DatasetColumn {
                name: "笔试成绩".into(),
                dtype: "number".into(),
            },
            DatasetColumn {
                name: "面试成绩".into(),
                dtype: "number".into(),
            },
        ];
        let rows = vec![
            vec![Some("张三".into()), Some("80".into()), Some("90".into())],
            vec![Some("李四".into()), Some("90".into()), Some("80".into())],
            vec![Some("王五".into()), Some("70".into()), Some("60".into())],
        ];
        let ds = store::create_dataset(
            &mut conn,
            store::NewDataset {
                name: "成绩",
                kind: "import",
                source_file: "",
                origin: "单元测试",
                project_id: "",
                columns: &cols,
                rows: &rows,
            },
        )
        .expect("dataset");
        (conn, ds.id)
    }

    fn setup_custom(cols: &[(&str, &str)], rows: &[Vec<&str>]) -> (Connection, String) {
        let mut conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch(crate::db::SCHEMA).expect("schema");
        let columns: Vec<DatasetColumn> = cols
            .iter()
            .map(|(name, dtype)| DatasetColumn {
                name: name.to_string(),
                dtype: dtype.to_string(),
            })
            .collect();
        let rows: Vec<Vec<Option<String>>> = rows
            .iter()
            .map(|r| r.iter().map(|v| Some(v.to_string())).collect())
            .collect();
        let ds = store::create_dataset(
            &mut conn,
            store::NewDataset {
                name: "测试数据",
                kind: "import",
                source_file: "",
                origin: "单元测试",
                project_id: "",
                columns: &columns,
                rows: &rows,
            },
        )
        .expect("dataset");
        (conn, ds.id)
    }

    fn run_steps(
        conn: &mut Connection,
        ds_id: &str,
        flow_id: &str,
        steps_value: Value,
    ) -> RunResult {
        insert_flow(conn, flow_id, steps_value);
        run_flow(conn, flow_id, ds_id).expect("run")
    }

    fn insert_flow(conn: &Connection, id: &str, steps: Value) {
        conn.execute(
            "INSERT INTO flows (id, name, steps, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            rusqlite::params![id, "测试流程", steps.to_string(), now()],
        )
        .expect("flow");
    }

    fn steps(v: Value) -> Vec<FlowStep> {
        serde_json::from_value(v).expect("valid steps")
    }

    #[test]
    fn run_flow_materializes_ranked_dataset() {
        let (mut conn, ds_id) = setup();
        insert_flow(
            &conn,
            "f_rank",
            json!([
                { "id": "s1", "operator": "require_fields",
                  "params": { "fields": ["姓名", "笔试成绩", "面试成绩"] } },
                { "id": "s2", "operator": "compute",
                  "params": { "out": "总成绩", "expr": "[笔试成绩]*0.6 + [面试成绩]*0.4", "decimals": 2 } },
                { "id": "s3", "operator": "rank",
                  "params": { "by": "总成绩", "order": "desc", "out": "排名" } },
                { "id": "s4", "operator": "sort", "params": { "by": "总成绩", "order": "desc" } },
                { "id": "s5", "operator": "output", "params": { "name": "排名结果" } }
            ]),
        );

        let result = run_flow(&mut conn, "f_rank", &ds_id).expect("run");
        assert_eq!(result.status, "success");
        let out_id = result.output_dataset_id.expect("output id");
        let page = store::read_rows(&conn, &out_id, 0, 10).expect("read output");
        assert_eq!(page.total, 3);
        assert_eq!(page.rows[0][0], json!("李四"));
        // 列顺序：姓名 / 笔试成绩 / 面试成绩 / 总成绩 / 排名
        assert_eq!(page.rows[0][3], json!(86.0));
        assert_eq!(page.rows[0][4], json!(1.0));

        let run_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM flow_runs WHERE status = 'success'",
                [],
                |r| r.get(0),
            )
            .expect("count runs");
        assert_eq!(run_rows, 1);
    }

    #[test]
    fn project_flow_fails_when_required_field_missing_from_template() {
        let (mut conn, ds_id) = setup();
        conn.execute(
            "INSERT INTO project_fields
               (id, project_id, name, label, created_at, updated_at)
             VALUES ('pf1', 'p1', '姓名', '姓名', 'now', 'now')",
            [],
        )
        .expect("project field");
        conn.execute(
            "INSERT INTO flows (id, name, steps, project_id, created_at, updated_at)
             VALUES ('f_proj', '项目流程', ?1, 'p1', 'now', 'now')",
            rusqlite::params![json!([
                { "id": "s1", "operator": "require_fields",
                  "params": { "fields": ["姓名", "身份证号"] } }
            ])
            .to_string()],
        )
        .expect("flow");

        let result = run_flow(&mut conn, "f_proj", &ds_id).expect("run returns result");
        assert_eq!(result.status, "failed");
        assert!(result.output_dataset_id.is_none());
        assert!(result.message.expect("message").contains("身份证号"));
    }

    #[test]
    fn failed_flow_records_run_without_creating_dataset() {
        let (mut conn, ds_id) = setup();
        insert_flow(
            &conn,
            "f_bad",
            json!([{ "id": "s1", "operator": "require_fields",
                     "params": { "fields": ["身份证号"] } }]),
        );
        let result = run_flow(&mut conn, "f_bad", &ds_id).expect("run returns result");
        assert_eq!(result.status, "failed");
        assert!(result.output_dataset_id.is_none());

        let outputs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM datasets WHERE kind = 'flow_output'",
                [],
                |r| r.get(0),
            )
            .expect("count outputs");
        assert_eq!(outputs, 0);
        let status: String = conn
            .query_row(
                "SELECT status FROM flow_runs WHERE flow_id = 'f_bad'",
                [],
                |r| r.get(0),
            )
            .expect("run row");
        assert_eq!(status, "failed");
    }

    #[test]
    fn preview_step_returns_rows_for_each_node() {
        let (conn, ds_id) = setup();
        insert_flow(
            &conn,
            "f_prev",
            json!([
                { "id": "s1", "operator": "require_fields",
                  "params": { "fields": ["姓名", "笔试成绩", "面试成绩"] } },
                { "id": "s2", "operator": "compute",
                  "params": { "out": "总成绩", "expr": "[笔试成绩]*0.6 + [面试成绩]*0.4", "decimals": 2 } },
                { "id": "s3", "operator": "rank",
                  "params": { "by": "总成绩", "order": "desc", "out": "排名" } },
                { "id": "s4", "operator": "sort", "params": { "by": "总成绩", "order": "desc" } },
                { "id": "s5", "operator": "output", "params": { "name": "排名结果" } }
            ]),
        );

        // 第 0 步（必需字段校验）后仍是原始 3 列
        let after_check = preview_step(&conn, "f_prev", &ds_id, 0, 0, 10).expect("step 0");
        assert_eq!(after_check.total, 3);
        assert_eq!(after_check.columns.len(), 3);
        assert_eq!(after_check.label, "必需字段校验");

        // 第 1 步（计算总成绩）后新增一列，数据仍是原始 3 行
        let after_compute = preview_step(&conn, "f_prev", &ds_id, 1, 0, 10).expect("step 1");
        assert_eq!(after_compute.columns.len(), 4);
        assert_eq!(after_compute.total, 3);
        let li = after_compute
            .rows
            .iter()
            .find(|row| row[0] == json!("李四"))
            .expect("李四行");
        assert_eq!(li[3], json!(86.0));

        // 第 2 步（排名）后新增排名列，此时尚未排序
        let after_rank = preview_step(&conn, "f_prev", &ds_id, 2, 0, 10).expect("step 2");
        assert_eq!(after_rank.columns.len(), 5);
        let li = after_rank
            .rows
            .iter()
            .find(|row| row[0] == json!("李四"))
            .expect("李四行");
        // ROW_NUMBER() 返回整数，排名列为整数值
        assert_eq!(li[4], json!(1));

        // 最终输出（step_index == 步骤数）应用了排序
        let final_data = preview_step(&conn, "f_prev", &ds_id, 5, 0, 10).expect("final");
        assert_eq!(final_data.label, "流程输出");
        assert_eq!(final_data.rows[0][0], json!("李四"));
        assert_eq!(final_data.rows[0][4], json!(1));
        assert_eq!(final_data.total, 3);
    }

    #[test]
    fn preview_step_rejects_out_of_range_index() {
        let (conn, ds_id) = setup();
        insert_flow(
            &conn,
            "f_prev_bad",
            json!([{ "id": "s1", "operator": "output", "params": { "name": "结果" } }]),
        );
        let err = preview_step(&conn, "f_prev_bad", &ds_id, 9, 0, 10).expect_err("越界必须报错");
        assert_eq!(err.code, "bad_request");
        assert!(err.message.contains("超出范围"));
    }

    #[test]
    fn validate_reports_missing_fields_without_touching_db() {
        let result = validate_steps(
            &steps(json!([{ "id": "s1", "operator": "require_fields",
                           "params": { "fields": ["姓名", "身份证号"] } }])),
            &["姓名".to_string()],
        );
        assert!(!result.ok);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].message.contains("身份证号"));
    }

    #[test]
    fn new_row_operators_add_expected_columns() {
        let (mut conn, ds_id) = setup();
        insert_flow(
            &conn,
            "f_new",
            json!([
                { "id": "s1", "operator": "text_substr",
                  "params": { "column": "姓名", "start": 1, "length": 1, "out": "姓氏" } },
                { "id": "s2", "operator": "conditional",
                  "params": { "condition": "[笔试成绩] >= 80", "then": "合格", "else": "不合格", "out": "结论" } },
                { "id": "s3", "operator": "amount_upper",
                  "params": { "column": "笔试成绩", "out": "金额大写" } },
                { "id": "s4", "operator": "percentage",
                  "params": { "column": "面试成绩", "out": "占比", "decimals": 2 } },
                { "id": "s5", "operator": "random_number",
                  "params": { "out": "抽签号", "mode": "integer", "min": 1, "max": 10 } },
                { "id": "s6", "operator": "sort",
                  "params": { "keys": [{ "field": "笔试成绩", "order": "asc" }] } },
                { "id": "s7", "operator": "output", "params": { "name": "处理结果" } }
            ]),
        );
        let result = run_flow(&mut conn, "f_new", &ds_id).expect("run");
        assert_eq!(
            result.status,
            "success",
            "{}",
            result.message.clone().unwrap_or_default()
        );
        let page = store::read_rows(&conn, &result.output_dataset_id.expect("output id"), 0, 10)
            .expect("read");
        let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
        for want in ["姓氏", "结论", "金额大写", "占比", "抽签号"] {
            assert!(names.contains(&want), "缺少列 {want}");
        }
        // 升序后第一行是笔试 70 的王五
        assert_eq!(page.rows[0][0], json!("王五"));
        let idx = |name: &str| names.iter().position(|n| *n == name).expect("column");
        assert_eq!(page.rows[0][idx("结论")], json!("不合格"));
        assert_eq!(page.rows[0][idx("金额大写")], json!("柒拾元整"));
        assert_eq!(page.rows[0][idx("姓氏")], json!("王"));
        let pct = page.rows[0][idx("占比")].as_f64().expect("number");
        assert!((pct - 26.09).abs() < 0.01, "占比实际 {pct}");
    }

    #[test]
    fn group_aggregate_computes_counts_and_averages() {
        let (mut conn, ds_id) = setup();
        insert_flow(
            &conn,
            "f_group",
            json!([
                { "id": "s1", "operator": "conditional",
                  "params": { "condition": "[笔试成绩] >= 80", "then": "1", "else": "2", "out": "组别" } },
                { "id": "s2", "operator": "group_aggregate",
                  "params": {
                      "group_by": ["组别"],
                      "aggregations": [
                          { "func": "count" },
                          { "func": "avg", "field": "面试成绩", "out": "面试均分", "decimals": 1 },
                          { "func": "max", "field": "笔试成绩", "out": "最高笔试" },
                          { "func": "stddev", "field": "面试成绩", "out": "面试标准差", "decimals": 2 }
                      ]
                  } },
                { "id": "s3", "operator": "sort",
                  "params": { "keys": [{ "field": "组别", "order": "asc" }] } },
                { "id": "s4", "operator": "output", "params": { "name": "分组统计" } }
            ]),
        );
        let result = run_flow(&mut conn, "f_group", &ds_id).expect("run");
        assert_eq!(
            result.status,
            "success",
            "{}",
            result.message.clone().unwrap_or_default()
        );
        let page = store::read_rows(&conn, &result.output_dataset_id.expect("output id"), 0, 10)
            .expect("read");
        assert_eq!(page.total, 2);
        assert_eq!(page.columns[0].name, "组别");
        let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"数量"));
        assert!(names.contains(&"面试均分"));
        let idx = |name: &str| names.iter().position(|n| *n == name).expect("column");
        // 组别 1：张三(80/90)、李四(90/80)
        assert_eq!(page.rows[0][idx("数量")].as_f64(), Some(2.0));
        assert_eq!(page.rows[0][idx("面试均分")].as_f64(), Some(85.0));
        assert_eq!(page.rows[0][idx("最高笔试")].as_f64(), Some(90.0));
        assert_eq!(
            page.rows[0][idx("面试标准差")].as_f64(),
            Some(5.0),
            "实际：{:?}",
            page.rows[0][idx("面试标准差")]
        );
    }

    #[test]
    fn parse_id_card_extracts_birth_gender_and_validity() {
        let conn = Connection::open_in_memory().expect("db");
        conn.execute_batch(crate::db::SCHEMA).expect("schema");
        crate::db::register_functions(&conn).expect("functions");
        let (birth, gender, valid): (Option<String>, Option<String>, i64) = conn
            .query_row(
                "SELECT id_card_birth('11010519491231002X'), id_card_gender('11010519491231002X'), id_card_valid('11010519491231002X')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("query");
        assert_eq!(birth.as_deref(), Some("1949-12-31"));
        assert_eq!(gender.as_deref(), Some("女"));
        assert_eq!(valid, 1);
        let bad: i64 = conn
            .query_row("SELECT id_card_valid('110105194912310021')", [], |r| {
                r.get(0)
            })
            .expect("query");
        assert_eq!(bad, 0);
    }

    #[test]
    fn date_calc_and_normalize_date_handle_multiple_formats() {
        let conn = Connection::open_in_memory().expect("db");
        crate::db::register_functions(&conn).expect("functions");
        let (d1, d2, d3): (String, String, String) = conn
            .query_row(
                "SELECT norm_date('2010/5/3'), norm_date('20100503'), norm_date('2010年5月3日')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("query");
        assert_eq!(d1, "2010-05-03");
        assert_eq!(d2, "2010-05-03");
        assert_eq!(d3, "2010-05-03");
    }

    #[test]
    fn exam_score_operators_compute_total_level_and_standard_score() {
        let (mut conn, ds_id) = setup();
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_exam_ops",
            json!([
                { "id": "s1", "operator": "multi_column_stat",
                  "params": { "columns": ["笔试成绩", "面试成绩"], "mode": "sum", "out": "总分", "decimals": 0 } },
                { "id": "s2", "operator": "sequence",
                  "params": { "out": "序号", "start": 1 } },
                { "id": "s3", "operator": "grade_level",
                  "params": { "column": "总分", "cutoffs": "160,140", "labels": "A,B,C", "out": "等级" } },
                { "id": "s4", "operator": "standard_score",
                  "params": { "column": "总分", "mode": "z", "out": "Z分", "decimals": 2 } },
                { "id": "s5", "operator": "output", "params": { "name": "成绩分析" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        assert_eq!(page.rows.len(), 3);
        assert_eq!(page.rows[0][3], json!(170.0));
        assert_eq!(page.rows[0][4], json!(1.0));
        assert_eq!(page.rows[0][5], json!("A"));
        assert_eq!(page.rows[2][5], json!("C"));
        assert!(page.rows[0][6].as_f64().is_some());
    }

    #[test]
    fn cleaning_operators_split_clean_and_map_values() {
        let (mut conn, ds_id) = setup_custom(
            &[("编号", "text"), ("座位", "text"), ("备注", "text")],
            &[vec!["001", "A-1", "　１２３ "], vec!["002", "B-2", "abc"]],
        );
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_clean",
            json!([
                { "id": "s1", "operator": "split_column",
                  "params": { "column": "座位", "delimiter": "-", "max_parts": 2, "names": "考场,座位号" } },
                { "id": "s2", "operator": "text_clean",
                  "params": { "column": "备注", "mode": "full_to_half", "out": "清洗后" } },
                { "id": "s3", "operator": "value_map",
                  "params": { "column": "备注", "mappings": [{ "from": "abc", "to": "ABC" }], "out": "映射后" } },
                { "id": "s4", "operator": "output", "params": { "name": "清洗结果" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        assert_eq!(page.columns[3].name, "考场");
        assert_eq!(page.columns[4].name, "座位号");
        assert_eq!(page.rows[0][3], json!("A"));
        assert_eq!(page.rows[0][4], json!("1"));
        assert_eq!(page.rows[0][5], json!(" 123 "));
        assert_eq!(page.rows[1][6], json!("ABC"));
    }

    #[test]
    fn finance_operators_check_balance_and_build_aging_and_pivot() {
        let (mut conn, ds_id) = setup_custom(
            &[
                ("凭证号", "text"),
                ("科目", "text"),
                ("借方", "number"),
                ("贷方", "number"),
                ("日期", "text"),
                ("金额", "number"),
                ("客户", "text"),
            ],
            &[
                vec!["P1", "收入", "0", "500", "2024-01-05", "500", "甲"],
                vec!["P1", "银行", "500", "0", "2024-01-05", "500", "甲"],
                vec!["P2", "费用", "100", "0", "2024-02-05", "100", "乙"],
                vec!["P2", "银行", "0", "90", "2024-02-05", "90", "乙"],
            ],
        );
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_finance",
            json!([
                { "id": "s1", "operator": "balance_check",
                  "params": { "voucher": "凭证号", "debit": "借方", "credit": "贷方", "tolerance": 0.01, "on_error": "warn" } },
                { "id": "s2", "operator": "cumulative",
                  "params": { "column": "金额", "out": "累计金额", "order_by": "日期", "partition_by": ["客户"], "decimals": 2 } },
                { "id": "s3", "operator": "aging_bucket",
                  "params": { "column": "日期", "mode": "date", "buckets": "30,60,90,180", "out": "账龄" } },
                { "id": "s4", "operator": "pivot",
                  "params": { "group_by": ["科目"], "pivot_column": "客户", "value_column": "金额", "agg": "sum", "decimals": 0 } },
                { "id": "s5", "operator": "output", "params": { "name": "财务分析" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"甲"));
        assert!(names.contains(&"乙"));
        assert_eq!(page.rows.len(), 3);
        let bank = page
            .rows
            .iter()
            .find(|r| r[0] == json!("银行"))
            .expect("银行行");
        let jia = names.iter().position(|n| *n == "甲").unwrap();
        let yi = names.iter().position(|n| *n == "乙").unwrap();
        assert_eq!(bank[jia], json!(500.0));
        assert_eq!(bank[yi], json!(90.0));
    }

    #[test]
    fn append_dataset_unions_rows_by_column_name() {
        let (mut conn, left_id) = setup_custom(
            &[("学号", "text"), ("姓名", "text")],
            &[vec!["001", "张三"]],
        );
        let right_cols = vec![
            DatasetColumn {
                name: "姓名".into(),
                dtype: "text".into(),
            },
            DatasetColumn {
                name: "学号".into(),
                dtype: "text".into(),
            },
            DatasetColumn {
                name: "备注".into(),
                dtype: "text".into(),
            },
        ];
        let right_rows = vec![
            vec![Some("李四".into()), Some("002".into()), Some("补录".into())],
            vec![Some("王五".into()), Some("003".into()), Some("补录".into())],
        ];
        let right = store::create_dataset(
            &mut conn,
            store::NewDataset {
                name: "补录名单",
                kind: "import",
                source_file: "",
                origin: "单元测试",
                project_id: "",
                columns: &right_cols,
                rows: &right_rows,
            },
        )
        .expect("right dataset");
        let result = run_steps(
            &mut conn,
            &left_id,
            "f_append",
            json!([
                { "id": "s1", "operator": "append_dataset",
                  "params": { "dataset_id": right.id.clone(), "dedupe": false } },
                { "id": "s2", "operator": "output", "params": { "name": "合并名单" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        assert_eq!(page.columns.len(), 2);
        assert_eq!(page.columns[0].name, "学号");
        assert_eq!(page.rows.len(), 3);
        assert_eq!(page.rows[1][0], json!("002"));
        assert_eq!(page.rows[1][1], json!("李四"));
    }

    #[test]
    fn join_dataset_brings_columns_from_another_dataset() {
        let (mut conn, left_id) = setup_custom(
            &[("学号", "text"), ("姓名", "text")],
            &[vec!["001", "张三"], vec!["002", "李四"]],
        );
        let right_cols = vec![
            DatasetColumn {
                name: "学号".into(),
                dtype: "text".into(),
            },
            DatasetColumn {
                name: "手机号".into(),
                dtype: "text".into(),
            },
        ];
        let right_rows = vec![
            vec![Some("001".into()), Some("13800138000".into())],
            vec![Some("002".into()), Some("13900139000".into())],
        ];
        let right = store::create_dataset(
            &mut conn,
            store::NewDataset {
                name: "考生档案",
                kind: "import",
                source_file: "",
                origin: "单元测试",
                project_id: "",
                columns: &right_cols,
                rows: &right_rows,
            },
        )
        .expect("right dataset");
        let result = run_steps(
            &mut conn,
            &left_id,
            "f_join",
            json!([
                { "id": "s1", "operator": "join_dataset",
                  "params": { "dataset_id": right.id.clone(), "left_field": "学号", "right_field": "学号",
                              "fields": ["手机号"], "join_type": "left" } },
                { "id": "s2", "operator": "output", "params": { "name": "关联结果" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        assert_eq!(page.columns.len(), 3);
        assert_eq!(page.columns[2].name, "手机号");
        assert_eq!(page.rows[0][2], json!("13800138000"));
        assert_eq!(page.rows[1][2], json!("13900139000"));

        let validate_steps = steps(json!([
            { "id": "s1", "operator": "join_dataset",
              "params": { "dataset_id": right.id.clone(), "left_field": "学号", "right_field": "学号",
                          "fields": ["手机号"] } }
        ]));
        let mut map: HashMap<String, Vec<DatasetColumn>> = HashMap::new();
        map.insert(
            right.id.clone(),
            store::get_columns(&conn, &right.id).expect("right cols"),
        );
        let validated = validate_steps_with(
            &validate_steps,
            &["学号".to_string(), "姓名".to_string()],
            &map,
        );
        assert!(validated.ok, "{:?}", validated.errors);
        assert_eq!(
            validated.steps[0].output_fields,
            vec!["学号", "姓名", "手机号"]
        );
    }

    #[test]
    fn depreciation_and_allocation_compute_finance_values() {
        let (mut conn, ds_id) = setup_custom(
            &[
                ("资产名称", "text"),
                ("原值", "number"),
                ("已计提月数", "number"),
                ("金额", "number"),
                ("权重", "number"),
            ],
            &[
                vec!["电脑", "12000", "12", "1000", "1"],
                vec!["打印机", "6000", "0", "1000", "3"],
            ],
        );
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_dep",
            json!([
                { "id": "s1", "operator": "depreciation",
                  "params": { "original": "原值", "salvage_rate": 5, "years": 5, "months": "已计提月数",
                              "monthly_out": "月折旧额", "accumulated_out": "累计折旧", "net_out": "净值" } },
                { "id": "s2", "operator": "allocate",
                  "params": { "amount": "金额", "weight": "权重", "out": "分摊金额", "decimals": 2 } },
                { "id": "s3", "operator": "output", "params": { "name": "财务计算" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        assert_eq!(page.rows[0][5], json!(190.0));
        assert_eq!(page.rows[0][6], json!(2280.0));
        assert_eq!(page.rows[0][7], json!(9720.0));
        assert_eq!(page.rows[1][6], json!(0.0));
        assert_eq!(page.rows[0][8], json!(250.0));
        assert_eq!(page.rows[1][8], json!(750.0));
    }

    #[test]
    fn balance_check_fails_on_unbalanced_vouchers() {
        let (mut conn, ds_id) = setup_custom(
            &[("凭证号", "text"), ("借方", "number"), ("贷方", "number")],
            &[vec!["P1", "100", "90"]],
        );
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_balance_fail",
            json!([{ "id": "s1", "operator": "balance_check",
                     "params": { "voucher": "凭证号", "debit": "借方", "credit": "贷方", "on_error": "fail" } }]),
        );
        assert_eq!(result.status, "failed");
        assert!(result.message.unwrap_or_default().contains("借贷不平衡"));
    }

    #[test]
    fn data_profile_summarizes_each_column() {
        let (mut conn, ds_id) = setup();
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_profile",
            json!([
                { "id": "s1", "operator": "data_profile", "params": { "columns": [] } },
                { "id": "s2", "operator": "output", "params": { "name": "数据概览" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        assert_eq!(page.rows.len(), 3);
        assert_eq!(page.columns[0].name, "字段名");
        assert_eq!(page.rows[0][0], json!("姓名"));
        assert_eq!(page.rows[0][1], json!(3.0));
        assert_eq!(page.rows[0][2], json!(0.0));
        assert_eq!(page.rows[1][0], json!("笔试成绩"));
        assert_eq!(page.rows[1][6], json!(80.0));
    }

    #[test]
    fn score_summary_computes_rates_and_stddev() {
        let (mut conn, ds_id) = setup_custom(
            &[("班级", "text"), ("成绩", "number")],
            &[
                vec!["一班", "90"],
                vec!["一班", "80"],
                vec!["一班", "50"],
                vec!["二班", "70"],
            ],
        );
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_summary",
            json!([
                { "id": "s1", "operator": "score_summary",
                  "params": { "column": "成绩", "group_by": ["班级"], "pass_score": 60,
                              "excellent_score": 85, "decimals": 2 } },
                { "id": "s2", "operator": "output", "params": { "name": "成绩分析" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.columns[0].name, "班级");
        assert_eq!(page.columns[1].name, "人数");
        assert_eq!(page.rows[0][0], json!("一班"));
        assert_eq!(page.rows[0][1], json!(3.0));
        assert_eq!(page.rows[0][2], json!(73.33));
        assert_eq!(page.rows[0][4], json!(50.0));
        assert_eq!(page.rows[0][5], json!(66.67));
        assert_eq!(page.rows[0][6], json!(33.33));
        assert_eq!(page.rows[0][7], json!(17.0));
    }

    #[test]
    fn score_segment_produces_one_point_table_with_cumulative_count() {
        let (mut conn, ds_id) = setup_custom(
            &[("姓名", "text"), ("成绩", "number"), ("班级", "text")],
            &[
                vec!["张三", "85", "一班"],
                vec!["李四", "92", "一班"],
                vec!["王五", "78", "二班"],
                vec!["赵六", "88", "一班"],
            ],
        );
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_segment",
            json!([
                { "id": "s1", "operator": "score_segment",
                  "params": { "column": "成绩", "step": 10, "partition_by": ["班级"], "cumulative": true,
                              "segment_out": "分数段", "count_out": "人数" } },
                { "id": "s2", "operator": "output", "params": { "name": "分数段统计" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        assert_eq!(page.rows.len(), 3);
        assert_eq!(page.columns[0].name, "班级");
        assert_eq!(page.columns[1].name, "分数段");
        assert_eq!(page.columns[3].name, "累计人数");
        let one: Vec<&Vec<Value>> = page.rows.iter().filter(|r| r[0] == json!("一班")).collect();
        assert_eq!(one.len(), 2);
        assert_eq!(one[0][1], json!("80-89分"));
        assert_eq!(one[0][2], json!(2.0));
        assert_eq!(one[0][3], json!(2.0));
        assert_eq!(one[1][1], json!("90-99分"));
        assert_eq!(one[1][3], json!(3.0));
    }

    #[test]
    fn assign_seats_fills_rooms_and_seats_in_order() {
        let (mut conn, ds_id) = setup_custom(
            &[("姓名", "text"), ("准考证号", "text")],
            &[
                vec!["张三", "001"],
                vec!["李四", "002"],
                vec!["王五", "003"],
                vec!["赵六", "004"],
                vec!["钱七", "005"],
            ],
        );
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_seats",
            json!([
                { "id": "s1", "operator": "assign_seats",
                  "params": { "order_by": "准考证号", "per_room": 2, "start_room": 3,
                              "room_out": "考场号", "seat_out": "座位号" } },
                { "id": "s2", "operator": "output", "params": { "name": "考场编排" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        assert_eq!(page.rows.len(), 5);
        let rooms: Vec<f64> = page.rows.iter().map(|r| r[2].as_f64().unwrap()).collect();
        let seats: Vec<f64> = page.rows.iter().map(|r| r[3].as_f64().unwrap()).collect();
        assert_eq!(rooms, vec![3.0, 3.0, 4.0, 4.0, 5.0]);
        assert_eq!(seats, vec![1.0, 2.0, 1.0, 2.0, 1.0]);
    }

    #[test]
    fn format_and_not_null_checks_detect_bad_rows() {
        let (mut conn, ds_id) = setup_custom(
            &[("姓名", "text"), ("手机号", "text")],
            &[
                vec!["张三", "13800138000"],
                vec!["李四", "12345"],
                vec!["王五", ""],
            ],
        );
        // 格式错误只警告
        let ok = run_steps(
            &mut conn,
            &ds_id,
            "f_format_warn",
            json!([
                { "id": "s1", "operator": "format_check",
                  "params": { "column": "手机号", "mode": "mobile", "on_error": "warn" } },
                { "id": "s2", "operator": "not_null_check",
                  "params": { "columns": ["姓名"], "on_error": "fail" } }
            ]),
        );
        assert_eq!(ok.status, "success", "{:?}", ok.message);

        // 手机号格式错误时终止
        let failed = run_steps(
            &mut conn,
            &ds_id,
            "f_format_fail",
            json!([{ "id": "s1", "operator": "format_check",
                     "params": { "column": "手机号", "mode": "mobile", "on_error": "fail" } }]),
        );
        assert_eq!(failed.status, "failed");
        assert!(failed.message.unwrap_or_default().contains("手机号"));
    }

    #[test]
    fn column_compare_checks_two_columns() {
        let (mut conn, ds_id) = setup_custom(
            &[("姓名", "text"), ("姓名2", "text")],
            &[vec!["张三", "张三"], vec!["李四", "李伍"]],
        );
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_compare",
            json!([{ "id": "s1", "operator": "column_compare",
                     "params": { "left": "姓名", "right": "姓名2", "on_error": "fail" } }]),
        );
        assert_eq!(result.status, "failed");
        assert!(result.message.unwrap_or_default().contains("不一致"));
    }

    #[test]
    fn range_check_rejects_out_of_range_scores() {
        let (mut conn, ds_id) = setup();
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_range",
            json!([{ "id": "s1", "operator": "range_check",
                     "params": { "column": "笔试成绩", "min": 0, "max": 100, "on_error": "fail" } }]),
        );
        assert_eq!(result.status, "success");

        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_range_bad",
            json!([{ "id": "s1", "operator": "range_check",
                     "params": { "column": "笔试成绩", "min": 0, "max": 75, "on_error": "fail" } }]),
        );
        assert_eq!(result.status, "failed");
        assert!(result.message.unwrap_or_default().contains("超出"));
    }

    #[test]
    fn tax_calc_splits_exclusive_and_inclusive_amounts() {
        let (mut conn, ds_id) = setup_custom(&[("金额", "number")], &[vec!["100"], vec!["113"]]);
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_tax_exclusive",
            json!([
                { "id": "s1", "operator": "tax_calc",
                  "params": { "column": "金额", "rate": 0.13, "mode": "exclusive" } },
                { "id": "s2", "operator": "output", "params": { "name": "价外税" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        // 金额、不含税金额、税额、含税金额
        assert_eq!(page.rows[0][1], json!(100.0));
        assert_eq!(page.rows[0][2], json!(13.0));
        assert_eq!(page.rows[0][3], json!(113.0));

        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_tax_inclusive",
            json!([
                { "id": "s1", "operator": "tax_calc",
                  "params": { "column": "金额", "rate": 0.13, "mode": "inclusive" } },
                { "id": "s2", "operator": "output", "params": { "name": "价内税" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        // 113 含税 → 100 不含税、13 税额、113 含税
        assert_eq!(page.rows[1][1], json!(100.0));
        assert_eq!(page.rows[1][2], json!(13.0));
        assert_eq!(page.rows[1][3], json!(113.0));
    }

    #[test]
    fn budget_variance_reports_execution_rate_and_status() {
        let (mut conn, ds_id) = setup_custom(
            &[("部门", "text"), ("预算", "number"), ("实际", "number")],
            &[vec!["研发", "1000", "1100"], vec!["市场", "500", "300"]],
        );
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_budget",
            json!([
                { "id": "s1", "operator": "budget_variance",
                  "params": { "budget_column": "预算", "actual_column": "实际",
                              "group_by": ["部门"], "warning_rate": 100 } },
                { "id": "s2", "operator": "output", "params": { "name": "预算执行" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        // 部门、预算数、实际数、执行率、差额、预算状态
        let row = page
            .rows
            .iter()
            .find(|r| r[0] == json!("研发"))
            .expect("研发行");
        assert_eq!(row[1], json!(1000.0));
        assert_eq!(row[2], json!(1100.0));
        assert_eq!(row[3], json!(110.0));
        assert_eq!(row[4], json!(100.0));
        assert_eq!(row[5], json!("超支"));

        let market = page
            .rows
            .iter()
            .find(|r| r[0] == json!("市场"))
            .expect("市场行");
        assert_eq!(market[3], json!(60.0));
        assert_eq!(market[4], json!(-200.0));
        assert_eq!(market[5], json!("正常"));
    }

    #[test]
    fn validate_steps_checks_tax_and_budget_params() {
        let columns = vec!["金额".to_string(), "预算".to_string(), "实际".to_string()];
        let result = validate_steps(
            &steps(json!([
                { "id": "s1", "operator": "tax_calc",
                  "params": { "column": "金额", "rate": 1.5 } },
                { "id": "s2", "operator": "budget_variance",
                  "params": { "budget_column": "预算", "actual_column": "预算" } }
            ])),
            &columns,
        );
        let messages: Vec<String> = result.steps.iter().flat_map(|s| s.errors.clone()).collect();
        assert!(messages.iter().any(|m| m.contains("税率")));
        assert!(messages.iter().any(|m| m.contains("同一字段")));
    }

    #[test]
    fn shuffle_interleaves_groups_to_avoid_adjacent_same_school() {
        let (mut conn, ds_id) = setup_custom(
            &[("姓名", "text"), ("学校", "text")],
            &[
                vec!["甲", "A校"],
                vec!["乙", "A校"],
                vec!["丙", "B校"],
                vec!["丁", "B校"],
                vec!["戊", "C校"],
                vec!["己", "C校"],
            ],
        );
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_shuffle",
            json!([
                { "id": "s1", "operator": "shuffle", "params": { "group_by": ["学校"] } },
                { "id": "s2", "operator": "output", "params": { "name": "打散结果" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        assert_eq!(page.rows.len(), 6);
        // 前三行应分别来自三所学校（各组第 1 轮）
        let first_three: Vec<&str> = page.rows[..3]
            .iter()
            .map(|r| r[1].as_str().unwrap())
            .collect();
        assert_eq!(first_three, vec!["A校", "B校", "C校"]);
        // 随机序号为 1、1、1、2、2、2
        let seqs: Vec<f64> = page.rows.iter().map(|r| r[2].as_f64().unwrap()).collect();
        assert_eq!(seqs, vec![1.0, 1.0, 1.0, 2.0, 2.0, 2.0]);
    }

    #[test]
    fn reconcile_marks_matched_mismatched_and_one_sided_entries() {
        let (mut conn, left_id) = setup_custom(
            &[("单号", "text"), ("金额", "number")],
            &[vec!["A", "100"], vec!["B", "50"], vec!["C", "20"]],
        );
        let right_cols = vec![
            DatasetColumn {
                name: "单号".into(),
                dtype: "text".into(),
            },
            DatasetColumn {
                name: "金额".into(),
                dtype: "number".into(),
            },
        ];
        let right_rows = vec![
            vec![Some("A".to_string()), Some("100".to_string())],
            vec![Some("B".to_string()), Some("40".to_string())],
            vec![Some("D".to_string()), Some("10".to_string())],
        ];
        let right = store::create_dataset(
            &mut conn,
            store::NewDataset {
                name: "银行流水",
                kind: "import",
                source_file: "",
                origin: "单元测试",
                project_id: "",
                columns: &right_cols,
                rows: &right_rows,
            },
        )
        .expect("right dataset");

        let result = run_steps(
            &mut conn,
            &left_id,
            "f_reconcile_all",
            json!([
                { "id": "s1", "operator": "reconcile",
                  "params": { "dataset_id": right.id, "left_key": "单号", "left_amount": "金额",
                              "right_key": "单号", "right_amount": "金额", "mode": "all" } },
                { "id": "s2", "operator": "output", "params": { "name": "对账结果" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        assert_eq!(page.rows.len(), 4);
        let status_of = |key: &str| {
            page.rows
                .iter()
                .find(|r| r[0] == json!(key))
                .map(|r| r[4].as_str().unwrap().to_string())
                .expect("key row")
        };
        assert_eq!(status_of("A"), "一致");
        assert_eq!(status_of("B"), "金额不符");
        assert_eq!(status_of("C"), "本方独有");
        assert_eq!(status_of("D"), "对方独有");
        let b = page.rows.iter().find(|r| r[0] == json!("B")).expect("B 行");
        assert_eq!(b[1], json!(50.0));
        assert_eq!(b[2], json!(40.0));
        assert_eq!(b[3], json!(10.0));

        // 仅差异模式应过滤掉一致记录
        let result = run_steps(
            &mut conn,
            &left_id,
            "f_reconcile_diff",
            json!([
                { "id": "s1", "operator": "reconcile",
                  "params": { "dataset_id": right.id, "left_key": "单号", "left_amount": "金额",
                              "right_key": "单号", "right_amount": "金额", "mode": "diff" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        assert_eq!(page.rows.len(), 3);
        assert!(page.rows.iter().all(|r| r[4] != json!("一致")));
    }

    #[test]
    fn sum_check_validates_total_against_detail_columns() {
        let (mut conn, ds_id) = setup_custom(
            &[("语文", "number"), ("数学", "number"), ("总分", "number")],
            &[vec!["80", "90", "170"], vec!["70", "90", "150"]],
        );
        let ok = run_steps(
            &mut conn,
            &ds_id,
            "f_sum_warn",
            json!([{ "id": "s1", "operator": "sum_check",
                     "params": { "total": "总分", "parts": ["语文", "数学"], "on_error": "warn" } }]),
        );
        assert_eq!(ok.status, "success", "{:?}", ok.message);

        let failed = run_steps(
            &mut conn,
            &ds_id,
            "f_sum_fail",
            json!([{ "id": "s1", "operator": "sum_check",
                     "params": { "total": "总分", "parts": ["语文", "数学"], "on_error": "fail" } }]),
        );
        assert_eq!(failed.status, "failed");
        assert!(failed.message.unwrap_or_default().contains("合计"));
    }

    #[test]
    fn loan_payment_computes_equal_installment_amounts() {
        let (mut conn, ds_id) = setup_custom(&[("本金", "number")], &[vec!["120000"]]);
        let result = run_steps(
            &mut conn,
            &ds_id,
            "f_loan",
            json!([
                { "id": "s1", "operator": "loan_payment",
                  "params": { "principal": "本金", "annual_rate": 0.06, "years": 10 } },
                { "id": "s2", "operator": "output", "params": { "name": "还款测算" } }
            ]),
        );
        assert_eq!(result.status, "success", "{:?}", result.message);
        let page = store::read_rows(&conn, result.output_dataset_id.as_deref().unwrap(), 0, 10)
            .expect("read");
        let expected = 120000.0 * 0.005 * 1.005f64.powi(120) / (1.005f64.powi(120) - 1.0);
        let payment = page.rows[0][1].as_f64().unwrap();
        assert!((payment - expected).abs() < 0.02, "{payment} vs {expected}");
        let total = page.rows[0][3].as_f64().unwrap();
        assert!((total - payment * 120.0).abs() < 0.05);
        let interest = page.rows[0][2].as_f64().unwrap();
        assert!((interest - (total - 120000.0)).abs() < 0.05);
    }
}
