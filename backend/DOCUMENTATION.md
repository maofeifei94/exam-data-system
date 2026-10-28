# 后端设计文档（exam-backend）

考试数据处理系统后端：Rust + Axum + rusqlite（bundled SQLite）。
默认监听 `127.0.0.1:8710`，所有业务接口统一以 `/api` 为前缀。

本文件说明**每个文件负责什么**、**数据在数据库里怎么存**、**核心算法怎么算**，
方便排查问题和二次开发。

---

## 1. 总体架构

```
Electron 前端
   │  HTTP/JSON（少数接口返回 PDF/CSV 二进制）
   ▼
Axum Router (routes/mod.rs)
   │  每个 handler 都是 async，但数据库操作一律下沉
   ▼
AppState → DbPool（固定容量 SQLite 连接池）
   │  state.db(...) 把闭包调度到 spawn_blocking 阻塞线程
   ▼
store.rs / engine / 各 routes 内联 SQL
   │  复合写操作统一用 db::with_tx 包事务
   ▼
SQLite（WAL 模式）
```

三条硬性约定，全仓库必须遵守：

1. **数据库操作不占用 tokio 工作线程**。rusqlite 是同步 API，任何 handler 都通过
   `state.db(...)` / `state.blocking(...)` 调度到阻塞线程池执行。
2. **复合写操作必须包事务**。多步写入（建表 + 写数据 + 写元数据）统一用
   `db::with_tx`，闭包返回 `Err` 时自动回滚，避免产生「一半成功」的脏数据。
3. **不 panic**。生产代码禁止 `unwrap` / `expect` / `unreachable`，所有失败路径返回
   `AppError`，由统一错误契约序列化成 JSON。

---

## 2. 文件清单与职责

### 2.1 入口与基础设施

| 文件 | 职责 |
| --- | --- |
| `src/main.rs` | 进程入口。读取环境变量 → 打开数据库 → `db::init` 建表与种子数据 → 构建路由 → 监听端口。 |
| `src/bin/pdfcheck.rs` | 离线自检工具：验证 printpdf + 中文字体 + 位图嵌入链路是否正常，不参与线上服务。 |
| `src/state.rs` | **连接池 + 阻塞任务隔离**。`DbPool`（固定容量、超时等待、panic 安全归还）与 `AppState`，对外提供 `state.db()` / `state.blocking()`。 |
| `src/db.rs` | 建库语句 `SCHEMA`、`migrate`（老库补列）、`seed`（种子字典）；工具函数 `new_id` / `now` / `table_name`（表名白名单）/ `configure`（连接级 PRAGMA）/ `with_tx`（事务包装）。 |
| `src/error.rs` | **统一错误契约**。`AppError { status, code, message }`，`IntoResponse` 序列化为 `{"error": "...", "code": "..."}`；内部细节只记日志，不外泄。 |
| `src/util.rs` | 类型推断 `infer_dtype`、`name_forces_text`、`urlencode`、`parse_hex_color` 等无状态小工具。 |
| `src/models.rs` | 全部请求/响应数据结构（serde）。含 `TemplateElement` 元素树、`TemplateBody` 模板正文及其 `normalize` 迁移逻辑。 |

### 2.2 数据访问

| 文件 | 职责 |
| --- | --- |
| `src/store.rs` | 数据集存储层。建表、写数据、读分页、读全量字符串、删数据集、字段智能匹配建议。 |
| `src/importer.rs` | `xlsx / xls / ods / csv / tsv` 解析，CSV 编码自动识别，表头切分与空行/空列清理。 |
| `src/engine/mod.rs` | 流程引擎。把可编辑的操作符流程编译成一条 SQL（命名 CTE 串联）并执行、写运行日志；`preview_step` 提供不落库的节点数据预览。 |
| `src/engine/expr.rs` | 面向用户的表达式编译器：`[字段]*0.6 + [面试成绩]*0.4` → 安全 SQL。 |
| `src/engine/operators.rs` | 原子操作符目录（类型、标签、参数 schema、默认值），前端据此渲染参数表单。 |
| `src/pdfgen/mod.rs` | PDF 渲染引擎：坐标系换算、占位符插值、矩阵网格排版、照片文件解析。 |

### 2.3 路由层

每个路由文件都是「参数校验 → state.db 取/写数据 → JSON 或二进制响应」，
不直接持有连接、不做跨请求状态。

| 文件 | 覆盖接口 |
| --- | --- |
| `src/routes/mod.rs` | 路由注册表，唯一集中定义所有 URL 与 HTTP 方法的地方。 |
| `src/routes/projects.rs` | 考试项目 CRUD。删除项目时解绑关联数据集；可把项目绑定写回配置文件。 |
| `src/routes/datasets.rs` | 数据集列表/详情/删除、分页行读取、CSV 导出、文件导入、字段映射应用。 |
| `src/routes/fields.rs` | 标准字段、字段分组、别名映射；导入列映射建议。 |
| `src/routes/project_fields.rs` | 项目本地字段（项目模板）CRUD、项目本地别名 CRUD（`/api/project-aliases`），以及从全局字段库幂等复制字段+别名到本项目的 `/api/project-fields/copy`。 |
| `src/routes/flows.rs` | 流程 CRUD、复制、运行、运行记录、静态校验、操作符目录、节点数据预览（`/steps/:index/rows`）。 |
| `src/routes/pdf.rs` | PDF 模板 CRUD、复制、按已存模板渲染、按草稿模板预览。 |

---

## 3. 数据结构

### 3.1 元数据表（`db::SCHEMA`）

| 表 | 说明 |
| --- | --- |
| `exam_projects` | 考试项目。绑定 `flow_id` / `pdf_template_id` / `dataset_id`（均为可空字符串）。 |
| `datasets` | 数据集元信息：名称、来源 kind（import/flow）、源文件、所属项目、行数。 |
| `dataset_columns` | 列元信息，主键 `(dataset_id, idx)`；`name` 是显示名，`dtype` 决定物理列类型。 |
| `field_groups` | 字段分组，仅影响「字段别名」页的展示顺序。 |
| `standard_fields` | 标准字段字典，`name` 唯一；含 `precision`（小数位）、`group_id`、`system` 标记。 |
| `alias_mappings` | 别名 → 标准名的映射，`alias` 唯一，用于导入时自动建议。 |
| `flows` | 流程定义，`steps` 列存 JSON 数组；`scope` = global/project。 |
| `flow_runs` | 流程运行记录：状态、消息、生成的 SQL、输出数据集 id。 |
| `pdf_templates` | PDF 模板，`template` 列存 `TemplateBody` 的 JSON。 |
| `project_fields` | 项目本地字段副本。由全局 `standard_fields` 挑选复制而来，`UNIQUE(project_id, name)`；修改只影响本项目，不回写全局字典。 |
| `project_alias_mappings` | 项目本地别名。复制全局字段时连同别名单一起复制，`UNIQUE(project_id, alias)`；可独立增删，不写回 `alias_mappings`。 |

索引：`datasets(project_id)`、`flows(project_id)`、`pdf_templates(project_id)`、
`project_fields(project_id)`、`project_alias_mappings(project_id)`、`flow_runs(flow_id)`、
`flow_runs(dataset_id)`，避免项目/流程列表全表扫描。

`project_fields` 与 `standard_fields` 字段结构一致（`name / label / dtype / precision /
group_id / description`），额外带 `project_id` 与 `source_id`（来源全局字段 id，仅作溯源）。

作用域规则：**项目内只能引用本项目字段**。导入建议（`store::suggest_columns` 收到
`project_id` 时）只查 `project_fields`，别名只查 `project_alias_mappings`；
流程编辑器、导入映射、项目 PDF 模板的字段候选同样只来自 `project_fields`。
全局字段必须先在「项目模板」中复制到本项目，才能被项目内的数据集/流程/模板引用。

### 3.2 动态数据表 `data_<dataset_id>`

每个数据集一张物理表，列名固定为 `c0`、`c1`、`c2`……：

- 表的**真实列名**永远是 `c<序号>`，可读的列名与类型存在 `dataset_columns`。
  这样列改名、改类型只是元数据操作，不受 SQLite 标识符限制。
- 物理类型由 `dtype` 决定：`number` → `REAL`，其余（含 `text`、`date`、`decimal`、
  `integer`）→ `TEXT`。**日期/小数保持文本存储**，避免 SQLite 数值亲和性丢精度。
- 表名由 `db::table_name(dataset_id)` 生成，对 id 做白名单校验（仅字母数字下划线、
  长度 ≤ 64），防止路径参数拼接造成 SQL 注入。

### 3.3 模板元素树（`TemplateElement`）

模板正文 `TemplateBody` 由「页面设置 + 矩阵设置（兼容字段）+ 元素树」组成。
元素是一个**递归树**：

```
TemplateElement
  ├─ type: text | field | image | line | rect | matrix
  ├─ x_mm / y_mm / w_mm / h_mm   相对父元素左上角，单位毫米
  ├─ text / field                文本内容 / 绑定的数据字段
  ├─ font_size / bold / align / color / fill / border / border_radius_mm
  ├─ children: Vec<TemplateElement>   子元素（坐标为父元素相对坐标）
  ├─ matrix: MatrixSetup?             type=matrix 时的网格配置
  └─ photo_dir / photo_name / photo_exts   type=image 的照片解析三要素
```

坐标系约定（关键）：**根元素相对页面内容区**（页面去掉页边距后的区域）左上角；
**子元素相对父元素左上角**；矩阵的 `children` 会被复制到每一个单元格，
其坐标相对「单元格左上角」。前端拖动时同样遵守该规则（见前端文档 `toParentBox`）。

### 3.4 错误契约（`error.rs`）

响应体固定为：

```json
{ "error": "给用户看的信息", "code": "machine_readable_code" }
```

`code` 是稳定标识，前端只按 `code` 分支，不匹配文案。常见值：
`bad_request`、`validation_error`、`not_found`、`conflict`、`internal_error`。
`internal` 类错误对外只返回通用提示，具体原因写日志，避免泄露文件路径/SQL。

### 3.5 节点数据快照（`StepData`）

「数据流」栏目按节点查看数据时返回的结构（`GET /api/flows/:id/steps/:index/rows`）：

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `step_index` | number | 0 基节点下标。等于步骤总数时代表整条流程的最终输出。 |
| `label` | string | 节点显示名（步骤标签，缺省时回退操作符中文名）。 |
| `operator` | string | 操作符类型；终点节点固定为 `output`。 |
| `columns` | `DatasetColumn[]` | 该节点执行后可见的列（显示名 + 类型）。 |
| `rows` | `Value[][]` | 分页后的行数据，单元格类型与数据集一致。 |
| `total` | number | 该节点的总行数；若流程含 `limit` 步骤，按截断后的上限计。 |

`StepData` 是一次性快照，**不写库**：它由 `engine::preview_step` 现算现返回，
不产生新的数据集，也不污染 `datasets` 表。

---

## 4. 关键算法

### 4.1 连接池与阻塞隔离（`state.rs`）

- `DbPool` 用 `Mutex<Vec<Connection>> + Condvar` 实现固定容量池；`acquire` 带超时，
  超时返回明确错误而不是无限等待。
- 执行闭包时用 `catch_unwind` 捕获 panic：若闭包 panic，则**丢弃该连接**而不是归还，
  保证「连接要么可用、要么被回收」，池容量不会因一次 panic 永久泄漏。
- 因为连接是 `Arc<Mutex<...>>` 跨线程，**事务内联在单个闭包中完成**，
  不允许跨 `state.db` 调用做「先读后写」，避免使用不同连接导致事务失效。

### 4.2 事务边界（`db::with_tx`）

```rust
pub fn with_tx<T>(conn: &mut Connection, f: impl FnOnce(&Connection) -> AppResult<T>) -> AppResult<T>
```

约定：所有「建表 + 写数据 + 写元数据」的操作都要包一层。为支持嵌套组合，
`store.rs` 里的复合操作都拆成两个版本：

- `xxx_tx(&Connection, ...)`：**纯事务内实现**，调用方负责提交/回滚，内部绝不 `BEGIN`；
- `xxx(&mut Connection, ...)`：对外便捷版，自动 `with_tx`。

路由层在更大的事务里直接调用 `_tx` 版本，避免嵌套事务报错。
典型例子：删除标准字段时，同一事务内先删指向它的别名再删字段；
重命名标准字段时，同一事务内同步 `alias_mappings.standard`，否则别名会「留在旧名上」。

### 4.3 列名去重（`store::unique_names`）

导入时对列名做去重：空名填「未命名列」；重名依次追加 `(2)`、`(3)`……
保证同一数据集内列名唯一，前端和流程表达式引用不产生歧义。

### 4.4 类型推断（`util::infer_dtype`）

对每个列最多采样 200 个非空值，按顺序判断：

1. 列名命中 `name_forces_text` 的敏感片段（证号、身份证、考号、编号、手机…）→ `text`
   （避免 18 位身份证被当成 REAL 变成科学计数法）；
2. 全部像数字 `safe_number`（可解析、长度 ≤ 15、无前导零歧义）→ `number`；
3. 全部像日期或数字 → `date`；
4. 其余 → `text`。

照片**不是数据类型**：照片文件名按文本存储，文件是否存在由 PDF 渲染阶段解析。

### 4.5 导入（`importer.rs`）

- 行数上限保护：超过 20 万行直接报错，避免无界内存占用。
- 尾随空列裁剪、全空行跳过、表头行号合法性校验。
- CSV/TSV 先按 UTF-8 解析，失败则按 GBK 解码，兼容国内导出的表格。
- 浮点格式化 `format_float` 去掉尾随零，避免 `1.500000` 这类显示噪声。

### 4.6 表达式编译（`engine/expr.rs`）

用户写类 Excel/VFP 公式，字段用中括号：`[笔试成绩]*0.6 + [面试成绩]*0.4`。

- 词法分析 → 语法检查 → 只把**已登记字段**翻译成带引号的 SQL 标识符；
- 任何未登记的裸标识符、非法运算符都直接报错。
- 因此最终生成的是可直接执行的 SQL，而不是把用户输入原样拼接，天然防注入。
- 附带 `numeric_guard_sql`：对参与算术的列做数值兜底，非数字按 0 处理。

### 4.7 流程编译（`engine/mod.rs`）

每个步骤生成一段命名 CTE（`s0`、`s1`、`s2`…），最终拼成一条完整 SQL：

```sql
WITH s0 AS (SELECT "c0" AS "身份证号", ... FROM "data_xxx"),
     s1 AS (SELECT *, ROUND(("笔试成绩"*0.6)+("面试成绩"*0.4), 2) AS "总成绩" FROM s0),
     s2 AS (SELECT *, RANK() OVER (ORDER BY "总成绩" DESC) AS "排名" FROM s1)
SELECT * FROM s2 ORDER BY "总成绩" DESC
```

- 这条 SQL 会写入 `flow_runs.sql_text`，用户可直观看到流程做了什么。
- 运行成功路径整体在**一个事务**里：`CREATE TABLE AS SELECT` 产出新数据集 →
  写 `datasets` / `dataset_columns` → 写运行记录；任一步失败全部回滚，不留半成品。
- `count_current` 等辅助函数返回 `Result`，不再 `unwrap`。
- 在校验接口 `validate_steps` 中做静态数据流分析：逐步推导每步的输入/输出字段集合，
  提前暴露「引用了不存在的字段」这类问题，无需真正执行。

### 4.8 节点数据预览（`engine::preview_step`）

「数据流」栏目点开某个节点时，无需真正运行流程，只要看到该节点「执行完之后的表」。

做法是**复用同一套编译器，只编译到目标节点为止**，然后直接 `SELECT`：

```text
节点 i 的数据 = 执行完第 i 步之后的数据
终点节点（i == 步骤数）= 执行完全部步骤后的最终输出（含排序 / limit）
```

- 步骤下标先校验范围 `0..=steps.len()`，越界返回 `bad_request`；
- 跳过 `enabled == false` 的步骤，与正式运行的语义保持一致；
- 只对已编译的 CTE 链做 `count_current` 与分页查询，**不执行 `CREATE TABLE`**；
- 若流程含 `limit` 步骤，`total` 取实际行数与上限的较小值，分页 `take` 同步收敛；
- 单次请求行数上限 `MAX_PREVIEW_ROWS = 2000`，防止巨表拖垮接口；
- 单元格用 `store::json_value` 转成 JSON，缺失列补 `Null`（列数随步骤变化时保持行宽一致）。

编译期错误会带上「第 N 步「某操作」执行失败」的上下文，便于定位是哪个节点参数写错。

### 4.9 PDF 渲染（`pdfgen/mod.rs`）

渲染分为两段，避免长时间占用连接池：

- 路由层在阻塞线程里**读出数据后立即释放连接**；
- 纯渲染函数 `render_pdf_data(col_meta, rows, body, kind, opts)` 只做 CPU 密集排版。

要点：

- **占位符插值** `interpolate`：支持 `{字段}` 取数据值，`{{index}}`/`{{no}}`/`{{page}}`/
  `{{pages}}`/`{{count}}` 内置变量；逐字符扫描，不做正则回溯。
- **文本宽度** `text_width_mm`：按字符类型（CJK / ASCII / 其他）估算，用于对齐计算。
- **矩阵排版**：网格是连续栅格（无间距），`children` 复制到每个 `rows × cols` 单元格；
  `page_level` 元素每页只画一次（标题等）。
- **照片解析** `resolve_photo(name, dir, exts)`：先按名称原样（含绝对路径）查找，
  再按目录 + `名称.后缀` 查找，后缀按配置顺序并同时尝试大写变体（兼容 Windows 导出）。
  配置为空时回退到默认后缀 `jpg / jpeg / png`；找不到文件画空框便于排查，绝不 panic。
- **非法颜色**按默认值处理（填色非法即视为无填充），单个元素出错不影响整份文档。

新增照片元素的三个配置：`photo_dir`（目录，可由系统对话框选择）、
`photo_name`（名称模板，字段 + 固定文本组合）、`photo_exts`（候选后缀，多选）。

### 4.10 项目字段复制与作用域（`routes/project_fields.rs::copy_fields`）

`POST /api/project-fields/copy` 接收 `{ project_id, field_ids }`，把全局字段复制为本项目字段：

- 整个复制在 `db::with_tx` 事务中执行，任一条失败则整体回滚；
- 逐 id 从 `standard_fields` 读取名称/类型/精度等，写入 `project_fields` 时带上
  `source_id`（原全局字段 id）作为溯源，`project_id` 换成本项目；
- 使用 `INSERT ... ON CONFLICT(project_id, name) DO NOTHING`：
  **同项目中同名时跳过而不是报错**，因此重复点击「从全局添加」结果是幂等的，
  返回本次实际新增的条数；未知 `field_ids` 直接忽略；
- **别名单一并复制**：按 `alias_mappings.standard = 字段名` 取出全局别名，
  写入 `project_alias_mappings`（`UNIQUE(project_id, alias)`，冲突跳过），
  因此项目字段卡片打开时就与全局别名页完全一致；
- 复制后项目字段拥有独立主键，后续改名/改类型/删除只影响本项目，全局字典不变；
  项目字段改名时同步更新 `project_alias_mappings.standard`，删除字段时连带删除其项目别名；
- 项目别名通过 `GET /api/project-aliases?project_id=` 列出，
  `POST /api/project-aliases` 新增/更新（只允许指向本项目已有字段），
  `DELETE /api/project-aliases/:id` 删除；
- 导入时的列映射建议（`store::suggest_columns(conn, names, project_id)`）在
  `project_id` 非空时只匹配本项目字段与**项目别名**，全局字段/全局别名不会穿透，
  因此项目数据集不会隐式引用全局字段。

---

## 5. 环境变量

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `EXAM_DB` | `exam_data.db` | SQLite 数据库文件路径 |
| `EXAM_ADDR` | `127.0.0.1:8710` | 监听地址 |
| `EXAM_DATA_DIR` | 当前目录 `photos/` | 渲染请求未显式传 `photo_dir` 时的兜底照片目录 |

---

## 6. 测试与质量门禁

```bash
cargo test          # 单元测试：事务回滚、连接池 panic 安全、导入边界、照片解析、表达式/流程编译等
cargo clippy --all-targets
cargo fmt
```

单元测试覆盖的核心路径：数据集增删读与分页越界、事务失败回滚、表名注入拦截、
列名去重、字段匹配（精确/别名/未知）、流程编译与运行、节点数据预览与越界校验、
照片后缀查找、模板照片元素迁移、dtype 白名单。

---

## 7. 扩展指引

- **新增接口**：在对应 `routes/*.rs` 加 handler → 在 `routes/mod.rs` 注册 →
  handler 内用 `state.db` 包裹数据库操作、写操作用 `db::with_tx`。
- **新增流程操作符**：在 `engine/operators.rs` 的 `catalog` 增加定义（含参数 schema），
  在 `engine/mod.rs` 的编译分支实现 SQL 生成，前端无需改动即可渲染参数表单。
- **新增模板元素类型**：在 `models::TemplateElement` 增加字段 → `pdfgen` 的
  `Painter::element` 增加绘制分支 → 前端 `templateModel` 增加默认值与面板。
