//! 原子操作符目录。前端「处理流程」编辑器直接根据这份目录渲染参数表单。

use crate::models::{OperatorDef, OperatorParam};
use serde_json::{json, Value};

fn p(key: &str, label: &str, ptype: &str, required: bool, default: Value) -> OperatorParam {
    OperatorParam {
        key: key.to_string(),
        label: label.to_string(),
        ptype: ptype.to_string(),
        required,
        default,
        options: None,
        placeholder: None,
        help: None,
    }
}

fn with_options(mut p: OperatorParam, opts: &[&str]) -> OperatorParam {
    p.options = Some(opts.iter().map(|s| s.to_string()).collect());
    p
}

fn with_help(mut p: OperatorParam, help: &str) -> OperatorParam {
    p.help = Some(help.to_string());
    p
}

fn with_placeholder(mut p: OperatorParam, ph: &str) -> OperatorParam {
    p.placeholder = Some(ph.to_string());
    p
}

fn def(
    otype: &str,
    label: &str,
    group: &str,
    description: &str,
    params: Vec<OperatorParam>,
) -> OperatorDef {
    OperatorDef {
        otype: otype.to_string(),
        label: label.to_string(),
        group: group.to_string(),
        description: description.to_string(),
        params,
    }
}

pub fn catalog() -> Vec<OperatorDef> {
    vec![
        def(
            "require_fields",
            "必需字段校验",
            "校验",
            "检查当前数据是否包含流程要求的字段，缺失则终止流程。用于保证每次导入的数据都符合标准。",
            vec![
                with_help(
                    p("fields", "必需字段", "fields", true, json!([])),
                    "从标准字段字典中选择，也可以用 [字段名] 形式手写",
                ),
                with_help(p("allow_extra", "允许存在其它字段", "bool", false, json!(true)), "关闭后数据中不能出现未声明字段"),
            ],
        ),
        def(
            "unique_check",
            "唯一性校验",
            "校验",
            "检查指定字段组合是否重复（例如身份证号不允许重复）。",
            vec![
                p("columns", "参与校验的字段", "columns", true, json!([])),
                with_options(p("on_error", "发现重复时", "enum", false, json!("fail")), &["fail", "warn"]),
            ],
        ),
        def(
            "map_columns",
            "字段标准化映射",
            "字段",
            "按别名映射表把原始表头改名成标准字段（如 szf → 身份证号）。",
            vec![
                with_help(p("apply_global_alias", "使用全局别名表", "bool", false, json!(true)), "关闭后只使用下面的手工映射"),
                with_help(p("mappings", "手工映射", "mappings", false, json!([])), "原始列 → 标准字段。手工映射优先级高于全局别名表"),
            ],
        ),
        def(
            "rename",
            "重命名字段",
            "字段",
            "把某一列改成新的名字。",
            vec![
                p("from", "原字段", "column", true, json!("")),
                p("to", "新字段名", "text", true, json!("")),
            ],
        ),
        def(
            "select",
            "选择/裁减字段",
            "字段",
            "只保留需要的字段，并按给定顺序输出。",
            vec![p("columns", "保留字段", "columns", true, json!([]))],
        ),
        def(
            "cast",
            "字段类型转换",
            "字段",
            "把文本列转换成数值/整数等类型，并可选保留几位小数。",
            vec![
                p("column", "字段", "column", true, json!("")),
                with_options(p("to", "目标类型", "enum", true, json!("number")), &["number", "integer", "text"]),
                with_help(p("decimals", "保留小数位", "number", false, json!(2)), "-1 表示不取整"),
                with_options(p("on_error", "遇到非法值时", "enum", false, json!("null")), &["null", "fail"]),
            ],
        ),
        def(
            "fill_null",
            "空值填充",
            "字段",
            "把空值（NULL 或空字符串）替换成指定内容，例如“缺考”。",
            vec![
                p("column", "字段", "column", true, json!("")),
                p("value", "填充值", "text", true, json!("")),
            ],
        ),
        def(
            "compute",
            "计算新字段",
            "计算",
            "用公式新建一列。字段用中括号引用，例如 [笔试成绩]*0.6 + [面试成绩]*0.4。",
            vec![
                p("out", "新字段名", "text", true, json!("")),
                with_placeholder(p("expr", "计算公式", "expr", true, json!("")), "[笔试成绩]*0.6 + [面试成绩]*0.4"),
                with_help(p("decimals", "保留小数位", "number", false, json!(2)), "-1 表示不取整"),
            ],
        ),
        def(
            "rank",
            "排名",
            "计算",
            "按某字段排名，可指定分组字段（例如按考场分别排名）。",
            vec![
                p("by", "排序依据字段", "column", true, json!("")),
                with_options(p("order", "排序方向", "enum", false, json!("desc")), &["desc", "asc"]),
                p("out", "排名字段名", "text", false, json!("排名")),
                with_options(p("method", "并列名次处理", "enum", false, json!("rank")), &["rank", "dense_rank", "row_number"]),
                p("partition_by", "分组字段（可空）", "columns", false, json!([])),
            ],
        ),
        def(
            "filter",
            "筛选行",
            "行",
            "按条件保留满足条件的行，例如 [笔试成绩]>=60。",
            vec![with_placeholder(p("expr", "筛选条件", "expr", true, json!("")), "[总成绩] >= 60")],
        ),
        def(
            "dedupe",
            "去重",
            "行",
            "按字段组合去重，可选按某字段排序后保留第一条（例如保留分数最高的一条）。",
            vec![
                p("columns", "去重依据字段", "columns", true, json!([])),
                with_help(p("order_by", "保留顺序依据（可空）", "column", false, json!("")), "留空表示保留原始顺序中的第一条"),
                with_options(p("order", "顺序", "enum", false, json!("desc")), &["desc", "asc"]),
            ],
        ),
        def(
            "sort",
            "排序",
            "行",
            "按某字段排序输出结果。",
            vec![
                p("by", "排序字段", "column", true, json!("")),
                with_options(p("order", "方向", "enum", false, json!("desc")), &["desc", "asc"]),
            ],
        ),
        def(
            "limit",
            "取前 N 行",
            "行",
            "只输出前 N 行，常用于预览或抽取名单。",
            vec![p("n", "行数", "number", true, json!(100))],
        ),
        def(
            "output",
            "输出结果表",
            "输出",
            "标记流程输出，并为结果数据集命名。",
            vec![p("name", "输出数据集名称", "text", false, json!("流程输出"))],
        ),
    ]
}

pub fn find(otype: &str) -> Option<OperatorDef> {
    catalog().into_iter().find(|o| o.otype == otype)
}

pub fn label_of(otype: &str) -> String {
    find(otype)
        .map(|o| o.label)
        .unwrap_or_else(|| otype.to_string())
}

pub fn default_params(otype: &str) -> Value {
    let mut obj = serde_json::Map::new();
    if let Some(d) = find(otype) {
        for p in d.params {
            obj.insert(p.key.clone(), p.default);
        }
    }
    Value::Object(obj)
}
