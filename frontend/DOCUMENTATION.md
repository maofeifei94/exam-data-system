# 前端设计文档（exam-data-frontend）

考试数据处理系统的桌面端界面：React 18 + TypeScript + Vite，
渲染成 Electron 桌面应用（无边框窗口 + 自定义标题栏）。

本文件说明**每个文件负责什么**、**数据模型长什么样**、**坐标/布局等算法怎么算**，
方便定位问题和二次开发。

---

## 1. 总体架构

```
Electron 主进程 electron/main.cjs
   │  注入 window.examDesktop（窗口控制、原生文件对话框）
   │  注入 window.examapi.baseUrl（后端绝对地址）
   ▼
React 应用 src/main.tsx → App.tsx
   │
   ├─ 顶栏菜单 / 左侧边栏 / 底部状态栏（App.tsx）
   ├─ ProjectProvider（全局「当前考试项目」上下文）
   └─ 页面 pages/*
          │  纯函数逻辑下沉到 lib/*
          │  网络请求统一走 api/client.ts
          ▼
      后端 http://127.0.0.1:8710/api
```

三条约定：

1. **网络请求只走 `api/client.ts`**。页面不直接 `fetch`，统一获得超时、错误码、
   二进制下载等处理。
2. **可测试的纯逻辑放 `lib/`**，页面只做状态与交互；`lib/*` 都有对应的 `*.test.ts`。
3. **颜色/主题只用 CSS 变量**（`styles.css` 定义，`theme.tsx` 切换 light/dark）。

---

## 2. 目录与文件清单

### 2.1 入口与全局

| 文件 | 职责 |
| --- | --- |
| `src/main.tsx` | React 挂载入口。 |
| `src/App.tsx` | 应用外壳：顶栏菜单栏（含「选项」下拉）、项目切换工具条、左侧活动栏、编辑区、状态栏、窗口控制按钮；路由到 7 个页面；5 秒轮询 `/api/health` 显示后端连接状态。 |
| `src/theme.tsx` | 明暗主题上下文 `ThemeProvider` / `useThemeMode`，持久化到 localStorage，切换 `<html>` 上的主题属性。 |
| `src/context/UiSettingsContext.tsx` | **外观选项**上下文 `UiSettingsProvider` / `useUiSettings`：`tableCenter`（表格内容居中，默认 true）、`tableBorder`（表格边框，默认 true）；持久化 `localStorage['exam.ui.settings.v1']`，解析失败回退默认值。 |
| `src/styles.css` | 全量样式（VS Code 风格的布局与组件视觉），颜色全部走 CSS 变量。 |
| `src/context/ProjectContext.tsx` | 「当前考试项目」全局上下文：项目列表、当前选中项目、切换、刷新、局部更新。选中项持久化到 `localStorage`。 |

### 2.2 网络层

| 文件 | 职责 |
| --- | --- |
| `src/api/client.ts` | **统一请求层**。封装 `apiRequest` / `apiRequestBlob`，导出全部后端接口函数；超时、`AbortSignal`、错误码透传。 |
| `src/api/types.ts` | 与后端一一对应的 TypeScript 类型；`normalizeDtype` 兼容历史 dtype；模板元素的照片字段定义。 |
| `src/api/client.test.ts` | 请求层单元测试（成功解析、错误码、非 JSON 错误页、204、超时、取消等）。 |
| `src/lib/native.ts` | 桌面能力封装：窗口控制、`pickFolder`（系统文件夹选择）、`revealFolder`；浏览器环境优雅降级返回 null。 |

**请求层关键行为**（`client.ts`）：

- 普通接口 30 秒超时，导入 / 流程运行 / PDF 渲染 / CSV 导出等长任务 5 分钟；
- 失败分类为 `timeout` / `aborted` / `network_error` / `invalid_response`，并保留后端 `code`；
- 错误响应不是 JSON 时不把 HTML 错误页内容透给用户；
- 兼容 204 与空响应体，避免空字符串触发 JSON 解析异常；
- 支持外部 `AbortSignal`，请求结束后清理监听器与定时器。

### 2.3 通用组件

| 文件 | 职责 |
| --- | --- |
| `src/components/Modal.tsx` | 轻量模态层（遮罩 + 卡片）。 |
| `src/components/DialogWindow.tsx` | 带标题栏的窗口式对话框（可拖动感、关闭按钮）。 |
| `src/components/Toast.tsx` | 全局提示 `ToastProvider` / `useToast`，成功/错误消息统一出口。 |
| `src/components/Spinner.tsx` | 加载指示器。 |
| `src/components/EmptyState.tsx` | 空状态占位。 |
| `src/components/Field.tsx` | 表单字段外壳（标签 + 提示）。 |
| `src/components/DataTable.tsx` | 数据表格：自动识别数值列右对齐、空值占位、单元格格式化；根据 `UiSettings` 追加 `align-center` / `bordered` class（默认居中 + 带边框，可在「选项」中关闭）。 |
| `src/components/NewProjectDialog.tsx` | 新建考试项目对话框（名称、说明、文件夹选择）。 |
| `src/components/TokenEditor.tsx` | **字段胶囊编辑器**：展示态把 `{{字段}}`/`[字段]` 渲染成彩色胶囊，双击进入编辑态；支持 template / expr 两种模式。 |

### 2.4 可测试的纯逻辑 `src/lib/`

| 文件 | 职责 |
| --- | --- |
| `src/lib/templateModel.ts` | **PDF 模板的几何与元素树模型**（详见第 4 节）。 |
| `src/lib/flowModel.ts` | 流程图画布布局算法：主链 / 分支节点坐标计算、相对插入、分支兄弟节点。 |
| `src/lib/tokens.ts` | 字段胶囊解析与配色：表达式 token、模板 token、字段配色、hex→rgba。 |
| `src/lib/projectFields.ts` | `projectFieldsAsStandard(projectFields)`：把项目本地字段转换成标准字段形状。**不做全局合并**——项目内只能引用本项目字段，全局字段需先复制进项目。 |
| `src/lib/native.ts` | 见上（桌面能力）。 |
| `*.test.ts` | 上述模块的单元测试（vitest）。 |

### 2.5 页面 `src/pages/`

| 文件 | 职责 |
| --- | --- |
| `ProjectTemplatesPage.tsx` | **项目模板**：左侧栏「项目文件」之前的栏目，默认空。分「字段模板 / PDF 模板」两个 tab。字段以卡片展示（与全局「字段别名」页一致：卡片内联改名/改类型/改小数位/改说明），可从全局字段库勾选复制、移除；PDF 模板可从全局复制或新建空白。保存结果全部落在本项目（`project_fields` / project scope 模板），不依赖全局。 |
| `ProjectFilesPage.tsx` | 项目文件总览：项目信息、关联数据集、快捷入口（导入 / 流程 / 模板）。 |
| `ImportPage.tsx` | 数据导入：顶部横向步骤条（选择文件 → 数据预览 → 字段映射 → 完成），每步独立面板、内部滚动，底部固定上一步/下一步按钮，避免长表单溢出窗口。 |
| `ProjectFlowPage.tsx` | 处理流程编辑器：画布节点、两级参数表单、静态校验、运行与运行日志。 |
| `DataFlowPage.tsx` | **数据流**：左侧按 `branch_of` 还原的节点多级目录，右侧查看该节点数据表；终点节点展示 PDF 预览与下载。 |
| `AliasPage.tsx` | 字段别名与标准字段管理：分组卡片、拖拽换组、改名、增删别名。别名胶囊共用同一前缀「别名：」，其后并列展示全部别名标签（不重复前缀、不加分隔符）；`InlineSelect` / `PrecisionInput` 也供项目模板卡片复用。 |
| `TemplatePage.tsx` | PDF 模板设计器：页面/矩阵设置、元素树、画布拖拽缩放、属性面板、预览与渲染导出。 |

---

## 3. 数据结构（与后端对齐）

### 3.1 字段类型 `ColumnDtype`

```
text | integer | decimal | percent | date | datetime | boolean
```

- 需要「精度」的类型是 `decimal` / `percent`（`PRECISION_DTYPES`）。
- `normalizeDtype` 兼容历史数据：`number → decimal`，`image → text`。
- **照片不是数据类型**。照片文件名按文本存储，文件解析由 PDF 模板的照片元素负责。

### 3.2 模板正文

```
PdfTemplate
  ├─ kind: 'single' | 'matrix'
  ├─ page:  PageSetup   { size, orientation, margin_mm }
  ├─ matrix: MatrixSetup{ rows, cols, cell_h_mm, origin_y_mm, show_grid, show_border, gap_* }
  └─ elements: TemplateElement[]   递归树
```

`TemplateElement` 关键字段：

| 字段 | 含义 |
| --- | --- |
| `type` | `text`/`field`/`image`/`line`/`rect`/`matrix` |
| `x_mm/y_mm/w_mm/h_mm` | 相对父元素左上角的坐标与尺寸（毫米） |
| `children` | 子元素；矩阵的子元素会被复制到每个单元格 |
| `matrix` | `type=matrix` 时的网格配置 |
| `photo_dir` | `type=image`：照片目录（可由系统对话框选择） |
| `photo_name` | `type=image`：照片名模板，支持 `{{字段}}` 与固定文本组合 |
| `photo_exts` | `type=image`：候选后缀，按顺序查找，如 `['jpg','png']` |

### 3.3 流程

```
Flow { steps: FlowStep[], scope: 'global'|'project', project_id, source_id }
FlowStep { id, operator, label, enabled, params, pos?, branch?, branch_of? }
```

- `pos` 是画布坐标，缺省时由 `flowModel.computeLayout` 自动排布；
- `branch` 标识可视分支组，`branch_of` 指明挂在哪个步骤之后。

### 3.4 节点数据快照 `FlowStepData`

```ts
FlowStepData extends RowPage {
  step_index: number;  // 0 基下标；等于 steps.length 时表示流程最终输出
  label: string;       // 节点显示名
  operator: string;    // 操作符类型；终点为 'output'
}
```

对应后端 `StepData`，由 `getFlowStepRows(flowId, stepIndex, datasetId, offset, limit)` 获取。
它是**一次性快照**：查看节点不会生成新数据集。

---

## 4. 关键算法

### 4.1 模板坐标系与拖拽（`templateModel.ts`）

**坐标系**（与后端 `pdfgen` 完全一致）：

- 根元素坐标相对「页面内容区」（页面去掉页边距后的区域）左上角；
- 子元素相对「父元素左上角」；
- 矩阵的 `children` 相对「单元格左上角」。

**拖动顶点缩放** `resizeRect(box, handle, dx, dy, minW, minH)`：

- 拖动某个顶点时，**对角点保持不动**，相邻两条边随顶点移动，矩形始终轴对齐；
- 宽高不小于最小尺寸；触底时固定对边，避免继续拖动导致矩形翻转。

**相对坐标换算** `toParentBox(located, box)`：

- `walkElements` 遍历时把父元素偏移累加，得到元素的**页面绝对坐标**；
- 缩放提交时先减去父元素绝对原点，把绝对矩形换算回元素自身的相对坐标。
  否则嵌套元素（矩形框内、矩阵单元格内）会在松手瞬间跳位。

该换算逻辑（含矩阵子元素按第一个单元格计算）有单元测试覆盖。

### 4.2 元素树操作

| 函数 | 作用 |
| --- | --- |
| `walkElements` | 深度优先遍历，累加父偏移得到绝对坐标 |
| `findElement` | 返回元素 + 父元素 + 绝对坐标 |
| `findPath` | 从根到目标的路径（面包屑） |
| `updateElement` | 不可变地深度更新任意层级元素 |
| `removeElement` | 删除元素及其子树，保留其他节点 |
| `appendChild` | 在指定容器（null = 页面根）追加子元素 |
| `isContainer` | 判断元素能否容纳子元素（rect / matrix） |
| `cloneElementWithNewIds` | 复制元素并为整棵子树重新生成 id |

### 4.3 兼容旧数据的归一化

- `normalizeTree`：补全 `children`、收紧旧版矩阵宽高（旧版把间距算进了宽高，新版是连续栅格）；
  对 `image` 元素做迁移——旧模板用 `field` 绑定单个字段，新结构转换为
  `photo_name = '{{字段}}'` 并补全默认后缀。
- `normalizeTemplate`：把「扁平元素 + 页面级矩阵」的旧结构迁移成嵌套元素树
  （页面标题留在根，单元格元素挂到 matrix 元素下）。

### 4.4 流程图布局（`flowModel.ts`）

- 主链步骤按固定节点宽高与纵向间距自上而下排布；
- 分支步骤在主链节点右侧按分支序号水平展开（`BRANCH_GAP_X`）；
- `computeLayout` 返回每个步骤的坐标映射 + PDF 节点位置；
- `insertStepAt` / `insertRelativeStep` 支持在指定位置或某步骤前后插入。

### 4.5 字段胶囊解析（`tokens.ts`）

- `splitExprTokens` 解析表达式模式 `[字段]`；
- `splitTemplateTokens` 解析模板模式 `{{字段}}` 与内置变量；
- `fieldColor` 由字段名哈希到固定调色板，同一字段颜色稳定；
- `TokenEditor` 展示态渲染胶囊、编辑态显示原文，未知字段标红提示。

### 4.6 照片元素的三项配置（本次新增）

属性面板中，选中 `image` 元素后可配置：

1. **照片目录**：文本框 + 「选择…」按钮，桌面端调用 `pickFolder()` 弹出系统文件夹选择窗口，
   浏览器环境提示手动填写；
2. **照片名**：用 `TokenEditor`（template 模式）编辑，可组合数据字段与固定文本，
   例如 `zp_{{身份证号}}`；
3. **照片后缀**：多选下拉，按选择顺序作为文件查找优先级。

这些配置写入元素的 `photo_dir` / `photo_name` / `photo_exts`，
渲染时由后端 `pdfgen::resolve_photo` 按「原样路径 → 目录+名称.后缀 → 大写后缀」依次查找。

### 4.7 节点树还原与数据流查看（`DataFlowPage.tsx`）

**树还原** `buildStepTree(steps)`：

- 以 `branch_of` 为父指针，把线性的 `steps` 还原成多级目录；没有父节点或父节点为空的
  步骤挂在主链根上；
- 用 `visited` 集合防环（`branch_of` 指向自身或指向已访问节点时按根处理），
  避免脏数据导致无限递归；
- 返回 `{ step, children }[]`，渲染时按 `depth` 递增左缩进并显示折叠按钮与序号徽标。

**节点与数据源的映射**：

| 选择 | 数据来源 |
| --- | --- |
| 起点「数据导入」 | `getDatasetRows(project.dataset_id, offset, 50)` |
| 中间节点（第 i 步） | `getFlowStepRows(flow.id, i, dataset_id, offset, 50)` |
| 终点「PDF 输出」 | `getFlowStepRows(flow.id, flow.steps.length, ...)` + PDF 预览卡片 |

- `step_index` 与 `steps` 的**原始下标**一致（禁用步骤也占位），因此树的顺序与后端编译顺序对齐；
- 分页统一 `PAGE_SIZE = 50`，切换节点时重置 `offset`；
- 终点卡片：先 `runFlow()` 拿到 `output_dataset_id`，再 `renderPdfTemplate(template.id, output_dataset_id)`
  得到 PDF Blob，用 `URL.createObjectURL` 交给 `<iframe>` 预览，并提供下载按钮；
  组件卸载或重新生成时 `URL.revokeObjectURL` 释放内存。

### 4.8 导入向导（`ImportPage.tsx`）

旧的「一页铺开全流程」在小窗口下会把第三步字段映射挤出可视区，导致按钮点不到。
现改为单步向导：

- `STEPS = ['选择文件','数据预览','字段映射','完成']`，`step` 为当前步、`maxStep` 为已解锁的最大步；
  点击步骤条只能跳到 `<= maxStep` 的步骤，避免跳过未完成的校准；
- 布局：`.page > .import-wizard.page-fill`，内部 `Steps` + `.import-body`（`flex:1; overflow:auto`）
  + `.import-footer`（固定按钮区），因此**任何窗口高度下按钮都可见**；
- 步骤 0 选择文件（名称、表头行）→ 上传成功后进入步骤 1 预览；
- 步骤 2 映射表使用 `data-table align-center bordered`，列出「原名 / 建议标准字段 / 保存为新别名」，
  必要字段以 chips 展示；点击「应用映射」后进入步骤 3；
- 步骤 3 展示导入结果行数并向父组件回调 `onImported(dataset)`，「再导入一个文件」调用 `reset()`。
- 字段候选只取 `listProjectFields(projectId)`；全局字段必须先复制进本项目才会出现。
  后端导入建议同样按 `project_id` 作用域，避免数据集隐式引用全局字段。

### 4.9 项目模板的复制与作用域

项目模板页（`ProjectTemplatesPage.tsx`）把全局资源「复制」为本项目资源，复制后完全本地可控：

| 资源 | 复制来源 | 落地位置 |
| --- | --- | --- |
| 字段 + 别名 | `listStandardFields()` 勾选 → `copyProjectFields(projectId, ids)` | `project_fields`（`UNIQUE(project_id, name)`）+ `project_alias_mappings`（字段的全局别名单整体复制） |
| PDF 模板 | `listPdfTemplates({scope:'global'})` → `duplicatePdfTemplate(id, {project_id})` | `pdf_templates` 中 `scope='project'` 的新行 |

- 项目字段卡片与全局「字段别名」页使用同一套卡片样式（`alias-card` / `alias-name-input` /
  `alias-type-combo` / `alias-chips` / `alias-add`），颜色与交互一致；
- 编辑项目字段：卡片内联修改名称/类型/小数位，别名可增删（`saveProjectAlias` /
  `deleteProjectAlias`），`updateProjectField` 只改本项目副本；
- 移除项目字段：仅删本地副本，不影响全局与其他项目；
- 「新建空白模板」通过 `TemplatePage` 的 `initialNew` 参数打开空白草稿，保存时 `scope='project'`；
- 项目内只使用 `project_fields`，全局字段不会自动出现（`TemplatePage` 仅在全局作用域时使用全局字段库）。

### 4.10 外观选项（`选项` 菜单）

顶栏「选项」下拉与 `UiSettings` 联动，勾选项以 `✓` 前缀表示当前状态：

| 选项 | 默认 | 效果 |
| --- | --- | --- |
| 表格内容居中 | 开 | `DataTable` 追加 `align-center`；关闭后表头/单元格回到左对齐 |
| 显示表格边框 | 开 | 追加 `bordered`（`1px solid var(--border)`）；关闭后移除全部单元格边框 |
| 恢复默认外观 | — | `reset()` 恢复两个开关为默认值 |

配置仅存于 `localStorage`，不写后端；刷新/重启后保持。

---

## 5. 状态与数据流

- **当前项目**：`ProjectProvider` 管理，页面通过 `useProject()` 读取；
  项目切换会触发各页面重新拉取该项目的流程 / 数据集 / 模板。
- **全局配置入口**：字段别名、PDF 模板库从顶栏「全局配置」菜单进入；左侧边栏只保留
  项目模板、项目文件、项目流程、数据流四个视图（`SIDEBAR_VIEWS`），「项目模板」在「项目文件」之前。
- **外观选项**：顶栏「选项」菜单读写 `UiSettingsProvider`，只影响显示层。
- **提示消息**：所有成功/失败反馈统一走 `useToast()`。
- **后端状态**：App 每 5 秒轮询 `/api/health`，状态栏显示「后端已连接 / 未连接」。

---

## 6. 构建、测试与打包

```bash
npm run dev        # Vite 开发服务器（/api 代理到后端）
npm test           # vitest 单元测试
npm run build      # tsc --noEmit 类型检查 + vite 生产构建
npm run electron   # 启动 Electron 桌面应用
npm run dist       # electron-builder 打包（Windows NSIS / Linux AppImage）
```

质量门禁：`npm run build` 必须通过（含 `tsc --noEmit`），`npm test` 全绿。

单元测试覆盖的重点：模板坐标换算与缩放、照片元素迁移与默认值、
流程图布局算法、请求层的错误与边界处理。

---

## 7. 扩展指引

- **新增页面**：在 `src/App.tsx` 增加 `ViewKey` 与 `VIEWS`，写页面组件，
  页面内只通过 `api/client.ts` 访问后端。
- **新增模板元素类型**：在 `api/types.ts` 的 `TemplateElement` 与 `lib/templateModel.ts`
  的 `createElement` / `normalizeTree` / `elementLabel` 增加分支，在
  `TemplatePage.tsx` 的画布渲染与属性面板补充对应 UI；后端同步在 `pdfgen` 增加绘制分支。
- **新增请求**：在 `api/client.ts` 增加导出函数（普通接口用 `apiRequest`，
  返回文件用 `apiRequestBlob`），并补一条 `client.test.ts` 用例。
