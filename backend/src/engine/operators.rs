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
            "按一个或多个字段排序输出结果；多字段时从上到下依次比较（先按第一项，再按第二项…）。",
            vec![
                with_help(
                    p("keys", "排序字段（可多级）", "sorts", false, json!([])),
                    "按从上到下的顺序比较，例如先按考场升序、再按总成绩降序",
                ),
                p("by", "单字段排序依据（兼容旧版）", "column", false, json!("")),
                with_options(p("order", "单字段方向", "enum", false, json!("desc")), &["desc", "asc"]),
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
            "random_number",
            "随机数",
            "计算",
            "为每一行生成一个随机数，可指定整数/小数与取值范围。常用于抽签、随机考场分配。",
            vec![
                p("out", "新字段名", "text", true, json!("随机数")),
                with_options(p("mode", "类型", "enum", false, json!("integer")), &["integer", "decimal"]),
                p("min", "最小值", "number", false, json!(1)),
                p("max", "最大值", "number", false, json!(100)),
                with_help(p("decimals", "保留小数位", "number", false, json!(2)), "仅小数类型有效"),
            ],
        ),
        def(
            "random_rows",
            "随机行",
            "行",
            "把数据随机打乱，或随机抽取 N 行。抽签、随机分组、抽查名单都会用到。",
            vec![
                with_options(p("mode", "方式", "enum", false, json!("shuffle")), &["shuffle", "sample"]),
                with_help(p("n", "抽取行数", "number", false, json!(10)), "仅在「随机抽取」时生效"),
            ],
        ),
        def(
            "group_aggregate",
            "分组汇总",
            "计算",
            "按字段分组后计算合计、平均、计数、最大最小、标准差等。留空分组字段时对全表汇总。",
            vec![
                with_help(p("group_by", "分组字段", "columns", false, json!([])), "例如按考场分组统计人数"),
                with_help(
                    p("aggregations", "汇总项", "aggregations", false, json!([])),
                    "每个汇总项选择字段、统计方式和输出列名；计数可不选字段",
                ),
            ],
        ),
        def(
            "round_number",
            "数值取整",
            "计算",
            "对数值做四舍五入、向下取整或向上取整，可原地覆盖或输出到新字段。",
            vec![
                p("column", "字段", "column", true, json!("")),
                with_options(p("mode", "方式", "enum", false, json!("round")), &["round", "floor", "ceil"]),
                with_help(p("decimals", "保留小数位", "number", false, json!(2)), "仅四舍五入有效；0 表示取整"),
                with_help(p("out", "输出字段", "text", false, json!("")), "留空表示覆盖原字段"),
            ],
        ),
        def(
            "text_concat",
            "文本拼接",
            "文本",
            "把多个字段拼接成一个新字段，例如「考场-座位号」生成座位标签。",
            vec![
                p("columns", "参与拼接的字段", "columns", true, json!([])),
                with_help(p("separator", "分隔符", "text", false, json!("")), "例如 - 或 /，留空表示直接相连"),
                p("out", "新字段名", "text", true, json!("拼接结果")),
            ],
        ),
        def(
            "text_substr",
            "文本截取",
            "文本",
            "从指定位置截取一段文字，例如从身份证号取出生日期、从学号取年级。",
            vec![
                p("column", "字段", "column", true, json!("")),
                with_help(p("start", "起始位置", "number", true, json!(1)), "从 1 开始计数"),
                with_help(p("length", "截取长度", "number", false, json!(0)), "0 表示截到末尾"),
                p("out", "新字段名", "text", true, json!("截取结果")),
            ],
        ),
        def(
            "text_pad",
            "补位格式化",
            "文本",
            "在文本左侧或右侧补字符到指定长度，常用于学号、准考证号、编号对齐。",
            vec![
                p("column", "字段", "column", true, json!("")),
                p("width", "目标长度", "number", true, json!(6)),
                p("pad", "补齐字符", "text", false, json!("0")),
                with_options(p("side", "补齐方向", "enum", false, json!("left")), &["left", "right"]),
                p("out", "输出字段", "text", false, json!("")),
            ],
        ),
        def(
            "text_replace",
            "文本替换",
            "文本",
            "把字段中的指定文字替换成新的文字，例如去掉手机号中的空格。",
            vec![
                p("column", "字段", "column", true, json!("")),
                p("find", "查找内容", "text", true, json!("")),
                p("replace", "替换为", "text", false, json!("")),
                p("out", "输出字段", "text", false, json!("")),
            ],
        ),
        def(
            "conditional",
            "条件赋值",
            "计算",
            "按条件给新字段赋值，例如总成绩 ≥ 60 标为「合格」，否则「不合格」。",
            vec![
                with_placeholder(p("condition", "判断条件", "expr", true, json!("")), "[总成绩] >= 60"),
                with_help(p("then", "条件成立时的值", "text", true, json!("")), "可直接写文字，也可写 [字段] 引用或数字"),
                with_help(p("else", "条件不成立时的值", "text", false, json!("")), "留空表示空值"),
                p("out", "新字段名", "text", true, json!("结果")),
            ],
        ),
        def(
            "date_calc",
            "日期计算",
            "计算",
            "按出生日期/报名日期计算年龄、加减天数、计算相差天数，或提取年、月、日。",
            vec![
                p("column", "日期字段", "column", true, json!("")),
                with_options(
                    p("mode", "计算方式", "enum", false, json!("age")),
                    &["age", "add_days", "diff_days", "year", "month", "day", "weekday"],
                ),
                with_help(p("days", "天数", "number", false, json!(0)), "日期加减使用，可为负数"),
                with_help(p("other", "对比日期字段", "column", false, json!("")), "计算相差天数时使用"),
                p("out", "新字段名", "text", true, json!("日期结果")),
            ],
        ),
        def(
            "parse_id_card",
            "身份证解析",
            "校验",
            "从身份证号提取出生日期、性别、年龄，并可校验号码是否合法。",
            vec![
                p("column", "身份证号字段", "column", true, json!("身份证号")),
                p("birth_out", "出生日期字段名", "text", false, json!("出生日期")),
                p("gender_out", "性别字段名", "text", false, json!("性别")),
                p("age_out", "年龄字段名", "text", false, json!("年龄")),
                with_help(p("valid_out", "校验结果字段名", "text", false, json!("")), "留空表示不输出校验结果"),
                with_options(p("on_error", "发现非法号码时", "enum", false, json!("null")), &["null", "fail"]),
            ],
        ),
        def(
            "amount_upper",
            "金额大写",
            "文本",
            "把金额数字转成中文大写，例如 1234.56 → 壹仟贰佰叁拾肆元伍角陆分。",
            vec![
                p("column", "金额字段", "column", true, json!("")),
                p("out", "输出字段", "text", false, json!("金额大写")),
            ],
        ),
        def(
            "percentage",
            "占比/百分比",
            "计算",
            "计算每行占分组合计的百分比，可按分组字段分别计算。",
            vec![
                p("column", "数值字段", "column", true, json!("")),
                p("out", "输出字段", "text", false, json!("占比")),
                p("decimals", "保留小数位", "number", false, json!(2)),
                with_help(p("partition_by", "分组字段", "columns", false, json!([])), "留空表示占全表合计"),
            ],
        ),
        def(
            "depreciation",
            "固定资产折旧",
            "计算",
            "按直线法计算月折旧额、累计折旧和资产净值，适用于固定资产台账。",
            vec![
                p("original", "原值字段", "column", true, json!("")),
                with_help(p("salvage_rate", "预计净残值率(%)", "number", false, json!(5)), "如 5 表示残值率为 5%"),
                p("years", "使用年限(年)", "number", false, json!(5)),
                with_help(p("months", "已计提月数字段（可空）", "column", false, json!("")), "留空按 0 个月计算"),
                p("monthly_out", "月折旧额字段名", "text", false, json!("月折旧额")),
                p("accumulated_out", "累计折旧字段名", "text", false, json!("累计折旧")),
                p("net_out", "净值字段名", "text", false, json!("固定资产净值")),
            ],
        ),
        def(
            "allocate",
            "按权重分摊",
            "计算",
            "把金额按权重分摊到多行，例如费用按人数分摊、总价按数量分摊。",
            vec![
                p("amount", "待分摊金额字段", "column", true, json!("")),
                p("weight", "权重字段", "column", true, json!("")),
                with_help(p("partition_by", "分摊范围字段（可空）", "columns", false, json!([])), "如按项目、部门分别分摊；留空表示全表"),
                p("out", "分摊结果字段名", "text", true, json!("分摊金额")),
                p("decimals", "保留小数位", "number", false, json!(2)),
            ],
        ),
        def(
            "tax_calc",
            "税额计算",
            "计算",
            "按税率拆分含税/不含税金额，支持价外税与价内税，适用于发票、报销与合同台账。",
            vec![
                p("column", "金额字段", "column", true, json!("")),
                with_help(p("rate", "税率", "number", true, json!(0.06)), "如 0.06 表示 6%，13% 填 0.13"),
                with_options(
                    p("mode", "金额口径", "enum", false, json!("exclusive")),
                    &["exclusive", "inclusive"],
                ),
                with_help(
                    p("decimals", "保留小数位", "number", false, json!(2)),
                    "税额与拆出金额按该精度四舍五入",
                ),
                p("net_out", "不含税金额字段名", "text", false, json!("不含税金额")),
                p("tax_out", "税额字段名", "text", false, json!("税额")),
                p("gross_out", "含税金额字段名", "text", false, json!("含税金额")),
            ],
        ),
        def(
            "budget_variance",
            "预算执行分析",
            "分析",
            "按部门/科目汇总预算与实际，输出执行率、差额和超支预警，适用于预算控制与费用分析。",
            vec![
                p("budget_column", "预算金额字段", "column", true, json!("")),
                p("actual_column", "实际金额字段", "column", true, json!("")),
                p("group_by", "分组字段（可空）", "columns", false, json!([])),
                with_help(p("warning_rate", "超支预警线(%)", "number", false, json!(100)), "执行率高于该值时标记为超支"),
                p("decimals", "保留小数位", "number", false, json!(2)),
                p("status_out", "状态字段名", "text", false, json!("预算状态")),
            ],
        ),
        def(
            "loan_payment",
            "等额本息月供",
            "计算",
            "按本金、年利率和期限计算等额本息月供、总利息与还款总额，适用于贷款台账与融资测算。",
            vec![
                p("principal", "本金字段", "column", true, json!("")),
                with_help(p("annual_rate", "年利率", "number", true, json!(0.049)), "如 4.9% 填 0.049"),
                p("years", "期限(年)", "number", true, json!(10)),
                p("decimals", "保留小数位", "number", false, json!(2)),
                p("payment_out", "月供字段名", "text", false, json!("月供")),
                p("interest_out", "总利息字段名", "text", false, json!("总利息")),
                p("total_out", "还款总额字段名", "text", false, json!("还款总额")),
            ],
        ),
        def(
            "reconcile",
            "对账核对",
            "分析",
            "按关键字段汇总本方与对方数据集金额，逐项比对并标记一致、金额不符或单边记录，适用于银行流水与账簿对账。",
            vec![
                p("dataset_id", "对方数据集", "dataset", true, json!("")),
                p("left_key", "本方关键字段", "column", true, json!("")),
                p("left_amount", "本方金额字段", "column", true, json!("")),
                p("right_key", "对方关键字段", "dataset_column", true, json!("")),
                p("right_amount", "对方金额字段", "dataset_column", true, json!("")),
                with_help(
                    p("tolerance", "允许差异", "number", false, json!(0.01)),
                    "金额差异在允许范围内视为一致",
                ),
                with_options(
                    p("mode", "输出范围", "enum", false, json!("diff")),
                    &["diff", "all"],
                ),
            ],
        ),
        def(
            "data_profile",
            "数据概览",
            "分析",
            "逐字段统计非空数、空值数、去重数、最小/最大值与平均值，导入后先跑一遍可快速发现数据质量问题。",
            vec![with_help(
                p("columns", "统计字段（可空）", "columns", false, json!([])),
                "留空表示统计全部字段",
            )],
        ),
        def(
            "not_null_check",
            "空值校验",
            "校验",
            "检查指定字段是否存在空值（NULL 或空字符串），例如姓名、身份证号不能为空。",
            vec![
                p("columns", "必须非空的字段", "columns", true, json!([])),
                with_options(p("on_error", "发现空值时", "enum", false, json!("fail")), &["fail", "warn"]),
            ],
        ),
        def(
            "column_compare",
            "两列一致性校验",
            "校验",
            "比较两列内容是否一致，例如姓名与身份证解析结果、订单金额与实收金额。",
            vec![
                p("left", "左字段", "column", true, json!("")),
                p("right", "右字段", "column", true, json!("")),
                with_help(p("ignore_blank", "忽略空值行", "bool", false, json!(true)), "任一列为空时不参与比较"),
                with_options(p("on_error", "发现不一致时", "enum", false, json!("fail")), &["fail", "warn"]),
            ],
        ),
        def(
            "format_check",
            "格式校验",
            "校验",
            "校验手机号、身份证号、邮箱、银行卡号等格式，考试报名与财务付款前常用。",
            vec![
                p("column", "待校验字段", "column", true, json!("")),
                with_options(
                    p("mode", "格式类型", "enum", false, json!("mobile")),
                    &["mobile", "id_card", "email", "bank_card"],
                ),
                with_options(p("on_error", "发现格式错误时", "enum", false, json!("fail")), &["fail", "warn"]),
            ],
        ),
        def(
            "range_check",
            "数值范围校验",
            "校验",
            "检查数值是否落在允许区间内，例如成绩必须在 0~100、金额不能为负。",
            vec![
                p("column", "数值字段", "column", true, json!("")),
                with_help(p("min", "最小值", "number", false, json!(null)), "留空表示不限制下限"),
                with_help(p("max", "最大值", "number", false, json!(null)), "留空表示不限制上限"),
                with_options(p("on_error", "发现越界时", "enum", false, json!("fail")), &["fail", "warn"]),
            ],
        ),
        def(
            "voucher_check",
            "凭证号连续性校验",
            "校验",
            "检查凭证号是否重复或断号，支持按期间/账套分组；自动识别「记-001」这类带前缀的编号。",
            vec![
                p("column", "凭证号字段", "column", true, json!("")),
                with_help(
                    p("group_by", "分组字段（可空）", "columns", false, json!([])),
                    "如按期间、账套分别检查连续性",
                ),
                with_help(
                    p("check_gap", "检查断号", "bool", false, json!(true)),
                    "关闭后只检查重复凭证号",
                ),
                with_options(
                    p("on_error", "发现问题时", "enum", false, json!("fail")),
                    &["fail", "warn"],
                ),
            ],
        ),
        def(
            "sum_check",
            "合计一致性校验",
            "校验",
            "校验合计字段是否等于各明细字段之和，例如总成绩与各科成绩、发票金额与明细合计。",
            vec![
                p("total", "合计字段", "column", true, json!("")),
                p("parts", "明细字段", "columns", true, json!([])),
                with_help(p("tolerance", "允许误差", "number", false, json!(0.01)), "绝对值不超过该误差视为一致"),
                with_options(
                    p("on_error", "不一致时", "enum", false, json!("fail")),
                    &["fail", "warn"],
                ),
            ],
        ),
        def(
            "balance_check",
            "借贷平衡校验",
            "校验",
            "按凭证号汇总借方和贷方金额，检查每张凭证是否平衡。财务记账、报销汇总前的必备校验。",
            vec![
                p("voucher", "凭证号字段", "column", true, json!("")),
                p("debit", "借方金额字段", "column", true, json!("")),
                p("credit", "贷方金额字段", "column", true, json!("")),
                with_help(p("tolerance", "允许误差", "number", false, json!(0.01)), "借贷差额绝对值不超过该值视为平衡"),
                with_options(p("on_error", "发现不平衡时", "enum", false, json!("fail")), &["fail", "warn"]),
            ],
        ),
        def(
            "split_column",
            "拆分字段",
            "字段",
            "按分隔符把一列拆成多列，例如把「考场-座位」拆成考场和座位两列。",
            vec![
                p("column", "待拆分字段", "column", true, json!("")),
                p("delimiter", "分隔符", "text", true, json!("-")),
                with_help(p("max_parts", "拆分段数", "number", false, json!(2)), "最多拆成多少列"),
                with_help(p("names", "新字段名", "text", false, json!("")), "用逗号分隔，如 考场,座位；留空自动命名「原字段_1」"),
            ],
        ),
        def(
            "append_dataset",
            "纵向合并数据表",
            "行",
            "把另一个数据集的行追加到当前数据下面，按字段名对齐，缺失字段自动补空。适合合并多批报名表、多个月流水。",
            vec![
                with_help(p("dataset_id", "追加数据集", "dataset", true, json!("")), "选择要追加到当前数据后面的数据集"),
                with_help(p("dedupe", "去除完全重复的行", "bool", false, json!(false)), "关闭时保留全部行（UNION ALL）"),
            ],
        ),
        def(
            "join_dataset",
            "关联数据表",
            "字段",
            "按关键字段把另一个数据集的字段关联进来，类似 Excel 的 VLOOKUP。例如按学号关联考生信息，按订单号关联收款记录。",
            vec![
                with_help(p("dataset_id", "关联数据集", "dataset", true, json!("")), "选择要从中取数的另一个数据集"),
                p("left_field", "当前表关键字段", "column", true, json!("")),
                p("right_field", "关联表关键字段", "dataset_column", true, json!("")),
                with_help(
                    p("fields", "取用字段", "dataset_fields", false, json!([])),
                    "留空表示取关联表的全部字段；重名字段会自动加后缀",
                ),
                with_options(p("join_type", "关联方式", "enum", false, json!("left")), &["left", "inner"]),
                with_help(p("prefix", "字段前缀", "text", false, json!("")), "如填「档案_」，关联字段会变成「档案_姓名」"),
            ],
        ),
        def(
            "value_map",
            "值映射替换",
            "字段",
            "把字段中的特定值替换成新值，例如 男→M、女→F、缺考→0。",
            vec![
                p("column", "字段", "column", true, json!("")),
                with_help(p("mappings", "值映射", "mappings", true, json!([])), "原始值 → 新值；未列出的值保持不变"),
                with_help(p("out", "输出字段", "text", false, json!("")), "留空表示覆盖原字段"),
                with_help(p("keep_unmatched", "保留未匹配的值", "bool", false, json!(true)), "关闭后未匹配的值输出为空"),
            ],
        ),
        def(
            "text_clean",
            "文本清洗",
            "文本",
            "清理导入数据中的多余空格、不可见字符，或统一大小写、全角半角。",
            vec![
                p("column", "字段", "column", true, json!("")),
                with_options(
                    p("mode", "清洗方式", "enum", false, json!("trim")),
                    &["trim", "remove_spaces", "upper", "lower", "full_to_half"],
                ),
                with_help(p("out", "输出字段", "text", false, json!("")), "留空表示覆盖原字段"),
            ],
        ),
        def(
            "shuffle",
            "随机打散",
            "行",
            "随机打乱行顺序；按分组打散时各组轮流穿插，避免同校、同岗位考生连号相邻。",
            vec![
                with_help(
                    p("group_by", "分组字段（可空）", "columns", false, json!([])),
                    "留空表示全表随机；选择分组后各组按轮次穿插排列",
                ),
                p("out", "随机序号字段名", "text", false, json!("随机序号")),
            ],
        ),
        def(
            "assign_seats",
            "考场座位编排",
            "计算",
            "按顺序自动分配考场号和座位号，可设置每考场人数与起始考场。",
            vec![
                with_help(p("order_by", "编排顺序字段", "column", false, json!("")), "如按准考证号、姓名排序后编排；留空按原始顺序"),
                p("per_room", "每考场人数", "number", false, json!(30)),
                p("start_room", "起始考场号", "number", false, json!(1)),
                p("room_out", "考场号字段名", "text", false, json!("考场号")),
                p("seat_out", "座位号字段名", "text", false, json!("座位号")),
            ],
        ),
        def(
            "sequence",
            "行号/序号",
            "计算",
            "为每行生成连续序号，可按分组重新编号（如每个考场从 1 开始）。",
            vec![
                p("out", "序号字段名", "text", false, json!("序号")),
                p("start", "起始序号", "number", false, json!(1)),
                with_help(p("order_by", "排序依据（可空）", "column", false, json!("")), "按该字段排序后编号"),
                with_help(p("partition_by", "分组字段（可空）", "columns", false, json!([])), "每个分组内单独编号"),
            ],
        ),
        def(
            "exam_number",
            "准考证号生成",
            "计算",
            "按前缀、起始序号和位宽批量生成准考证号，可按考点/考场分组重新编号。",
            vec![
                with_help(
                    p("prefix", "号码前缀（可空）", "text", false, json!("")),
                    "如 ZK、2026，留空只生成数字",
                ),
                p("start", "起始序号", "number", false, json!(1)),
                with_help(
                    p("width", "数字位宽", "number", false, json!(6)),
                    "不足位宽左侧补 0，如 6 位生成 000001",
                ),
                with_help(
                    p("group_by", "分组字段（可空）", "columns", false, json!([])),
                    "选择考点/考场后，各组分别从起始序号编号",
                ),
                p("order_by", "编号顺序字段（可空）", "column", false, json!("")),
                p("out", "输出的号码字段名", "text", false, json!("准考证号")),
            ],
        ),
        def(
            "multi_column_stat",
            "多列统计",
            "计算",
            "对多列横向求和/平均/最大/最小/计数，例如计算多科总分，或汇总多项费用。",
            vec![
                p("columns", "参与计算的字段", "columns", true, json!([])),
                with_options(p("mode", "计算方式", "enum", false, json!("sum")), &["sum", "avg", "max", "min", "count"]),
                p("out", "输出字段名", "text", true, json!("合计")),
                with_help(p("decimals", "保留小数位", "number", false, json!(2)), "计数与最大最小不取整"),
            ],
        ),
        def(
            "cumulative",
            "累计求和",
            "计算",
            "按顺序累计求和，财务上常用于计算累计发生额、期初余额。",
            vec![
                p("column", "数值字段", "column", true, json!("")),
                p("out", "输出字段名", "text", true, json!("累计金额")),
                with_help(p("order_by", "累计顺序字段", "column", false, json!("")), "例如日期、凭证号；留空按原始顺序"),
                with_help(p("partition_by", "分组字段（可空）", "columns", false, json!([])), "如按科目、客户分别累计"),
                p("decimals", "保留小数位", "number", false, json!(2)),
            ],
        ),
        def(
            "window_compare",
            "同比/环比",
            "计算",
            "与上一期数据比较，计算增长额或增长率。按日期排序后即可得到环比；错开一年即为同比。",
            vec![
                p("column", "数值字段", "column", true, json!("")),
                p("order_by", "期间顺序字段", "column", true, json!("")),
                with_help(p("partition_by", "分组字段（可空）", "columns", false, json!([])), "如按科目、门店分别比较"),
                with_options(p("mode", "比较方式", "enum", false, json!("pct")), &["pct", "diff"]),
                with_help(p("periods", "间隔期数", "number", false, json!(1)), "1 表示与上一期比较"),
                p("out", "输出字段名", "text", true, json!("环比")),
                p("decimals", "保留小数位", "number", false, json!(2)),
            ],
        ),
        def(
            "aging_bucket",
            "账龄分析",
            "计算",
            "按应收/应付日期或逾期天数划分账龄区间，例如 0-30 天、31-90 天、90 天以上。",
            vec![
                p("column", "日期或天数字段", "column", true, json!("")),
                with_options(p("mode", "字段类型", "enum", false, json!("date")), &["date", "days"]),
                with_help(p("buckets", "分段点", "text", false, json!("30,60,90,180")), "逗号分隔，按天数从小到大"),
                p("out", "输出字段名", "text", false, json!("账龄")),
            ],
        ),
        def(
            "grade_level",
            "等级划分",
            "计算",
            "按分数阈值把成绩划分为 A/B/C/D 等，例如 90 分以上 A、80 分以上 B。",
            vec![
                p("column", "成绩字段", "column", true, json!("")),
                with_help(p("cutoffs", "分数阈值", "text", true, json!("90,80,70,60")), "逗号分隔，从高到低"),
                with_help(p("labels", "等级名称", "text", false, json!("A,B,C,D,E")), "逗号分隔，数量可比阈值多一个"),
                p("out", "输出字段名", "text", false, json!("等级")),
            ],
        ),
        def(
            "standard_score",
            "标准分/百分位",
            "计算",
            "把原始分转换为 Z 分、T 分或百分位排名，便于不同科目、不同群体之间比较。",
            vec![
                p("column", "成绩字段", "column", true, json!("")),
                with_options(p("mode", "转换方式", "enum", false, json!("z")), &["z", "t", "percentile"]),
                with_help(p("partition_by", "分组字段（可空）", "columns", false, json!([])), "如按科目、考场分别计算"),
                p("out", "输出字段名", "text", false, json!("标准分")),
                p("decimals", "保留小数位", "number", false, json!(2)),
            ],
        ),
        def(
            "score_summary",
            "成绩统计汇总",
            "计算",
            "按分组一次算出人数、平均分、最高分、最低分、及格率、优秀率和标准差，直接生成成绩分析表。",
            vec![
                p("column", "成绩字段", "column", true, json!("")),
                with_help(p("group_by", "分组字段（可空）", "columns", false, json!([])), "如按班级、考场、科目分组"),
                with_help(p("pass_score", "及格分", "number", false, json!(60)), "达到该分数计为及格"),
                with_help(p("excellent_score", "优秀分", "number", false, json!(85)), "达到该分数计为优秀"),
                p("decimals", "保留小数位", "number", false, json!(2)),
            ],
        ),
        def(
            "score_segment",
            "分数段统计",
            "计算",
            "统计各分数段的人数，可生成一分一段表或按 10 分一段汇总。",
            vec![
                p("column", "成绩字段", "column", true, json!("")),
                with_help(p("step", "分段宽度", "number", false, json!(10)), "1 表示一分一段，10 表示每 10 分一段"),
                with_help(p("partition_by", "分组字段（可空）", "columns", false, json!([])), "如按科目、学校分别统计"),
                with_help(p("cumulative", "输出累计人数", "bool", false, json!(true)), "累计人数用于查看位次"),
                p("segment_out", "分数段字段名", "text", false, json!("分数段")),
                p("count_out", "人数字段名", "text", false, json!("人数")),
            ],
        ),
        def(
            "pivot",
            "透视表",
            "计算",
            "把某一列的不同取值展开成多列并汇总，例如按科目展开成绩、按月份展开收支。",
            vec![
                p("group_by", "行分组字段", "columns", true, json!([])),
                p("pivot_column", "展开字段", "column", true, json!("")),
                p("value_column", "汇总数值字段", "column", true, json!("")),
                with_options(p("agg", "汇总方式", "enum", false, json!("sum")), &["sum", "count", "avg", "max", "min"]),
                with_help(p("max_columns", "最多展开列数", "number", false, json!(50)), "展开取值过多时建议先筛选"),
                p("decimals", "保留小数位", "number", false, json!(2)),
            ],
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
