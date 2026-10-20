//! 面向用户的表达式编译器。
//!
//! 用户写的是类似 VFP / Excel 的公式，字段用中括号包裹：
//!   [笔试成绩]*0.6 + [面试成绩]*0.4
//!   [笔试成绩]>=60 AND [面试成绩]<>"缺考"
//! 编译器把 `[字段]` 翻译成带引号的 SQL 标识符，并拒绝任何未登记的裸标识符，
//! 因此最终生成的是可直接交给 SQLite 执行的 SQL，而不会引入意外注入。

const KEYWORDS: &[&str] = &[
    "AND", "OR", "NOT", "IS", "NULL", "IN", "LIKE", "GLOB", "BETWEEN", "CASE", "WHEN", "THEN",
    "ELSE", "END", "TRUE", "FALSE", "ESCAPE",
];

const FUNCTIONS: &[&str] = &[
    "ABS", "ROUND", "MIN", "MAX", "COALESCE", "IFNULL", "NULLIF", "IIF", "LENGTH", "LOWER",
    "UPPER", "TRIM", "LTRIM", "RTRIM", "SUBSTR", "REPLACE", "INSTR", "PRINTF", "FORMAT", "CAST",
    "CEIL", "CEILING", "FLOOR", "SQRT", "POWER", "POW", "MOD", "INT", "REAL", "TEXT", "SUM",
    "AVG", "COUNT", "TOTAL", "DATE", "DATETIME", "STRFTIME", "JULIANDAY", "TYPEOF", "HEX",
    "ROUNDUP", "GREATEST", "LEAST",
];

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Field(String),
    Num(String),
    Str(String),
    Word(String),
    Op(String),
    LParen,
    RParen,
    Comma,
}

fn tokenize(src: &str) -> Result<Vec<(Tok, usize)>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        match c {
            '[' => {
                i += 1;
                let mut name = String::new();
                while i < chars.len() && chars[i] != ']' {
                    name.push(chars[i]);
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("缺少右中括号 ]".to_string());
                }
                i += 1; // skip ]
                let name = name.trim().to_string();
                if name.is_empty() {
                    return Err("空的字段名 []".to_string());
                }
                out.push((Tok::Field(name), start));
            }
            '\'' => {
                i += 1;
                let mut s = String::new();
                let mut closed = false;
                while i < chars.len() {
                    if chars[i] == '\'' {
                        if i + 1 < chars.len() && chars[i + 1] == '\'' {
                            s.push('\'');
                            i += 2;
                            continue;
                        }
                        i += 1;
                        closed = true;
                        break;
                    }
                    s.push(chars[i]);
                    i += 1;
                }
                if !closed {
                    return Err("字符串缺少右单引号 '".to_string());
                }
                out.push((Tok::Str(s), start));
            }
            '(' => {
                out.push((Tok::LParen, start));
                i += 1;
            }
            ')' => {
                out.push((Tok::RParen, start));
                i += 1;
            }
            ',' => {
                out.push((Tok::Comma, start));
                i += 1;
            }
            _ if c.is_ascii_digit() || (c == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit()) => {
                let mut n = String::new();
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    n.push(chars[i]);
                    i += 1;
                }
                out.push((Tok::Num(n), start));
            }
            _ if c.is_alphabetic() || c == '_' => {
                let mut w = String::new();
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    w.push(chars[i]);
                    i += 1;
                }
                out.push((Tok::Word(w), start));
            }
            _ => {
                let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
                let two_upper = two.to_uppercase();
                if matches!(two_upper.as_str(), ">=" | "<=" | "<>" | "!=" | "==" | "&&" | "||") {
                    out.push((Tok::Op(two_upper.clone()), start));
                    i += 2;
                } else if "+-*/%=<>!".contains(c) {
                    out.push((Tok::Op(c.to_string()), start));
                    i += 1;
                } else {
                    return Err(format!("表达式中有无法识别的字符 `{c}`"));
                }
            }
        }
    }
    Ok(out)
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// 编译表达式 → SQLite SQL 片段。
pub fn compile(expr: &str, columns: &[String]) -> Result<String, String> {
    let src = expr.trim();
    if src.is_empty() {
        return Err("表达式不能为空".to_string());
    }
    let toks = tokenize(src)?;
    let mut parts: Vec<String> = Vec::with_capacity(toks.len());
    for (idx, (t, pos)) in toks.iter().enumerate() {
        match t {
            Tok::Field(name) => {
                if !columns.iter().any(|c| c == name) {
                    return Err(format!(
                        "字段 [{name}] 不存在。当前可用字段：{}",
                        preview_columns(columns)
                    ));
                }
                parts.push(quote_ident(name));
            }
            Tok::Num(n) => parts.push(n.clone()),
            Tok::Str(s) => parts.push(format!("'{}'", s.replace('\'', "''"))),
            Tok::Word(w) => {
                let up = w.to_uppercase();
                if KEYWORDS.contains(&up.as_str()) {
                    parts.push(up);
                } else if FUNCTIONS.contains(&up.as_str()) {
                    let next_is_paren = matches!(toks.get(idx + 1).map(|x| &x.0), Some(Tok::LParen));
                    if !next_is_paren {
                        return Err(format!("函数 {up} 后面缺少 ("));
                    }
                    parts.push(up);
                } else {
                    return Err(format!(
                        "第 {} 个字符处出现未知标识符 `{w}`：字段请用中括号写成 [字段名]，函数/关键字请使用 SQL 内置名称",
                        pos + 1
                    ));
                }
            }
            Tok::Op(o) => match o.as_str() {
                "&&" => parts.push("AND".to_string()),
                "||" => parts.push("OR".to_string()),
                "==" => parts.push("=".to_string()),
                "!=" => parts.push("<>".to_string()),
                other => parts.push(other.to_string()),
            },
            Tok::LParen => parts.push("(".to_string()),
            Tok::RParen => parts.push(")".to_string()),
            Tok::Comma => parts.push(",".to_string()),
        }
    }
    Ok(parts.join(" "))
}

pub fn preview_columns(columns: &[String]) -> String {
    let head: Vec<&str> = columns.iter().take(15).map(|s| s.as_str()).collect();
    let mut s = head.join("、");
    if columns.len() > 15 {
        s.push_str(" …");
    }
    s
}

/// 数值合法性判断（用于类型转换的严格模式）：允许数字、正负号、小数点、e/E。
pub fn numeric_guard_sql(col: &str) -> String {
    let q = quote_ident(col);
    format!(
        "TEXT({q}) IS NOT NULL AND TRIM(TEXT({q})) <> '' AND TRIM(TEXT({q})) NOT GLOB '*[^0-9eE+-.]*'"
    )
}

pub fn quote_column(col: &str) -> String {
    quote_ident(col)
}
