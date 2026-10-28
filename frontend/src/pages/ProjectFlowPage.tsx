import { useEffect, useMemo, useRef, useState } from 'react';
import type { DragEvent as ReactDragEvent, PointerEvent as ReactPointerEvent } from 'react';
import { Dropdown } from 'antd';
import type { MenuProps } from 'antd';
import {
  createFlow,
  duplicatePdfTemplate,
  listDatasets,
  listFlows,
  listOperators,
  listPdfTemplates,
  listProjectFields,
  listStandardFields,
  renderPdfTemplate,
  runFlow,
  updateFlow,
  updateProject,
  validateFlow,
} from '../api/client';
import type {
  Dataset,
  ExamProject,
  FlowStep,
  FlowValidateResult,
  OperatorDef,
  OperatorParam,
  PdfTemplate,
  RunResult,
  StandardField,
  StepFlow,
} from '../api/types';
import { DialogWindow } from '../components/DialogWindow';
import { EmptyState } from '../components/EmptyState';
import { Spinner } from '../components/Spinner';
import { useToast } from '../components/Toast';
import { useProject } from '../context/ProjectContext';
import { mergeFieldLists } from '../lib/projectFields';
import { TokenEditor } from '../components/TokenEditor';
import {
  NODE_H as FLOW_NODE_H,
  NODE_W as FLOW_NODE_W,
  PAD as FLOW_PAD,
  computeLayout,
  insertRelativeStep,
  insertStepAt,
  isMainStep,
} from '../lib/flowModel';
import type { InsertMode, Point } from '../lib/flowModel';
import ImportPage from './ImportPage';
import TemplatePage from './TemplatePage';

// 节点尺寸与间距（像素）
const NODE_W = FLOW_NODE_W;
const NODE_H = FLOW_NODE_H;
const PAD = FLOW_PAD;

const GROUP_COLORS: Record<string, string> = {
  校验: '#b45309',
  字段: '#0e7490',
  计算: '#0f766e',
  行: '#7c3aed',
  输出: '#334155',
};

function groupColor(group: string | undefined): string {
  return (group && GROUP_COLORS[group]) || '#475569';
}

function runStatusColor(status: string): string {
  if (status === 'ok') return '#16a34a';
  if (status === 'failed') return '#dc2626';
  return '#94a3b8';
}

function defaultParamsFor(op: OperatorDef | undefined): Record<string, unknown> {
  const params: Record<string, unknown> = {};
  if (!op) return params;
  for (const p of op.params) {
    if (p.default !== undefined && p.default !== null) {
      params[p.key] = p.default;
    } else if (p.type === 'columns' || p.type === 'fields' || p.type === 'mappings') {
      params[p.key] = [];
    } else if (p.type === 'bool') {
      params[p.key] = false;
    } else if (p.type === 'number') {
      params[p.key] = 0;
    } else {
      params[p.key] = '';
    }
  }
  return params;
}

function newStepId(): string {
  return `step_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`;
}

interface FlowDraft {
  id: string | null;
  name: string;
  description: string;
  steps: FlowStep[];
}

function edgePath(from: Point, to: Point): string {
  const x1 = from.x + NODE_W / 2;
  const y1 = from.y + NODE_H;
  const x2 = to.x + NODE_W / 2;
  const y2 = to.y;
  const dir = y2 >= y1 ? 1 : -1;
  const bend = Math.min(60, Math.max(24, Math.abs(y2 - y1) / 2));
  return `M ${x1} ${y1} C ${x1} ${y1 + dir * bend}, ${x2} ${y2 - dir * bend}, ${x2} ${y2}`;
}

/** 从父节点右侧连到分支节点左侧，用于横向分支。 */
function branchEdgePath(from: Point, to: Point): string {
  const x1 = from.x + NODE_W;
  const y1 = from.y + NODE_H / 2;
  const x2 = to.x;
  const y2 = to.y + NODE_H / 2;
  const bend = Math.max(30, Math.abs(x2 - x1) / 2);
  return `M ${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}`;
}

function saveBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = filename;
  document.body.appendChild(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
}

export default function ProjectFlowPage() {
  const { show } = useToast();
  const { currentProject, currentProjectId, patchCurrentProject, refresh } = useProject();

  const [operators, setOperators] = useState<OperatorDef[]>([]);
  const [datasets, setDatasets] = useState<Dataset[]>([]);
  const [standardFields, setStandardFields] = useState<StandardField[]>([]);
  const [projectTemplates, setProjectTemplates] = useState<PdfTemplate[]>([]);
  const [globalTemplates, setGlobalTemplates] = useState<PdfTemplate[]>([]);

  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [running, setRunning] = useState(false);

  const [draft, setDraft] = useState<FlowDraft>({ id: null, name: '', description: '', steps: [] });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [runResult, setRunResult] = useState<RunResult | null>(null);
  const [validateResult, setValidateResult] = useState<FlowValidateResult | null>(null);
  /** 同时只打开一个节点的插入菜单，选中菜单项后立即关闭，避免子菜单残留 */
  const [addMenu, setAddMenu] = useState<{ stepId: string; mode: InsertMode } | null>(null);

  const [importOpen, setImportOpen] = useState(false);
  const [templateOpen, setTemplateOpen] = useState(false);
  const [exporting, setExporting] = useState(false);

  const [canvasSize, setCanvasSize] = useState({ w: 760, h: 520 });
  const hostRef = useRef<HTMLDivElement | null>(null);

  const operatorMap = useMemo(() => {
    const map = new Map<string, OperatorDef>();
    operators.forEach((op) => map.set(op.type, op));
    return map;
  }, [operators]);

  const operatorGroups = useMemo(() => {
    const groups = new Map<string, OperatorDef[]>();
    operators.forEach((op) => {
      const list = groups.get(op.group) ?? [];
      list.push(op);
      groups.set(op.group, list);
    });
    return [...groups.entries()];
  }, [operators]);

  const selectedStep = draft.steps.find((step) => step.id === selectedId) ?? null;
  const selectedIndex = selectedStep ? draft.steps.findIndex((step) => step.id === selectedStep.id) : -1;
  const isImportSelected = selectedId === 'anchor:import';
  const isPdfSelected = selectedId === 'anchor:pdf';

  const boundDataset = datasets.find((dataset) => dataset.id === currentProject?.dataset_id) ?? null;
  const boundTemplate =
    projectTemplates.find((template) => template.id === currentProject?.pdf_template_id) ??
    globalTemplates.find((template) => template.id === currentProject?.pdf_template_id) ??
    null;
  const runDataset = boundDataset ?? datasets[0] ?? null;

  const availableColumns = useMemo(() => {
    const names = new Set<string>();
    runDataset?.columns.forEach((column) => names.add(column.name));
    standardFields.forEach((field) => names.add(field.name));
    return [...names];
  }, [runDataset, standardFields]);

  // 必要字段：流程中所有「必需字段校验」声明的字段，作为数据流推演的起点。
  const requiredFields = useMemo(() => {
    const names: string[] = [];
    draft.steps.forEach((step) => {
      if (!step.enabled || step.operator !== 'require_fields') return;
      const fields = Array.isArray(step.params?.fields) ? step.params.fields : [];
      fields.forEach((field) => {
        const name = String(field).trim();
        if (name && !names.includes(name)) names.push(name);
      });
    });
    return names;
  }, [draft.steps]);

  const startFields = requiredFields.length
    ? requiredFields
    : runDataset?.columns.map((column) => column.name) ?? [];
  const lastStepFlow = validateResult?.steps?.[validateResult.steps.length - 1];
  const endFields = lastStepFlow?.output_fields ?? startFields;

  const flowByStep = useMemo(() => {
    const map = new Map<number, StepFlow>();
    validateResult?.steps.forEach((flow) => map.set(flow.step_index, flow));
    return map;
  }, [validateResult]);

  const validateErrors = validateResult?.errors ?? [];

  // 任一环节增删改后立即重新推演数据流，保持流程图上的字段快照实时更新。
  useEffect(() => {
    if (!currentProjectId) return;
    let active = true;
    const timer = window.setTimeout(async () => {
      try {
        const base = requiredFields.length
          ? requiredFields
          : (runDataset?.columns.map((column) => column.name) ?? []);
        const result = await validateFlow(draft.steps, base);
        if (active) setValidateResult(result);
      } catch {
        // 校验失败时保留上一次结果，不打断编辑
      }
    }, 350);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draft.steps, requiredFields, runDataset, currentProjectId]);

  const layout = useMemo(() => computeLayout(draft.steps), [draft.steps]);
  const { positions, pdfPos } = layout;

  // 新增分支可能落在画布右侧，把选中的新节点滚动到可视区，避免“插入成功但看不见”。
  useEffect(() => {
    if (!selectedId) return;
    const host = hostRef.current;
    const p = positions.get(selectedId);
    if (!host || !p) return;
    const right = p.x + NODE_W;
    if (right > host.scrollLeft + host.clientWidth) {
      host.scrollLeft = right - host.clientWidth + PAD;
    } else if (p.x < host.scrollLeft) {
      host.scrollLeft = Math.max(0, p.x - PAD);
    }
    const bottom = p.y + NODE_H;
    if (bottom > host.scrollTop + host.clientHeight) {
      host.scrollTop = bottom - host.clientHeight + PAD;
    } else if (p.y < host.scrollTop) {
      host.scrollTop = Math.max(0, p.y - PAD);
    }
  }, [selectedId, positions]);

  const canvasSizeComputed = useMemo(() => {
    let maxX = canvasSize.w;
    let maxY = pdfPos.y + NODE_H + PAD;
    draft.steps.forEach((step) => {
      const p = positions.get(step.id);
      if (!p) return;
      maxX = Math.max(maxX, p.x + NODE_W + PAD);
      maxY = Math.max(maxY, p.y + NODE_H + PAD);
    });
    return { w: maxX, h: maxY };
  }, [draft.steps, canvasSize.w, pdfPos, positions]);

  // ------------------------------------------------------------------
  // 加载基础数据与项目流程
  // ------------------------------------------------------------------
  useEffect(() => {
    if (!currentProjectId) {
      setLoading(false);
      return;
    }
    let active = true;
    (async () => {
      setLoading(true);
      try {
        const [ops, datasetList, fields, projectFields, flows, projectTpl, globalTpl] =
          await Promise.all([
            listOperators(),
            listDatasets(currentProjectId),
            listStandardFields(),
            listProjectFields(currentProjectId),
            listFlows({ projectId: currentProjectId }),
            listPdfTemplates({ projectId: currentProjectId }),
            listPdfTemplates({ scope: 'global' }),
          ]);
        if (!active) return;
        setOperators(ops);
        setDatasets(datasetList);
        setStandardFields(mergeFieldLists(fields, projectFields));
        setProjectTemplates(projectTpl);
        setGlobalTemplates(globalTpl);

        const flow = flows[0];
        if (flow) {
          setDraft({
            id: flow.id,
            name: flow.name,
            description: flow.description,
            steps: flow.steps,
          });
        } else {
          setDraft({
            id: null,
            name: `${currentProject?.name ?? '项目'}处理流程`,
            description: '',
            steps: [],
          });
        }
        setSelectedId(null);
        setRunResult(null);
        setValidateResult(null);
      } catch (err) {
        show(err instanceof Error ? err.message : '加载流程数据失败', 'error');
      } finally {
        if (active) setLoading(false);
      }
    })();
    return () => {
      active = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentProjectId]);

  // 画布尺寸测量
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const measure = () => setCanvasSize({ w: host.clientWidth || 760, h: host.clientHeight || 520 });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(host);
    return () => observer.disconnect();
  }, [loading]);

  // ------------------------------------------------------------------
  // 草稿操作
  // ------------------------------------------------------------------
  const buildStep = (type: string, pos?: Point): FlowStep => {
    const def = operatorMap.get(type);
    return {
      id: newStepId(),
      operator: type,
      label: def?.label ?? type,
      enabled: true,
      params: defaultParamsFor(def),
      pos,
    };
  };

  const addStep = (type: string, index?: number, pos?: Point) => {
    const step = buildStep(type, pos);
    setDraft((prev) => ({ ...prev, steps: insertStepAt(prev.steps, step, index) }));
    setSelectedId(step.id);
    setAddMenu(null);
  };

  /** 载入某个节点的「前 / 后 / 分支」插入位置。 */
  const insertRelative = (step: FlowStep, type: string, mode: InsertMode) => {
    const created = buildStep(type);
    setDraft((prev) => ({ ...prev, steps: insertRelativeStep(prev.steps, created, step.id, mode) }));
    setSelectedId(created.id);
    setAddMenu(null);
  };

  const updateStep = (id: string, patch: Partial<FlowStep>) => {
    setDraft((prev) => ({
      ...prev,
      steps: prev.steps.map((step) => (step.id === id ? { ...step, ...patch } : step)),
    }));
  };

  const updateStepParam = (id: string, key: string, value: unknown) => {
    setDraft((prev) => ({
      ...prev,
      steps: prev.steps.map((step) =>
        step.id === id ? { ...step, params: { ...step.params, [key]: value } } : step,
      ),
    }));
  };

  const removeStep = (id: string) => {
    setDraft((prev) => ({ ...prev, steps: prev.steps.filter((step) => step.id !== id) }));
    if (selectedId === id) setSelectedId(null);
    setRunResult(null);
  };

  const moveStep = (id: string, delta: number) => {
    setDraft((prev) => {
      const index = prev.steps.findIndex((step) => step.id === id);
      if (index < 0) return prev;
      const target = index + delta;
      if (target < 0 || target >= prev.steps.length) return prev;
      const next = [...prev.steps];
      const [item] = next.splice(index, 1);
      next.splice(target, 0, item);
      return { ...prev, steps: next };
    });
  };

  const autoLayout = () => {
    setDraft((prev) => ({
      ...prev,
      steps: prev.steps.map((step) => ({ ...step, pos: undefined })),
    }));
    show('已按顺序重新排版', 'success');
  };

  const startDrag = (event: ReactPointerEvent<HTMLDivElement>, step: FlowStep) => {
    event.preventDefault();
    setSelectedId(step.id);
    const startX = event.clientX;
    const startY = event.clientY;
    const origin = positions.get(step.id) ?? { x: 110, y: PAD };

    const onMove = (ev: PointerEvent) => {
      const nx = origin.x + (ev.clientX - startX);
      const ny = origin.y + (ev.clientY - startY);
      setDraft((prev) => ({
        ...prev,
        steps: prev.steps.map((s) =>
          s.id === step.id ? { ...s, pos: { x: Math.max(0, nx), y: Math.max(0, ny) } } : s,
        ),
      }));
    };
    const onUp = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  };

  const handleDrop = (event: ReactDragEvent<HTMLDivElement>) => {
    event.preventDefault();
    const type = event.dataTransfer.getData('text/operator');
    if (!type) return;
    const host = hostRef.current;
    if (!host) {
      addStep(type);
      return;
    }
    const rect = host.getBoundingClientRect();
    const dropX = event.clientX - rect.left + host.scrollLeft;
    const dropY = event.clientY - rect.top + host.scrollTop;
    let index = draft.steps.length;
    for (let i = 0; i < draft.steps.length; i += 1) {
      const p = positions.get(draft.steps[i].id);
      if (p && dropY < p.y + NODE_H / 2) {
        index = i;
        break;
      }
    }
    addStep(type, index, { x: Math.max(0, dropX - NODE_W / 2), y: Math.max(0, dropY - 20) });
  };

  // ------------------------------------------------------------------
  // 保存 / 绑定
  // ------------------------------------------------------------------
  const bindProject = async (patch: Partial<ExamProject>) => {
    if (!currentProject) return;
    const updated = await updateProject(currentProject.id, {
      name: currentProject.name,
      description: currentProject.description,
      folder: currentProject.folder,
      flow_id: currentProject.flow_id,
      pdf_template_id: currentProject.pdf_template_id,
      dataset_id: currentProject.dataset_id,
      ...patch,
    });
    patchCurrentProject(updated);
    return updated;
  };

  const handleSave = async () => {
    if (!currentProjectId) {
      show('请先选择一个考试项目', 'error');
      return;
    }
    if (!draft.name.trim()) {
      show('请填写流程名称', 'error');
      return;
    }
    setSaving(true);
    try {
      const payload = {
        name: draft.name.trim(),
        description: draft.description,
        steps: draft.steps,
        scope: 'project' as const,
        project_id: currentProjectId,
      };
      const saved = draft.id ? await updateFlow(draft.id, payload) : await createFlow(payload);
      setDraft({ id: saved.id, name: saved.name, description: saved.description, steps: saved.steps });
      if (currentProject && currentProject.flow_id !== saved.id) {
        await bindProject({ flow_id: saved.id });
        await refresh();
      }
      show('项目流程已保存', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '保存流程失败', 'error');
    } finally {
      setSaving(false);
    }
  };

  const handleValidate = async () => {
    if (!runDataset) {
      show('请先在「数据导入」节点绑定数据集', 'error');
      return;
    }
    try {
      const base = requiredFields.length
        ? requiredFields
        : runDataset.columns.map((column) => column.name);
      const result = await validateFlow(draft.steps, base);
      setValidateResult(result);
      show(
        result.ok ? '校验通过，流程可执行' : `校验发现 ${result.errors.length} 个问题`,
        result.ok ? 'success' : 'error',
      );
    } catch (err) {
      show(err instanceof Error ? err.message : '校验失败', 'error');
    }
  };

  const handleRun = async () => {
    if (!draft.id) {
      show('请先保存流程后再运行', 'error');
      return;
    }
    if (!runDataset) {
      show('请先在「数据导入」节点绑定数据集', 'error');
      return;
    }
    setRunning(true);
    setRunResult(null);
    try {
      const result = await runFlow(draft.id, runDataset.id);
      setRunResult(result);
      show(result.message ?? '流程已执行', result.status === 'success' ? 'success' : 'error');
    } catch (err) {
      show(err instanceof Error ? err.message : '运行流程失败', 'error');
    } finally {
      setRunning(false);
    }
  };

  // ------------------------------------------------------------------
  // 数据导入节点
  // ------------------------------------------------------------------
  const bindDataset = async (datasetId: string) => {
    try {
      await bindProject({ dataset_id: datasetId });
      await refresh();
    } catch (err) {
      show(err instanceof Error ? err.message : '绑定数据集失败', 'error');
    }
  };

  const refreshDatasets = async () => {
    if (!currentProjectId) return;
    try {
      setDatasets(await listDatasets(currentProjectId));
    } catch {
      // 刷新失败不打断当前操作
    }
  };

  const handleImported = async (dataset: Dataset) => {
    await refreshDatasets();
    await bindDataset(dataset.id);
    setImportOpen(false);
    show(`已导入并绑定「${dataset.name}」`, 'success');
  };

  // ------------------------------------------------------------------
  // PDF 输出节点
  // ------------------------------------------------------------------
  const bindTemplate = async (templateId: string) => {
    try {
      await bindProject({ pdf_template_id: templateId });
      await refresh();
    } catch (err) {
      show(err instanceof Error ? err.message : '绑定模板失败', 'error');
    }
  };

  const importGlobalTemplate = async (templateId: string) => {
    if (!currentProjectId) return;
    try {
      const copy = await duplicatePdfTemplate(templateId, { project_id: currentProjectId });
      setProjectTemplates((prev) => [copy, ...prev]);
      await bindTemplate(copy.id);
      show(`已引用全局模板「${copy.name}」为本项目副本`, 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '引用模板失败', 'error');
    }
  };

  const refreshTemplates = async () => {
    if (!currentProjectId) return;
    try {
      const list = await listPdfTemplates({ projectId: currentProjectId });
      setProjectTemplates(list);
    } catch {
      // 忽略刷新失败
    }
  };

  const handleTemplateSaved = async (template: PdfTemplate) => {
    await refreshTemplates();
    await bindTemplate(template.id);
  };

  const handleExportPdf = async () => {
    const templateId = currentProject?.pdf_template_id;
    if (!templateId) {
      show('请先在「PDF 输出」节点选择模板', 'error');
      return;
    }
    if (!runDataset) {
      show('请先在「数据导入」节点绑定数据集', 'error');
      return;
    }
    setExporting(true);
    try {
      const blob = await renderPdfTemplate(templateId, runDataset.id);
      saveBlob(blob, `${boundTemplate?.name ?? '输出'}.pdf`);
      show('已生成 PDF', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '生成 PDF 失败', 'error');
    } finally {
      setExporting(false);
    }
  };

  // ------------------------------------------------------------------
  // 节点悬停加号 / 右键多级菜单
  // ------------------------------------------------------------------
  const insertMenu = (prefix: string): MenuProps['items'] =>
    operatorGroups
      .filter(([group]) => group !== '输出')
      .map(([group, list]) => ({
        key: `${prefix}:group_${group}`,
        label: (
          <span className="menu-group-label">
            <span className="palette-dot" style={{ background: groupColor(group) }} />
            {group}
          </span>
        ),
        children: list.map((op) => ({ key: `${prefix}:${op.type}`, label: op.label })),
      }));

  const nodeMenuItems = (): MenuProps['items'] => [
    { key: 'before', label: '在此节点前插入步骤', children: insertMenu('before') },
    { key: 'after', label: '在此节点后插入步骤', children: insertMenu('after') },
    { key: 'branch', label: '添加分支步骤', children: insertMenu('branch') },
    { type: 'divider' },
    { key: 'delete', label: '删除该步骤', danger: true },
  ];

  const handleNodeMenuClick = (step: FlowStep, info: { key: string }) => {
    const [prefix, type] = info.key.split(':');
    if (prefix === 'delete') {
      removeStep(step.id);
      return;
    }
    if (type && (prefix === 'before' || prefix === 'after' || prefix === 'branch')) {
      insertRelative(step, type, prefix);
    }
  };

  /**
   * 节点外侧的插入按钮。菜单采用受控 open：
   * 选中任意菜单项后立即关闭，避免 antd 子菜单浮层残留在画布上。
   */
  const renderAddMenu = (
    step: FlowStep,
    mode: InsertMode,
    className: string,
    symbol: string,
    title: string,
  ) => {
    const open = addMenu?.stepId === step.id && addMenu.mode === mode;
    return (
      <Dropdown
        trigger={['click']}
        destroyOnHidden
        open={open}
        onOpenChange={(nextOpen) => {
          if (nextOpen) {
            setAddMenu({ stepId: step.id, mode });
            return;
          }
          setAddMenu((prev) =>
            prev && prev.stepId === step.id && prev.mode === mode ? null : prev,
          );
        }}
        menu={{
          items: insertMenu(mode),
          onClick: ({ key }) => {
            const type = key.split(':')[1];
            setAddMenu(null);
            if (type) insertRelative(step, type, mode);
          },
        }}
      >
        <button
          type="button"
          className={`node-add ${className}`}
          title={title}
          onPointerDown={(event) => event.stopPropagation()}
          onMouseDown={(event) => event.stopPropagation()}
        >
          {symbol}
        </button>
      </Dropdown>
    );
  };

  if (!currentProjectId) {
    return (
      <div className="page">
        <div className="page-header">
          <div>
            <h2 className="page-title">项目流程</h2>
            <p className="page-subtitle">首尾固定的两个节点：数据导入、PDF 输出，中间可自由编排处理步骤。</p>
          </div>
        </div>
        <div className="page-fill">
          <EmptyState text="请先在顶部选择一个考试项目" />
        </div>
      </div>
    );
  }

  if (loading) {
    return (
      <div className="page">
        <div className="table-state">
          <Spinner />
          <span className="table-state-text">加载中…</span>
        </div>
      </div>
    );
  }

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h2 className="page-title">项目流程</h2>
          <p className="page-subtitle">
            流程首尾固定为「数据导入」「PDF 输出」，点击任一节点即可编辑其模板；中间节点可自由拖拽编排。
          </p>
        </div>
        <div className="row-actions">
          <button className="btn" onClick={autoLayout}>
            自动排版
          </button>
          <button className="btn" onClick={handleValidate}>
            校验
          </button>
          <button className="btn" onClick={handleRun} disabled={running}>
            {running ? '运行中…' : '运行'}
          </button>
          <button className="btn btn-primary" onClick={handleSave} disabled={saving}>
            {saving ? '保存中…' : '保存流程'}
          </button>
        </div>
      </div>

      <div className="flow-layout">
        {/* ---------------- 操作符面板 ---------------- */}
        <aside className="card flow-panel">
          <h3 className="card-title">处理步骤</h3>
          <p className="panel-hint">点击加入流程末尾，或拖到画布中间</p>
          <div className="palette">
            {operatorGroups.map(([group, list]) => (
              <div key={group} className="palette-group">
                <div className="palette-group-title">
                  <span className="palette-dot" style={{ background: groupColor(group) }} />
                  {group}
                </div>
                {list
                  .filter((op) => op.group !== '输出')
                  .map((op) => (
                    <div
                      key={op.type}
                      className="palette-item"
                      draggable
                      onDragStart={(event) => {
                        event.dataTransfer.setData('text/operator', op.type);
                        event.dataTransfer.effectAllowed = 'copy';
                      }}
                      onClick={() => addStep(op.type)}
                      title={op.description}
                    >
                      <span className="palette-item-label">{op.label}</span>
                      <span className="palette-item-type mono">{op.type}</span>
                    </div>
                  ))}
              </div>
            ))}
          </div>
        </aside>

        {/* ---------------- 流程图画布 ---------------- */}
        <section className="card flow-canvas-card">
          <div className="flow-canvas-toolbar">
            <span className="detail-meta">
              {boundDataset ? `数据源：${boundDataset.name}` : '尚未绑定数据集'}
            </span>
            <span className="detail-meta">
              {boundTemplate ? `输出模板：${boundTemplate.name}` : '尚未选择 PDF 模板'}
            </span>
          </div>
          <div
            className="uml-canvas"
            ref={hostRef}
            onDrop={handleDrop}
            onDragOver={(event) => {
              event.preventDefault();
              event.dataTransfer.dropEffect = 'copy';
            }}
            onMouseDown={() => setSelectedId(null)}
          >
            <div
              className="uml-surface"
              style={{ width: canvasSizeComputed.w, height: canvasSizeComputed.h }}
            >
              <svg
                className="uml-edges"
                width={canvasSizeComputed.w}
                height={canvasSizeComputed.h}
              >
                <defs>
                  <marker
                    id="uml-arrow"
                    viewBox="0 0 10 10"
                    refX="8"
                    refY="5"
                    markerWidth="7"
                    markerHeight="7"
                    orient="auto-start-reverse"
                  >
                    <path d="M 0 0 L 10 5 L 0 10 z" fill="#94a3b8" />
                  </marker>
                </defs>
                {(() => {
                  const mainSteps = draft.steps.filter((step) => isMainStep(step, draft.steps));
                  const points: Point[] = [
                    { x: 110, y: PAD },
                    ...mainSteps.map((step) => positions.get(step.id) ?? { x: 110, y: PAD }),
                    pdfPos,
                  ];
                  const paths = points.slice(0, -1).map((from, index) => ({
                    key: `main-${index}`,
                    d: edgePath(from, points[index + 1]),
                  }));

                  // 分支连线：首条从父节点横向连出，后续沿支链纵向连接。
                  draft.steps.forEach((step) => {
                    if (!step.branch_of) return;
                    const siblings = draft.steps.filter((item) => item.branch_of === step.branch_of);
                    const order = siblings.findIndex((item) => item.id === step.id);
                    const fromStep =
                      order > 0
                        ? siblings[order - 1]
                        : draft.steps.find((item) => item.id === step.branch_of);
                    const from = fromStep ? positions.get(fromStep.id) : null;
                    const to = positions.get(step.id);
                    if (!from || !to) return;
                    paths.push({
                      key: `branch-${step.id}`,
                      d: order === 0 ? branchEdgePath(from, to) : edgePath(from, to),
                    });
                  });

                  return paths.map((path) => (
                    <path
                      key={path.key}
                      d={path.d}
                      fill="none"
                      stroke="#94a3b8"
                      strokeWidth={1.6}
                      markerEnd="url(#uml-arrow)"
                    />
                  ));
                })()}
              </svg>

              {/* 起始锚点：数据导入 */}
              <div
                className={`uml-node anchor ${isImportSelected ? 'selected' : ''}`}
                style={{ left: 110, top: PAD, width: NODE_W, height: NODE_H }}
                onMouseDown={(event) => event.stopPropagation()}
                onClick={() => setSelectedId('anchor:import')}
              >
                <div className="uml-node-bar" style={{ background: '#0ea5e9' }} />
                <div className="uml-node-body">
                  <span className="uml-node-index" style={{ background: '#0ea5e9' }}>
                    起
                  </span>
                  <div className="uml-node-text">
                    <div className="uml-node-label">数据导入</div>
                    <div className="node-hint">
                      {boundDataset ? `${boundDataset.name} · ${boundDataset.row_count} 行` : '点击绑定数据集'}
                    </div>
                  </div>
                </div>
                <div className="uml-node-status flow" title={`数据流起点：${startFields.join('、') || '无'}`}>
                  输出字段 {startFields.length} 个
                </div>
                <div className="uml-node-port out" />
              </div>

              {/* 中间处理步骤 */}
              {draft.steps.map((step, index) => {
                const def = operatorMap.get(step.operator);
                const color = groupColor(def?.group);
                const p = positions.get(step.id) ?? { x: 110, y: PAD };
                const logEntry = runResult?.log?.[index + 1];
                const isSelected = selectedId === step.id;
                const flow = flowByStep.get(index);
                const stepErrors = flow?.errors ?? [];
                const inBranch = !isMainStep(step, draft.steps);
                const flowTip = flow
                  ? `输入字段（${flow.input_fields.length}）：${
                      flow.input_fields.join('、') || '无'
                    }\n输出字段（${flow.output_fields.length}）：${
                      flow.output_fields.join('、') || '无'
                    }`
                  : undefined;
                const statusTip = stepErrors.length ? stepErrors.join('\n') : flowTip;
                return (
                  <div
                    key={step.id}
                    className={`uml-node-wrap${
                      addMenu?.stepId === step.id ? ' menu-open' : ''
                    }`}
                    style={{ left: p.x, top: p.y, width: NODE_W, height: NODE_H }}
                  >
                    <Dropdown
                      trigger={['contextMenu']}
                      menu={{
                        items: nodeMenuItems(),
                        onClick: (info) => handleNodeMenuClick(step, info),
                      }}
                    >
                      <div
                        className={`uml-node ${isSelected ? 'selected' : ''} ${
                          step.enabled ? '' : 'disabled'
                        } ${stepErrors.length > 0 ? 'has-error' : ''}`}
                        style={{ borderColor: isSelected ? color : undefined }}
                        onMouseDown={(event) => event.stopPropagation()}
                        onPointerDown={(event) => startDrag(event, step)}
                        onClick={() => setSelectedId(step.id)}
                      >
                        <div className="uml-node-bar" style={{ background: color }} />
                        <div className="uml-node-body">
                          <span className="uml-node-index" style={{ background: color }}>
                            {inBranch ? '⑂' : index + 1}
                          </span>
                          <div className="uml-node-text">
                            <div className="uml-node-label">
                              {step.label || def?.label || step.operator}
                            </div>
                            <div className="uml-node-sub">
                              {inBranch ? '分支 · ' : ''}
                              {def ? `${def.group} · ${def.label}` : step.operator}
                            </div>
                          </div>
                        </div>
                        {!step.enabled ? (
                          <div className="uml-node-status muted">已禁用，运行时跳过</div>
                        ) : stepErrors.length > 0 ? (
                          <div className="uml-node-status error" title={statusTip}>
                            ⚠ {stepErrors.length} 个字段问题
                          </div>
                        ) : logEntry ? (
                          <div
                            className="uml-node-status"
                            style={{ color: runStatusColor(logEntry.status) }}
                            title={flowTip}
                          >
                            运行：{logEntry.status} · {logEntry.rows} 行
                          </div>
                        ) : flow ? (
                          <div className="uml-node-status flow" title={statusTip}>
                            字段 {flow.input_fields.length} → {flow.output_fields.length}
                          </div>
                        ) : null}
                        <div className="uml-node-port in" />
                        <div className="uml-node-port out" />
                      </div>
                    </Dropdown>

                    {renderAddMenu(step, 'before', 'before', '+', '在此节点前插入步骤')}
                    {renderAddMenu(step, 'after', 'after', '+', '在此节点后插入步骤')}
                    {renderAddMenu(step, 'branch', 'branch', '⑂', '从该节点添加分支步骤')}
                  </div>
                );
              })}

              {/* 结束锚点：PDF 输出 */}
              <div
                className={`uml-node anchor ${isPdfSelected ? 'selected' : ''}`}
                style={{ left: pdfPos.x, top: pdfPos.y, width: NODE_W, height: NODE_H }}
                onMouseDown={(event) => event.stopPropagation()}
                onClick={() => setSelectedId('anchor:pdf')}
              >
                <div className="uml-node-bar" style={{ background: '#0f172a' }} />
                <div className="uml-node-body">
                  <span className="uml-node-index" style={{ background: '#0f172a' }}>
                    终
                  </span>
                  <div className="uml-node-text">
                    <div className="uml-node-label">PDF 输出</div>
                    <div className="node-hint">
                      {boundTemplate ? boundTemplate.name : '点击选择 PDF 模板'}
                    </div>
                  </div>
                </div>
                <div className="uml-node-status flow" title={`数据流终点：${endFields.join('、') || '无'}`}>
                  输入字段 {endFields.length} 个
                </div>
                <div className="uml-node-port in" />
              </div>

              {draft.steps.length === 0 ? (
                <div className="uml-hint" style={{ top: PAD + NODE_H + 12 }}>
                  从左侧点击或拖入处理步骤，它们会排在「数据导入」与「PDF 输出」之间
                </div>
              ) : null}
            </div>
          </div>

          {runResult || validateErrors.length > 0 ? (
            <div className="run-drawer">
              {validateErrors.length > 0 ? (
                <div className="alert alert-error" style={{ marginTop: 0 }}>
                  <strong>校验未通过：</strong>
                  <ul>
                    {validateErrors.map((error, index) => (
                      <li key={index}>
                        第 {error.step_index + 1} 步：{error.message}
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
              {runResult ? (
                <>
                  <div
                    className={`alert ${
                      runResult.status === 'success' ? 'alert-success' : 'alert-error'
                    }`}
                    style={{ marginTop: 0 }}
                  >
                    {runResult.message}
                  </div>
                  <table className="data-table">
                    <thead className="data-table__head">
                      <tr>
                        <th className="num">#</th>
                        <th>步骤</th>
                        <th>操作符</th>
                        <th>状态</th>
                        <th className="num">行数</th>
                        <th>说明</th>
                      </tr>
                    </thead>
                    <tbody>
                      {runResult.log.map((entry, index) => (
                        <tr key={index} className={index % 2 === 1 ? 'even' : undefined}>
                          <td className="num mono">{index + 1}</td>
                          <td>{entry.label}</td>
                          <td className="mono">{entry.operator}</td>
                          <td>
                            <span
                              className={`badge badge-${
                                entry.status === 'ok'
                                  ? 'standard'
                                  : entry.status === 'failed'
                                    ? 'none'
                                    : 'alias'
                              }`}
                            >
                              {entry.status}
                            </span>
                          </td>
                          <td className="num mono">{entry.rows}</td>
                          <td>
                            {entry.message}
                            {entry.sql ? (
                              <details className="sql-detail">
                                <summary>查看 SQL</summary>
                                <pre>{entry.sql}</pre>
                              </details>
                            ) : null}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </>
              ) : null}
            </div>
          ) : null}
        </section>

        {/* ---------------- 节点属性 ---------------- */}
        <aside className="card flow-panel flow-props">
          <h3 className="card-title">
            {isImportSelected
              ? '数据导入'
              : isPdfSelected
                ? 'PDF 输出'
                : selectedStep
                  ? '步骤属性'
                  : '流程属性'}
          </h3>

          {isImportSelected ? (
            <>
              <label className="field">
                <span className="field-label">使用的数据集</span>
                <select value={currentProject?.dataset_id ?? ''} onChange={(event) => bindDataset(event.target.value)}>
                  <option value="">未绑定</option>
                  {datasets.map((dataset) => (
                    <option key={dataset.id} value={dataset.id}>
                      {dataset.name}（{dataset.row_count} 行）
                    </option>
                  ))}
                </select>
              </label>
              <button className="btn btn-primary" onClick={() => setImportOpen(true)}>
                ＋ 导入新的数据文件…
              </button>
              <p className="panel-hint" style={{ marginTop: 12 }}>
                导入后可在字段映射中把原始列名标准化；这里选择的数据集就是整条流程的输入。
              </p>
              {boundDataset ? (
                <>
                  <hr className="divider" />
                  <div className="info-grid">
                    <span className="info-label">行数</span>
                    <span className="info-value mono">{boundDataset.row_count}</span>
                    <span className="info-label">来源</span>
                    <span className="info-value">{boundDataset.source_file}</span>
                  </div>
                  <div className="chip-row">
                    {boundDataset.columns.slice(0, 40).map((column) => (
                      <span key={column.name} className="chip">
                        {column.name}
                      </span>
                    ))}
                  </div>
                </>
              ) : null}
            </>
          ) : isPdfSelected ? (
            <>
              <label className="field">
                <span className="field-label">本项目使用的模板</span>
                <select
                  value={
                    projectTemplates.some((t) => t.id === currentProject?.pdf_template_id)
                      ? currentProject?.pdf_template_id ?? ''
                      : ''
                  }
                  onChange={(event) => bindTemplate(event.target.value)}
                >
                  <option value="">未选择</option>
                  {projectTemplates.map((template) => (
                    <option key={template.id} value={template.id}>
                      {template.name}
                    </option>
                  ))}
                </select>
              </label>
              <div className="row-actions">
                <button className="btn btn-primary" onClick={() => setTemplateOpen(true)}>
                  编辑 PDF 模板
                </button>
                <button className="btn" onClick={handleExportPdf} disabled={exporting}>
                  {exporting ? '生成中…' : '生成 PDF'}
                </button>
              </div>

              <hr className="divider" />
              <label className="field">
                <span className="field-label">从全局模板库引用</span>
                <select
                  value=""
                  onChange={(event) => {
                    if (event.target.value) importGlobalTemplate(event.target.value);
                  }}
                >
                  <option value="">选择全局模板…</option>
                  {globalTemplates.map((template) => (
                    <option key={template.id} value={template.id}>
                      {template.name}
                    </option>
                  ))}
                </select>
              </label>
              <p className="panel-hint">
                引用会立即复制一份到本项目，之后对全局模板的修改不会影响本项目。
              </p>
            </>
          ) : selectedStep ? (
            <StepEditor
              step={selectedStep}
              index={selectedIndex}
              def={operatorMap.get(selectedStep.operator)}
              columns={availableColumns}
              flowFields={
                flowByStep.get(selectedIndex)?.input_fields ??
                (requiredFields.length ? requiredFields : undefined)
              }
              onRename={(label) => updateStep(selectedStep.id, { label })}
              onToggle={(enabled) => updateStep(selectedStep.id, { enabled })}
              onParam={(key, value) => updateStepParam(selectedStep.id, key, value)}
              onMove={(delta) => moveStep(selectedStep.id, delta)}
              onRemove={() => removeStep(selectedStep.id)}
            />
          ) : (
            <>
              <label className="field">
                <span className="field-label">流程名称</span>
                <input
                  type="text"
                  value={draft.name}
                  onChange={(event) => setDraft((prev) => ({ ...prev, name: event.target.value }))}
                />
              </label>
              <label className="field">
                <span className="field-label">说明</span>
                <input
                  type="text"
                  value={draft.description}
                  onChange={(event) =>
                    setDraft((prev) => ({ ...prev, description: event.target.value }))
                  }
                />
              </label>
              <p className="panel-hint">点击画布中的任一节点，即可在此编辑该节点的模板或参数。</p>
              <hr className="divider" />
              <div className="info-grid">
                <span className="info-label">步骤数</span>
                <span className="info-value mono">{draft.steps.length}</span>
                <span className="info-label">输入数据</span>
                <span className="info-value">{runDataset?.name ?? '未绑定'}</span>
                <span className="info-label">输出模板</span>
                <span className="info-value">{boundTemplate?.name ?? '未选择'}</span>
              </div>
            </>
          )}
        </aside>
      </div>

      {/* 数据导入窗口：节点内直接完成导入与字段映射 */}
      <DialogWindow
        open={importOpen}
        title="数据导入"
        width="92vw"
        flush
        onClose={() => setImportOpen(false)}
      >
        <div className="dialog-page-scroll">
          <ImportPage onImported={handleImported} />
        </div>
      </DialogWindow>

      {/* PDF 模板编辑窗口 */}
      <DialogWindow
        open={templateOpen}
        title="PDF 模板编辑"
        width="92vw"
        flush
        onClose={() => setTemplateOpen(false)}
      >
        <TemplatePage
          initialScope="project"
          initialTemplateId={currentProject?.pdf_template_id}
          onSaved={handleTemplateSaved}
        />
      </DialogWindow>
    </div>
  );
}

// --------------------------------------------------------------------
// 步骤属性编辑器
// --------------------------------------------------------------------

interface StepEditorProps {
  step: FlowStep;
  index: number;
  def: OperatorDef | undefined;
  columns: string[];
  /** 进入该步骤时的数据流字段快照；用于表达式字段校验 */
  flowFields?: string[];
  onRename: (label: string) => void;
  onToggle: (enabled: boolean) => void;
  onParam: (key: string, value: unknown) => void;
  onMove: (delta: number) => void;
  onRemove: () => void;
}

function StepEditor({
  step,
  index,
  def,
  columns,
  flowFields,
  onRename,
  onToggle,
  onParam,
  onMove,
  onRemove,
}: StepEditorProps) {
  return (
    <>
      <div className="step-head">
        <span className="step-badge" style={{ background: groupColor(def?.group) }}>
          {index + 1}
        </span>
        <div className="step-head-info">
          <div className="step-head-title">{def?.label ?? step.operator}</div>
          <div className="detail-meta mono">{step.operator}</div>
        </div>
      </div>
      {def ? <p className="panel-hint">{def.description}</p> : null}

      <label className="field">
        <span className="field-label">步骤名称</span>
        <input type="text" value={step.label} onChange={(event) => onRename(event.target.value)} />
      </label>

      <label className="checkbox-row">
        <input
          type="checkbox"
          checked={step.enabled}
          onChange={(event) => onToggle(event.target.checked)}
        />
        启用该步骤
      </label>

      {def?.params.map((param) => (
        <ParamControl
          key={param.key}
          param={param}
          value={step.params?.[param.key]}
          columns={columns}
          fields={flowFields}
          onChange={(value) => onParam(param.key, value)}
        />
      ))}

      <div className="row-actions">
        <button className="btn btn-sm" onClick={() => onMove(-1)}>
          上移
        </button>
        <button className="btn btn-sm" onClick={() => onMove(1)}>
          下移
        </button>
        <button className="btn btn-sm btn-danger" onClick={onRemove}>
          删除步骤
        </button>
      </div>
    </>
  );
}

// --------------------------------------------------------------------
// 单个参数控件
// --------------------------------------------------------------------

interface ParamControlProps {
  param: OperatorParam;
  value: unknown;
  columns: string[];
  /** 当前步骤可用的数据流字段，用于表达式字段高亮校验 */
  fields?: string[];
  onChange: (value: unknown) => void;
}

function ParamControl({ param, value, columns, fields, onChange }: ParamControlProps) {
  const label = (
    <span className="field-label">
      {param.label}
      {param.required ? <span className="req"> *</span> : null}
    </span>
  );

  if (param.type === 'bool') {
    return (
      <label className="checkbox-row">
        <input
          type="checkbox"
          checked={Boolean(value)}
          onChange={(event) => onChange(event.target.checked)}
        />
        {param.label}
      </label>
    );
  }

  if (param.type === 'enum') {
    return (
      <label className="field">
        {label}
        <select value={String(value ?? '')} onChange={(event) => onChange(event.target.value)}>
          {(param.options ?? []).map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </select>
        {param.help ? <span className="field-hint">{param.help}</span> : null}
      </label>
    );
  }

  if (param.type === 'columns' || param.type === 'fields') {
    return <ColumnsControl param={param} value={value} columns={columns} onChange={onChange} />;
  }

  if (param.type === 'mappings') {
    return <MappingsControl param={param} value={value} columns={columns} onChange={onChange} />;
  }

  if (param.type === 'number') {
    return (
      <label className="field">
        {label}
        <input
          type="number"
          value={value === undefined || value === null ? '' : Number(value)}
          onChange={(event) => onChange(event.target.value === '' ? null : Number(event.target.value))}
        />
        {param.help ? <span className="field-hint">{param.help}</span> : null}
      </label>
    );
  }

  if (param.type === 'expr') {
    return (
      <div className="field">
        {label}
        <TokenEditor
          mode="expr"
          value={value === undefined || value === null ? '' : String(value)}
          onChange={onChange}
          fields={fields ?? columns}
          placeholder={param.placeholder ?? '使用 [字段名] 插入字段，例如 [总成绩] >= 60'}
        />
        {param.help ? <span className="field-hint">{param.help}</span> : null}
      </div>
    );
  }

  const isColumn = param.type === 'column';
  // 列参数只允许选择当前步骤真实可用的字段：
  // 「必需字段校验」声明的字段 + 前序「计算新字段 / 排名」产出的新字段。
  const columnPool = fields && fields.length > 0 ? fields : columns;
  return (
    <label className="field">
      {label}
      <input
        type="text"
        list={isColumn ? `${param.key}-columns` : undefined}
        placeholder={param.placeholder}
        value={value === undefined || value === null ? '' : String(value)}
        onChange={(event) => onChange(event.target.value)}
      />
      {isColumn ? (
        <datalist id={`${param.key}-columns`}>
          {columnPool.map((column) => (
            <option key={column} value={column} />
          ))}
        </datalist>
      ) : null}
      {param.help ? <span className="field-hint">{param.help}</span> : null}
    </label>
  );
}

function toList(value: unknown): string[] {
  if (Array.isArray(value)) return value.map((item) => String(item));
  if (typeof value === 'string' && value.trim()) {
    return value
      .split(',')
      .map((item) => item.trim())
      .filter(Boolean);
  }
  return [];
}

interface ListControlProps {
  param: OperatorParam;
  value: unknown;
  columns: string[];
  onChange: (value: unknown) => void;
}

function ColumnsControl({ param, value, columns, onChange }: ListControlProps) {
  const list = toList(value);
  const toggle = (name: string) => {
    onChange(list.includes(name) ? list.filter((item) => item !== name) : [...list, name]);
  };
  return (
    <div className="field">
      <span className="field-label">
        {param.label}
        {param.required ? <span className="req"> *</span> : null}
      </span>
      <input
        type="text"
        placeholder="多个字段用逗号分隔"
        value={list.join(', ')}
        onChange={(event) => onChange(toList(event.target.value))}
      />
      {columns.length > 0 ? (
        <div className="chip-row">
          {columns.map((column) => (
            <button
              key={column}
              type="button"
              className={list.includes(column) ? 'chip chip-on' : 'chip'}
              onClick={() => toggle(column)}
            >
              {column}
            </button>
          ))}
        </div>
      ) : null}
      {param.help ? <span className="field-hint">{param.help}</span> : null}
    </div>
  );
}

function MappingsControl({ param, value, columns, onChange }: ListControlProps) {
  const rows = Array.isArray(value) ? (value as { from: string; to: string }[]) : [];
  const update = (index: number, patch: Partial<{ from: string; to: string }>) => {
    const next = rows.map((row, i) => (i === index ? { ...row, ...patch } : row));
    onChange(next);
  };
  return (
    <div className="field">
      <span className="field-label">{param.label}</span>
      {rows.map((row, index) => (
        <div key={index} className="mapping-row">
          <input
            type="text"
            placeholder="原始列名"
            value={row.from ?? ''}
            onChange={(event) => update(index, { from: event.target.value })}
          />
          <span className="mapping-arrow">→</span>
          <input
            type="text"
            list={`map-to-${index}`}
            placeholder="标准字段"
            value={row.to ?? ''}
            onChange={(event) => update(index, { to: event.target.value })}
          />
          <datalist id={`map-to-${index}`}>
            {columns.map((column) => (
              <option key={column} value={column} />
            ))}
          </datalist>
          <button
            type="button"
            className="icon-btn"
            onClick={() => onChange(rows.filter((_, i) => i !== index))}
          >
            ✕
          </button>
        </div>
      ))}
      <button type="button" className="btn btn-sm" onClick={() => onChange([...rows, { from: '', to: '' }])}>
        添加映射
      </button>
      {param.help ? <span className="field-hint">{param.help}</span> : null}
    </div>
  );
}
