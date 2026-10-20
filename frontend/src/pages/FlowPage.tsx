import { useEffect, useMemo, useRef, useState } from 'react';
import type { DragEvent as ReactDragEvent, PointerEvent as ReactPointerEvent } from 'react';
import {
  createFlow,
  deleteFlow,
  duplicateFlow,
  listDatasets,
  listFlows,
  listOperators,
  listStandardFields,
  runFlow,
  updateFlow,
  validateFlow,
} from '../api/client';
import type {
  Dataset,
  Flow,
  FlowStep,
  OperatorDef,
  OperatorParam,
  RunResult,
  StandardField,
} from '../api/types';
import { EmptyState } from '../components/EmptyState';
import { Spinner } from '../components/Spinner';
import { useToast } from '../components/Toast';
import { useProject } from '../context/ProjectContext';

// 流程图节点尺寸与间距（像素）
const NODE_W = 232;
const NODE_H = 82;
const GAP_Y = 92;
const PAD = 40;

// 操作符分组配色
const GROUP_COLORS: Record<string, string> = {
  校验: '#d97706',
  字段: '#2563eb',
  计算: '#16a34a',
  行: '#7c3aed',
  输出: '#0f172a',
};

function groupColor(group: string | undefined): string {
  return (group && GROUP_COLORS[group]) || '#475569';
}

function runStatusColor(status: string): string {
  if (status === 'ok') return '#16a34a';
  if (status === 'failed') return '#dc2626';
  return '#94a3b8';
}

/** 依据操作符定义生成一份默认参数。 */
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

interface Point {
  x: number;
  y: number;
}

/** 节点位置：有显式坐标用显式坐标，否则按顺序竖向自动排布。 */
function stepPosition(step: FlowStep, index: number): Point {
  if (step.pos) {
    return { x: step.pos.x, y: step.pos.y };
  }
  return { x: 120, y: PAD + index * (NODE_H + GAP_Y) };
}

/** 顺序连线：从上一节点底部中心到下一节点顶部中心，画一条带箭头的三次贝塞尔曲线。 */
function edgePath(from: Point, to: Point): string {
  const x1 = from.x + NODE_W / 2;
  const y1 = from.y + NODE_H;
  const x2 = to.x + NODE_W / 2;
  const y2 = to.y;
  const dir = y2 >= y1 ? 1 : -1;
  const bend = Math.min(60, Math.max(24, Math.abs(y2 - y1) / 2));
  return `M ${x1} ${y1} C ${x1} ${y1 + dir * bend}, ${x2} ${y2 - dir * bend}, ${x2} ${y2}`;
}

interface FlowDraft {
  id: string | null;
  name: string;
  description: string;
  steps: FlowStep[];
  scope: 'global' | 'project';
}

const EMPTY_DRAFT: FlowDraft = {
  id: null,
  name: '新流程',
  description: '',
  steps: [],
  scope: 'global',
};

export default function FlowPage() {
  const { show } = useToast();
  const { currentProject, currentProjectId } = useProject();

  const [operators, setOperators] = useState<OperatorDef[]>([]);
  const [flows, setFlows] = useState<Flow[]>([]);
  const [datasets, setDatasets] = useState<Dataset[]>([]);
  const [standardFields, setStandardFields] = useState<StandardField[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  const [scope, setScope] = useState<'project' | 'global'>('global');
  const [draft, setDraft] = useState<FlowDraft>(EMPTY_DRAFT);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [runDatasetId, setRunDatasetId] = useState('');
  const [runResult, setRunResult] = useState<RunResult | null>(null);
  const [validateErrors, setValidateErrors] = useState<{ step_index: number; message: string }[]>([]);
  const [running, setRunning] = useState(false);

  const [canvasSize, setCanvasSize] = useState({ w: 760, h: 520 });
  const [dragging, setDragging] = useState(false);

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
  const runDataset = datasets.find((dataset) => dataset.id === runDatasetId) ?? null;

  const availableColumns = useMemo(() => {
    const names = new Set<string>();
    runDataset?.columns.forEach((column) => names.add(column.name));
    standardFields.forEach((field) => names.add(field.name));
    return [...names];
  }, [runDataset, standardFields]);

  // 画布尺寸：随节点实际坐标自适应
  const canvasSizeComputed = useMemo(() => {
    let maxX = canvasSize.w;
    let maxY = canvasSize.h;
    draft.steps.forEach((step, index) => {
      const p = stepPosition(step, index);
      maxX = Math.max(maxX, p.x + NODE_W + PAD);
      maxY = Math.max(maxY, p.y + NODE_H + PAD);
    });
    return { w: maxX, h: maxY };
  }, [draft.steps, canvasSize]);

  // ------------------------------------------------------------------
  // 加载：操作符、当前作用域下的流程、数据集、标准字段
  // ------------------------------------------------------------------
  useEffect(() => {
    let active = true;
    (async () => {
      setLoading(true);
      try {
        const [ops, datasetList, fields] = await Promise.all([
          listOperators(),
          listDatasets(currentProjectId || undefined),
          listStandardFields(),
        ]);
        if (!active) return;
        setOperators(ops);
        setDatasets(datasetList);
        setStandardFields(fields);
        if (datasetList.length > 0) setRunDatasetId(datasetList[0].id);
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

  // 作用域切换 / 当前项目变化时重新拉流程列表
  useEffect(() => {
    let active = true;
    (async () => {
      try {
        const list = await listFlows(
          scope === 'project' && currentProjectId ? { projectId: currentProjectId } : { scope: 'global' },
        );
        if (!active) return;
        setFlows(list);
        if (list.length > 0) {
          setDraft(toDraft(list[0]));
        } else {
          setDraft({ ...EMPTY_DRAFT, scope });
        }
        setSelectedId(null);
        setRunResult(null);
      } catch (err) {
        show(err instanceof Error ? err.message : '加载流程列表失败', 'error');
      }
    })();
    return () => {
      active = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope, currentProjectId]);

  // ------------------------------------------------------------------
  // 画布容器尺寸测量
  // ------------------------------------------------------------------
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const measure = () =>
      setCanvasSize({ w: host.clientWidth || 760, h: host.clientHeight || 520 });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(host);
    return () => observer.disconnect();
  }, [loading]);

  // ------------------------------------------------------------------
  // 操作
  // ------------------------------------------------------------------
  const addStep = (type: string, index?: number, pos?: Point) => {
    const def = operatorMap.get(type);
    const step: FlowStep = {
      id: newStepId(),
      operator: type,
      label: def?.label ?? type,
      enabled: true,
      params: defaultParamsFor(def),
      pos,
    };
    setDraft((prev) => {
      const next = [...prev.steps];
      const at = index === undefined ? next.length : Math.min(Math.max(index, 0), next.length);
      next.splice(at, 0, step);
      return { ...prev, steps: next };
    });
    setSelectedId(step.id);
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

  /** 自动排版：清空显式坐标，回到竖向顺序布局。 */
  const autoLayout = () => {
    setDraft((prev) => ({
      ...prev,
      steps: prev.steps.map((step) => ({ ...step, pos: undefined })),
    }));
    show('已按顺序重新排版', 'success');
  };

  /** 自由拖拽节点。 */
  const startDrag = (event: ReactPointerEvent<HTMLDivElement>, step: FlowStep, index: number) => {
    event.preventDefault();
    setSelectedId(step.id);
    const startX = event.clientX;
    const startY = event.clientY;
    const origin = stepPosition(step, index);
    let moved = false;
    setDragging(true);

    const onMove = (ev: PointerEvent) => {
      const nx = origin.x + (ev.clientX - startX);
      const ny = origin.y + (ev.clientY - startY);
      if (Math.abs(ev.clientX - startX) > 2 || Math.abs(ev.clientY - startY) > 2) moved = true;
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
      setDragging(false);
      void moved;
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  };

  const selectFlow = (id: string) => {
    if (!id) {
      setDraft({ ...EMPTY_DRAFT, scope });
      setSelectedId(null);
      return;
    }
    const flow = flows.find((item) => item.id === id);
    if (!flow) return;
    setDraft(toDraft(flow));
    setSelectedId(null);
    setRunResult(null);
    setValidateErrors([]);
  };

  const refreshFlows = async (): Promise<Flow[]> => {
    const list = await listFlows(
      scope === 'project' && currentProjectId ? { projectId: currentProjectId } : { scope: 'global' },
    );
    setFlows(list);
    return list;
  };

  const handleSave = async () => {
    if (!draft.name.trim()) {
      show('请填写流程名称', 'error');
      return;
    }
    if (scope === 'project' && !currentProjectId) {
      show('请先在顶部选择一个考试项目', 'error');
      return;
    }
    setSaving(true);
    try {
      const payload = {
        name: draft.name.trim(),
        description: draft.description,
        steps: draft.steps,
        scope: draft.id ? draft.scope : scope,
        project_id: draft.id ? (draft.scope === 'project' ? currentProjectId : '') : scope === 'project' ? currentProjectId : '',
      };
      const saved = draft.id ? await updateFlow(draft.id, payload) : await createFlow(payload);
      await refreshFlows();
      setDraft(toDraft(saved));
      show(scope === 'global' ? '已保存到全局流程库' : '已保存到本项目', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '保存流程失败', 'error');
    } finally {
      setSaving(false);
    }
  };

  const handleDeleteFlow = async () => {
    if (!draft.id) {
      setDraft({ ...EMPTY_DRAFT, scope });
      setSelectedId(null);
      return;
    }
    if (!window.confirm(`确认删除流程「${draft.name}」？`)) return;
    try {
      await deleteFlow(draft.id);
      const list = await refreshFlows();
      setDraft(list.length > 0 ? toDraft(list[0]) : { ...EMPTY_DRAFT, scope });
      setSelectedId(null);
      show('流程已删除', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '删除流程失败', 'error');
    }
  };

  /** 把全局库中的流程引用到当前项目：立即复制一份项目副本。 */
  const handleDuplicate = async () => {
    if (!draft.id) {
      show('请先选择一个全局流程', 'error');
      return;
    }
    if (!currentProjectId) {
      show('请先在顶部选择一个考试项目', 'error');
      return;
    }
    try {
      const copy = await duplicateFlow(draft.id, { project_id: currentProjectId });
      show(`已复制为项目副本「${copy.name}」`, 'success');
      setScope('project');
      // 列表刷新后由 scope 变化触发；这里主动载入副本
      setTimeout(() => {
        setDraft(toDraft(copy));
      }, 0);
    } catch (err) {
      show(err instanceof Error ? err.message : '引用失败', 'error');
    }
  };

  const handleValidate = async () => {
    if (!runDataset) {
      show('请先选择用于校验的数据集', 'error');
      return;
    }
    try {
      const result = await validateFlow(
        draft.steps,
        runDataset.columns.map((column) => column.name),
      );
      setValidateErrors(result.errors);
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
    if (!runDatasetId) {
      show('请选择要处理的数据集', 'error');
      return;
    }
    setRunning(true);
    setRunResult(null);
    try {
      const result = await runFlow(draft.id, runDatasetId);
      setRunResult(result);
      show(result.message ?? '流程已执行', result.status === 'success' ? 'success' : 'error');
    } catch (err) {
      show(err instanceof Error ? err.message : '运行流程失败', 'error');
    } finally {
      setRunning(false);
    }
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
    // 依据落点纵坐标决定插入到第几步
    let index = draft.steps.length;
    for (let i = 0; i < draft.steps.length; i += 1) {
      const p = stepPosition(draft.steps[i], i);
      if (dropY < p.y + NODE_H / 2) {
        index = i;
        break;
      }
    }
    addStep(type, index, { x: Math.max(0, dropX - NODE_W / 2), y: Math.max(0, dropY - 20) });
  };

  const handleDragOver = (event: ReactDragEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.dataTransfer.dropEffect = 'copy';
  };

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
    <div className="page flow-page">
      <div className="page-header">
        <div>
          <h2 className="page-title">处理流程</h2>
          <p className="page-subtitle">
            以 UML 风格流程图编排原子操作符：节点可自由拖拽，步骤之间按顺序自动连线并带箭头。
          </p>
        </div>
        <div className="row-actions">
          <div className="segmented">
            <button
              className={scope === 'project' ? 'seg active' : 'seg'}
              onClick={() => setScope('project')}
              disabled={!currentProjectId}
              title={currentProjectId ? '本项目专用流程' : '请先在顶部选择考试项目'}
            >
              本项目{currentProject ? `（${currentProject.name}）` : ''}
            </button>
            <button
              className={scope === 'global' ? 'seg active' : 'seg'}
              onClick={() => setScope('global')}
            >
              全局流程库
            </button>
          </div>
        </div>
      </div>

      <div className="flow-toolbar card">
        <select value={draft.id ?? ''} onChange={(event) => selectFlow(event.target.value)}>
          <option value="">＋ 新建流程</option>
          {flows.map((flow) => (
            <option key={flow.id} value={flow.id}>
              {flow.name}
            </option>
          ))}
        </select>
        <button className="btn btn-primary" onClick={handleSave} disabled={saving}>
          {saving ? '保存中…' : '保存流程'}
        </button>
        <button className="btn" onClick={autoLayout}>
          自动排版
        </button>
        {scope === 'global' && draft.id ? (
          <button className="btn" onClick={handleDuplicate} disabled={!currentProjectId}>
            引用到本项目
          </button>
        ) : null}
        <button className="btn btn-danger" onClick={handleDeleteFlow}>
          删除
        </button>
        <span className="detail-meta">共 {draft.steps.length} 步</span>
      </div>

      <div className="split-3">
        {/* ---------------- 操作符面板 ---------------- */}
        <aside className="card flow-panel">
          <h3 className="card-title">原子操作符</h3>
          <p className="panel-hint">点击加入流程末尾，或拖到画布任意位置</p>
          <div className="palette">
            {operatorGroups.map(([group, list]) => (
              <div key={group} className="palette-group">
                <div className="palette-group-title">
                  <span className="palette-dot" style={{ background: groupColor(group) }} />
                  {group}
                </div>
                {list.map((op) => (
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
            <span className="detail-meta">拖动节点自由摆放；上/下移按钮调整执行顺序</span>
            {dragging ? <span className="detail-meta">拖拽中…</span> : null}
          </div>
          <div
            className="uml-canvas"
            ref={hostRef}
            onDrop={handleDrop}
            onDragOver={handleDragOver}
            onMouseDown={() => setSelectedId(null)}
          >
            {draft.steps.length === 0 ? (
              <div className="uml-empty">从左侧操作符面板点击或拖入，添加第一个处理步骤</div>
            ) : (
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
                  {draft.steps.slice(0, -1).map((step, index) => {
                    const from = stepPosition(step, index);
                    const to = stepPosition(draft.steps[index + 1], index + 1);
                    return (
                      <path
                        key={step.id}
                        d={edgePath(from, to)}
                        fill="none"
                        stroke={step.enabled ? '#94a3b8' : '#e2e8f0'}
                        strokeWidth={1.6}
                        markerEnd="url(#uml-arrow)"
                      />
                    );
                  })}
                </svg>

                {draft.steps.map((step, index) => {
                  const def = operatorMap.get(step.operator);
                  const color = groupColor(def?.group);
                  const p = stepPosition(step, index);
                  const logEntry = runResult?.log?.[index];
                  const isSelected = selectedId === step.id;
                  return (
                    <div
                      key={step.id}
                      className={`uml-node ${isSelected ? 'selected' : ''} ${
                        step.enabled ? '' : 'disabled'
                      }`}
                      style={{
                        left: p.x,
                        top: p.y,
                        width: NODE_W,
                        height: NODE_H,
                        borderColor: isSelected ? color : undefined,
                      }}
                      onMouseDown={(event) => event.stopPropagation()}
                      onPointerDown={(event) => startDrag(event, step, index)}
                    >
                      <div className="uml-node-bar" style={{ background: color }} />
                      <div className="uml-node-body">
                        <span className="uml-node-index" style={{ background: color }}>
                          {index + 1}
                        </span>
                        <div className="uml-node-text">
                          <div className="uml-node-label">
                            {step.label || def?.label || step.operator}
                          </div>
                          <div className="uml-node-sub">
                            {def ? `${def.group} · ${def.label}` : step.operator}
                          </div>
                        </div>
                      </div>
                      {!step.enabled ? (
                        <div className="uml-node-status muted">已禁用，运行时跳过</div>
                      ) : logEntry ? (
                        <div
                          className="uml-node-status"
                          style={{ color: runStatusColor(logEntry.status) }}
                        >
                          运行：{logEntry.status} · {logEntry.rows} 行
                        </div>
                      ) : null}
                      <div className="uml-node-port in" />
                      <div className="uml-node-port out" />
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </section>

        {/* ---------------- 属性面板 ---------------- */}
        <aside className="card flow-panel flow-props">
          <h3 className="card-title">流程属性</h3>
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
          <p className="panel-hint">
            作用域：{draft.id ? (draft.scope === 'global' ? '全局流程库' : '本项目') : scope === 'global' ? '全局流程库' : '本项目'}
          </p>

          <hr className="divider" />

          {selectedStep ? (
            <StepEditor
              step={selectedStep}
              index={draft.steps.findIndex((step) => step.id === selectedStep.id)}
              def={operatorMap.get(selectedStep.operator)}
              columns={availableColumns}
              onRename={(label) => updateStep(selectedStep.id, { label })}
              onToggle={(enabled) => updateStep(selectedStep.id, { enabled })}
              onParam={(key, value) => updateStepParam(selectedStep.id, key, value)}
              onMove={(delta) => moveStep(selectedStep.id, delta)}
              onRemove={() => removeStep(selectedStep.id)}
            />
          ) : (
            <EmptyState text="在流程图中选中一个节点，编辑其参数" />
          )}
        </aside>
      </div>

      {/* ---------------- 校验 / 运行 ---------------- */}
      <section className="card">
        <h3 className="card-title">校验与运行</h3>
        <div className="inline-add">
          <select
            value={runDatasetId}
            onChange={(event) => {
              setRunDatasetId(event.target.value);
              setRunResult(null);
              setValidateErrors([]);
            }}
          >
            <option value="">选择数据集…</option>
            {datasets.map((dataset) => (
              <option key={dataset.id} value={dataset.id}>
                {dataset.name}（{dataset.row_count} 行）
              </option>
            ))}
          </select>
          <button className="btn" onClick={handleValidate}>
            校验流程
          </button>
          <button className="btn btn-primary" onClick={handleRun} disabled={running}>
            {running ? '运行中…' : '运行流程'}
          </button>
        </div>

        {validateErrors.length > 0 ? (
          <div className="alert alert-error">
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
          <div className="run-result">
            <div
              className={`alert ${runResult.status === 'success' ? 'alert-success' : 'alert-error'}`}
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
                          entry.status === 'ok' ? 'standard' : entry.status === 'failed' ? 'none' : 'alias'
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
          </div>
        ) : null}
      </section>
    </div>
  );
}

function toDraft(flow: Flow): FlowDraft {
  return {
    id: flow.id,
    name: flow.name,
    description: flow.description,
    steps: flow.steps,
    scope: flow.scope === 'project' ? 'project' : 'global',
  };
}

// --------------------------------------------------------------------
// 步骤属性编辑器
// --------------------------------------------------------------------

interface StepEditorProps {
  step: FlowStep;
  index: number;
  def: OperatorDef | undefined;
  columns: string[];
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
        <input
          type="text"
          value={step.label}
          onChange={(event) => onRename(event.target.value)}
        />
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
  onChange: (value: unknown) => void;
}

function ParamControl({ param, value, columns, onChange }: ParamControlProps) {
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
          onChange={(event) =>
            onChange(event.target.value === '' ? null : Number(event.target.value))
          }
        />
        {param.help ? <span className="field-hint">{param.help}</span> : null}
      </label>
    );
  }

  const isColumn = param.type === 'column';
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
          {columns.map((column) => (
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
      <button
        type="button"
        className="btn btn-sm"
        onClick={() => onChange([...rows, { from: '', to: '' }])}
      >
        添加映射
      </button>
      {param.help ? <span className="field-hint">{param.help}</span> : null}
    </div>
  );
}
