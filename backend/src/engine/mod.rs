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

use crate::db::{new_id, now};
use crate::error::{AppError, AppResult};
use crate::models::{
    DatasetColumn, FlowStep, RunLogEntry, RunResult, ValidateError, ValidateResult,
};
use crate::store;
use rusqlite::Connection;
use serde_json::Value;

const PREVIEW_ROWS: i64 = 200;

struct Compiler<'a> {
    conn: &'a Connection,
    dataset_id: String,
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

impl<'a> Compiler<'a> {
    fn new(conn: &'a Connection, dataset_id: &str) -> AppResult<Self> {
        let columns = store::get_columns(conn, dataset_id)?;
        if columns.is_empty() {
            return Err(AppError::bad_request("数据集没有任何字段"));
        }
        let tname = crate::db::table_name(dataset_id);
        let items: Vec<String> = columns
            .iter()
            .enumerate()
            .map(|(i, c)| format!("\"c{i}\" AS {}", q(&c.name)))
            .collect();
        let cte = format!("s0 AS (SELECT {} FROM \"{}\")", items.join(", "), tname);
        Ok(Compiler {
            conn,
            dataset_id: dataset_id.to_string(),
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

    fn count_current(&self) -> i64 {
        let sql = self.with_ctes(&format!("SELECT COUNT(*) FROM {}", self.cur));
        self.conn.query_row(&sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0)
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

    fn col_list(&self) -> String {
        self.columns
            .iter()
            .map(|c| q(&c.name))
            .collect::<Vec<_>>()
            .join(", ")
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
                expr::preview_columns(&self.columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>())
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
            self.count_current(),
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
            self.count_current(),
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
                let from = item.get("from").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                let to = item.get("to").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
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
                self.count_current(),
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
                renames.iter().map(|(_, to)| to.as_str()).collect::<Vec<_>>().join("、")
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
            self.count_current(),
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
                let new = if c.name == from { to.clone() } else { c.name.clone() };
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
            self.count_current(),
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
        let items: Vec<String> = cols.iter().map(|c| format!("{} AS {}", q(c), q(c))).collect();
        let body = format!("SELECT {} FROM {}", items.join(", "), self.cur);
        let cte = self.push_cte(body.clone());
        self.columns.retain(|c| cols.contains(&c.name));
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current(),
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
            "text" => format!("TEXT({})", q(&col)),
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
                "SELECT COUNT(*) FROM {} WHERE TEXT({}) IS NOT NULL AND TRIM(TEXT({})) <> '' AND NOT ({guard})",
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
            c.dtype = if to == "text" { "text".to_string() } else { "number".to_string() };
        }
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current(),
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
            "CASE WHEN {} IS NULL OR TRIM(TEXT({})) = '' THEN '{}' ELSE {} END",
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
            self.count_current(),
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
        self.add_column(DatasetColumn { name: out.clone(), dtype: "number".to_string() });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current(),
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
            if o.is_empty() { "排名".to_string() } else { o }
        };
        if self.has_column(&out) {
            return Err(format!("字段 [{out}] 已存在"));
        }
        let method = match str_param(p, "method").as_str() {
            "dense_rank" => "DENSE_RANK()",
            "row_number" => "ROW_NUMBER()",
            _ => "RANK()",
        };
        let order = if str_param(p, "order") == "asc" { "ASC" } else { "DESC" };
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
        self.add_column(DatasetColumn { name: out.clone(), dtype: "number".to_string() });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current(),
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
            self.count_current(),
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
        let order = if str_param(p, "order") == "asc" { "ASC" } else { "DESC" };
        let window_order = if order_by.is_empty() {
            String::new()
        } else {
            format!("ORDER BY {} {order} ", q(&order_by))
        };
        let inner = self.project(
            &self.cur.clone(),
            &[],
            &[("__rn".to_string(), format!(
                "ROW_NUMBER() OVER (PARTITION BY {} {} )",
                cols.iter().map(|c| q(c)).collect::<Vec<_>>().join(", "),
                window_order.trim()
            ))],
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
            self.count_current(),
            Some(cte),
            Some(format!("按 {} 去重", cols.join("、"))),
        ))
    }

    fn op_sort(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let by = str_param(p, "by");
        if !self.has_column(&by) {
            return Err(format!("字段 [{by}] 不存在"));
        }
        let order = if str_param(p, "order") == "asc" { "ASC" } else { "DESC" };
        self.order_by = Some(format!("{} {order}", q(&by)));
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current(),
            None,
            Some(format!("结果按 [{by}] {order} 排序")),
        ))
    }

    fn op_limit(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let n = num_param(p, "n").unwrap_or(100);
        self.limit = Some(n.max(0));
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current(),
            None,
            Some(format!("只输出前 {n} 行")),
        ))
    }

    fn op_output(&mut self, i: i64, step: &FlowStep, p: &Value) -> Result<RunLogEntry, String> {
        let name = str_param(p, "name");
        self.output_name = Some(if name.is_empty() { "流程输出".to_string() } else { name });
        Ok(self.log_entry(
            i,
            step,
            "ok",
            self.count_current(),
            None,
            Some(format!("输出结果表：{}", self.output_name.clone().unwrap())),
        ))
    }
}

/// 运行流程：编译 → 执行 → 落地为新的数据集。
pub fn run_flow(conn: &Connection, flow_id: &str, dataset_id: &str) -> AppResult<RunResult> {
    let (flow_name, steps) = load_flow(conn, flow_id)?;
    if steps.is_empty() {
        return Err(AppError::bad_request("流程没有任何步骤"));
    }
    // 数据集必须存在
    let src_ds = store::get_dataset(conn, dataset_id)?;

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
                failed = Some(format!("第 {} 步「{}」失败：{msg}", i + 1, steps[idx].label));
                break;
            }
        }
    }

    let run_id = new_id();
    let full_sql = c.final_sql();
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
            log: c.log,
            output_dataset_id: None,
            output: None,
        });
    }

    let out_name = c
        .output_name
        .clone()
        .unwrap_or_else(|| format!("{flow_name} 结果"));
    let out_ds = store::create_dataset_from_select(
        conn,
        &out_name,
        "flow_output",
        &format!("流程「{flow_name}」"),
        &src_ds.project_id,
        &c.columns,
        &full_sql,
    )?;
    let output = store::read_rows(conn, &out_ds.id, 0, PREVIEW_ROWS)?;

    conn.execute(
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

    Ok(RunResult {
        run_id,
        status: "success".to_string(),
        message: Some(format!(
            "流程执行成功，输出 {} 行 / {} 列",
            out_ds.row_count,
            out_ds.columns.len()
        )),
        log: c.log,
        output_dataset_id: Some(out_ds.id),
        output: Some(output),
    })
}

fn load_flow(conn: &Connection, flow_id: &str) -> AppResult<(String, Vec<FlowStep>)> {
    let mut stmt = conn.prepare("SELECT name, steps FROM flows WHERE id = ?1")?;
    let mut rows = stmt.query(rusqlite::params![flow_id])?;
    let row = rows
        .next()?
        .ok_or_else(|| AppError::not_found(format!("流程不存在: {flow_id}")))?;
    let name: String = row.get(0)?;
    let raw: String = row.get(1)?;
    let steps: Vec<FlowStep> = serde_json::from_str(&raw).unwrap_or_default();
    Ok((name, steps))
}

/// 静态校验：不执行 SQL，只根据字段名推演每一步是否可行。
pub fn validate_steps(steps: &[FlowStep], columns: &[String]) -> ValidateResult {
    let mut cols: Vec<String> = columns.to_vec();
    let mut errors: Vec<ValidateError> = Vec::new();
    let q = |j: i64, m: String| ValidateError { step_index: j, message: m };

    for (idx, step) in steps.iter().enumerate() {
        let i = idx as i64;
        if !step.enabled {
            continue;
        }
        let p = if step.params.is_null() { Value::Object(Default::default()) } else { step.params.clone() };
        match step.operator.as_str() {
            "require_fields" => {
                let fields = list_param(&p, "fields");
                if fields.is_empty() {
                    errors.push(q(i, "必需字段列表为空".into()));
                }
                let missing: Vec<String> =
                    fields.iter().filter(|f| !cols.contains(f)).cloned().collect();
                if !missing.is_empty() {
                    errors.push(q(i, format!("缺少必要字段：{}", missing.join("、"))));
                }
            }
            "unique_check" | "dedupe" => {
                for c in list_param(&p, "columns") {
                    if !cols.contains(&c) {
                        errors.push(q(i, format!("字段 [{c}] 不存在")));
                    }
                }
            }
            "map_columns" => { /* 依赖全局别名表，运行时再校验 */ }
            "rename" => {
                let from = str_param(&p, "from");
                let to = str_param(&p, "to");
                if !cols.contains(&from) {
                    errors.push(q(i, format!("字段 [{from}] 不存在")));
                }
                if cols.contains(&to) && to != from {
                    errors.push(q(i, format!("字段 [{to}] 已存在")));
                }
                if let Some(pos) = cols.iter().position(|c| c == &from) {
                    cols[pos] = to;
                }
            }
            "select" => {
                let keep = list_param(&p, "columns");
                for c in &keep {
                    if !cols.contains(c) {
                        errors.push(q(i, format!("字段 [{c}] 不存在")));
                    }
                }
                if !keep.is_empty() {
                    cols = keep;
                }
            }
            "cast" => {
                let c = str_param(&p, "column");
                if !cols.contains(&c) {
                    errors.push(q(i, format!("字段 [{c}] 不存在")));
                }
            }
            "fill_null" => {
                let c = str_param(&p, "column");
                if !cols.contains(&c) {
                    errors.push(q(i, format!("字段 [{c}] 不存在")));
                }
            }
            "compute" => {
                let out = str_param(&p, "out");
                let raw = str_param(&p, "expr");
                if cols.contains(&out) {
                    errors.push(q(i, format!("字段 [{out}] 已存在")));
                }
                if let Err(e) = expr::compile(&raw, &cols) {
                    errors.push(q(i, e));
                } else if !out.is_empty() {
                    cols.push(out);
                }
            }
            "rank" => {
                let by = str_param(&p, "by");
                if !cols.contains(&by) {
                    errors.push(q(i, format!("字段 [{by}] 不存在")));
                }
                let out = {
                    let o = str_param(&p, "out");
                    if o.is_empty() { "排名".to_string() } else { o }
                };
                if cols.contains(&out) {
                    errors.push(q(i, format!("字段 [{out}] 已存在")));
                } else {
                    cols.push(out);
                }
                for c in list_param(&p, "partition_by") {
                    if !cols.contains(&c) {
                        errors.push(q(i, format!("分组字段 [{c}] 不存在")));
                    }
                }
            }
            "filter" => {
                let raw = str_param(&p, "expr");
                if raw.is_empty() {
                    errors.push(q(i, "筛选条件不能为空".into()));
                } else if let Err(e) = expr::compile(&raw, &cols) {
                    errors.push(q(i, e));
                }
            }
            "sort" => {
                let by = str_param(&p, "by");
                if !cols.contains(&by) {
                    errors.push(q(i, format!("字段 [{by}] 不存在")));
                }
            }
            "limit" => {}
            "output" => {}
            other => errors.push(q(i, format!("不支持的操作符: {other}"))),
        }
    }
    ValidateResult { ok: errors.is_empty(), errors }
}
