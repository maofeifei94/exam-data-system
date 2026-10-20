import { useEffect, useMemo, useRef, useState } from 'react';
import { App, Group, Rect, Text } from 'leafer-ui';
import {
  createPdfTemplate,
  deletePdfTemplate,
  duplicatePdfTemplate,
  listDatasets,
  listPdfTemplates,
  listStandardFields,
  previewPdfTemplate,
  renderPdfTemplate,
  updatePdfTemplate,
} from '../api/client';
import type {
  Dataset,
  MatrixSetup,
  PageSetup,
  PdfTemplate,
  PdfTemplateInput,
  StandardField,
  TemplateElement,
} from '../api/types';
import { EmptyState } from '../components/EmptyState';
import { Spinner } from '../components/Spinner';
import { useToast } from '../components/Toast';
import { useProject } from '../context/ProjectContext';

const MM_PER_PT = 0.352778;
const PAD = 22;

type ElementType = TemplateElement['type'];

interface TemplateDraft {
  id: string | null;
  name: string;
  kind: 'single' | 'matrix';
  description: string;
  page: PageSetup;
  matrix: MatrixSetup;
  elements: TemplateElement[];
  scope: 'global' | 'project';
}

const DEFAULT_PAGE: PageSetup = {
  size: 'A4',
  orientation: 'portrait',
  margin_mm: { top: 15, right: 12, bottom: 12, left: 12 },
};

const DEFAULT_MATRIX: MatrixSetup = {
  rows: 5,
  cols: 6,
  gap_x_mm: 2,
  gap_y_mm: 2,
  cell_h_mm: 42,
  origin_y_mm: 0,
};

const EMPTY_DRAFT: TemplateDraft = {
  id: null,
  name: '新模板',
  kind: 'single',
  description: '',
  page: DEFAULT_PAGE,
  matrix: DEFAULT_MATRIX,
  elements: [],
  scope: 'global',
};

function defaultElement(type: ElementType, field: string, pageLevel: boolean): TemplateElement {
  const base = {
    w_mm: 60,
    h_mm: 8,
    text: '',
    field: '',
    font_size: 11,
    bold: false,
    align: 'left' as const,
    color: '#000000',
    fill: '',
    border: false,
    border_radius_mm: 0,
    page_level: pageLevel,
  };
  switch (type) {
    case 'rect':
      return { ...base, id: newElementId(), type, x_mm: 6, y_mm: 6, w_mm: 60, h_mm: 30, fill: '#ffffff', border: true };
    case 'line':
      return { ...base, id: newElementId(), type, x_mm: 6, y_mm: 6, w_mm: 120, h_mm: 1 };
    case 'image':
      return { ...base, id: newElementId(), type, x_mm: 6, y_mm: 6, w_mm: 30, h_mm: 40, field: field || '照片', border: true };
    case 'field':
      return { ...base, id: newElementId(), type, x_mm: 6, y_mm: 6, w_mm: 60, h_mm: 7, field: field || '姓名' };
    default:
      return { ...base, id: newElementId(), type, x_mm: 6, y_mm: 6, w_mm: 60, h_mm: 8, text: '文本内容' };
  }
}

function newElementId(): string {
  return `el_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`;
}

function pageSizeMm(page: PageSetup): { w: number; h: number } {
  const sizes: Record<string, [number, number]> = {
    A3: [297, 420],
    A4: [210, 297],
    A5: [148, 210],
    Letter: [215.9, 279.4],
  };
  const [w, h] = sizes[page.size] ?? sizes.A4;
  return page.orientation === 'landscape' ? { w: h, h: w } : { w, h };
}

function round1(value: number): number {
  return Math.round(value * 10) / 10;
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

export default function TemplatePage() {
  const { show } = useToast();
  const { currentProject, currentProjectId } = useProject();

  const [templates, setTemplates] = useState<PdfTemplate[]>([]);
  const [datasets, setDatasets] = useState<Dataset[]>([]);
  const [fields, setFields] = useState<StandardField[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [busy, setBusy] = useState(false);
  const [datasetId, setDatasetId] = useState('');
  const [scope, setScope] = useState<'project' | 'global'>('global');

  const [draft, setDraft] = useState<TemplateDraft>(EMPTY_DRAFT);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [size, setSize] = useState({ w: 620, h: 740 });

  const hostRef = useRef<HTMLDivElement | null>(null);
  const appRef = useRef<App | null>(null);

  const page = pageSizeMm(draft.page);
  const scale = useMemo(() => {
    const s = Math.min((size.w - PAD * 2) / page.w, (size.h - PAD * 2) / page.h);
    return Math.max(0.6, Math.min(4, s));
  }, [size, page.w, page.h]);

  const origin = useMemo(
    () => ({ x: Math.max(PAD, (size.w - page.w * scale) / 2), y: PAD }),
    [size.w, page.w, scale, page.h, size.h],
  );

  const selected = draft.elements.find((element) => element.id === selectedId) ?? null;

  const columnNames = useMemo(() => {
    const names = new Set<string>();
    datasets.find((dataset) => dataset.id === datasetId)?.columns.forEach((column) => names.add(column.name));
    fields.forEach((field) => names.add(field.name));
    return [...names];
  }, [datasets, datasetId, fields]);

  // ------------------------------------------------------------------
  // 加载数据集 / 标准字段（随当前考试项目变化）
  // ------------------------------------------------------------------
  useEffect(() => {
    let active = true;
    (async () => {
      setLoading(true);
      try {
        const [datasetList, fieldList] = await Promise.all([
          listDatasets(currentProjectId || undefined),
          listStandardFields(),
        ]);
        if (!active) return;
        setDatasets(datasetList);
        setFields(fieldList);
        if (datasetList.length > 0) setDatasetId(datasetList[0].id);
      } catch (err) {
        show(err instanceof Error ? err.message : '加载数据失败', 'error');
      } finally {
        if (active) setLoading(false);
      }
    })();
    return () => {
      active = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentProjectId]);

  // ------------------------------------------------------------------
  // 加载当前作用域下的模板（随作用域 / 当前项目变化）
  // ------------------------------------------------------------------
  useEffect(() => {
    let active = true;
    (async () => {
      try {
        const list = await listPdfTemplates(
          scope === 'project' && currentProjectId ? { projectId: currentProjectId } : { scope: 'global' },
        );
        if (!active) return;
        setTemplates(list);
        setDraft(list.length > 0 ? toDraft(list[0]) : { ...EMPTY_DRAFT, scope });
        setSelectedId(null);
      } catch (err) {
        show(err instanceof Error ? err.message : '加载模板列表失败', 'error');
      }
    })();
    return () => {
      active = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope, currentProjectId]);

  // ------------------------------------------------------------------
  // 初始化画布
  // ------------------------------------------------------------------
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const app = new App({
      view: host as unknown as Record<string, unknown>,
      width: host.clientWidth || 620,
      height: host.clientHeight || 740,
      tree: { type: 'design' },
    });
    appRef.current = app;
    const measure = () => setSize({ w: host.clientWidth || 620, h: host.clientHeight || 740 });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(host);
    return () => {
      observer.disconnect();
      app.destroy();
      appRef.current = null;
    };
    // 依赖 loading：加载期间画布容器尚未挂载，需等真实布局渲染后再创建画布。
  }, [loading]);

  useEffect(() => {
    const app = appRef.current;
    if (!app) return;
    app.resize({ width: size.w, height: size.h });
  }, [size]);

  // ------------------------------------------------------------------
  // 重建画布
  // ------------------------------------------------------------------
  useEffect(() => {
    const app = appRef.current;
    if (!app || !app.tree) return;
    const tree = app.tree;
    tree.removeAll();

    const background = new Rect({ x: 0, y: 0, width: size.w, height: size.h, fill: '#eef2f7' });
    background.hittable = true;
    background.on('tap', () => setSelectedId(null));
    tree.add(background);

    // 纸张
    tree.add(
      new Rect({
        x: origin.x,
        y: origin.y,
        width: page.w * scale,
        height: page.h * scale,
        fill: '#ffffff',
        stroke: '#cbd5e1',
        strokeWidth: 1,
      }),
    );

    const m = draft.page.margin_mm;
    const contentX = origin.x + m.left * scale;
    const contentY = origin.y + m.top * scale;
    const contentW = Math.max(10, (page.w - m.left - m.right) * scale);
    const contentH = Math.max(10, (page.h - m.top - m.bottom) * scale);

    // 内容区虚线框
    tree.add(
      new Rect({
        x: contentX,
        y: contentY,
        width: contentW,
        height: contentH,
        stroke: '#cbd5e1',
        strokeWidth: 1,
        dashPattern: [4, 4],
      }),
    );

    const isMatrix = draft.kind === 'matrix';
    const mx = draft.matrix;
    const cols = Math.max(1, Math.round(mx.cols));
    const rows = Math.max(1, Math.round(mx.rows));
    const cellW = Math.max(4, (contentW - mx.gap_x_mm * scale * (cols - 1)) / cols);
    const cellH =
      mx.cell_h_mm > 0
        ? mx.cell_h_mm * scale
        : Math.max(4, (contentH - mx.origin_y_mm * scale - mx.gap_y_mm * scale * (rows - 1)) / rows);

    const cellOrigin = (r: number, c: number) => ({
      x: contentX + c * (cellW + mx.gap_x_mm * scale),
      y: contentY + mx.origin_y_mm * scale + r * (cellH + mx.gap_y_mm * scale),
    });

    let cellLevelOrigin = { x: contentX, y: contentY };
    let pageLevelOrigin = { x: contentX, y: contentY };

    if (isMatrix) {
      for (let r = 0; r < rows; r += 1) {
        for (let c = 0; c < cols; c += 1) {
          const o = cellOrigin(r, c);
          const first = r === 0 && c === 0;
          tree.add(
            new Rect({
              x: o.x,
              y: o.y,
              width: cellW,
              height: cellH,
              fill: first ? '#eff6ff' : '#f8fafc',
              stroke: first ? '#93c5fd' : '#e2e8f0',
              strokeWidth: 1,
              dashPattern: first ? undefined : [3, 3],
            }),
          );
        }
      }
      cellLevelOrigin = cellOrigin(0, 0);
    }

    const isPageLevel = (element: TemplateElement) => isMatrix && element.page_level;

    const buildElement = (
      element: TemplateElement,
      ox: number,
      oy: number,
      interactive: boolean,
    ): Group => {
      const gx = ox + element.x_mm * scale;
      const gy = oy + element.y_mm * scale;
      const w = Math.max(2, element.w_mm * scale);
      const h = Math.max(2, element.h_mm * scale);
      const fs = Math.max(6, element.font_size * MM_PER_PT * scale);
      const radius = Math.max(0, element.border_radius_mm * scale);
      const children = [
        new Rect({ x: 0, y: 0, width: w, height: h, fill: 'rgba(0,0,0,0.001)', hittable: true }),
      ];

      if (element.type === 'rect') {
        children.push(
          new Rect({
            x: 0,
            y: 0,
            width: w,
            height: h,
            fill: element.fill || undefined,
            stroke: element.border ? element.color : undefined,
            strokeWidth: element.border ? 1 : 0,
            cornerRadius: radius,
          }),
        );
      } else if (element.type === 'line') {
        children.push(new Rect({ x: 0, y: 0, width: w, height: h, fill: element.color || '#000000' }));
      } else if (element.type === 'image') {
        children.push(
          new Rect({ x: 0, y: 0, width: w, height: h, fill: element.fill || '#f1f5f9', stroke: '#cbd5e1', strokeWidth: 1 }),
          new Text({
            x: 0,
            y: h / 2 - fs / 2,
            width: w,
            text: `🖼 ${element.field || '照片'}`,
            fontSize: Math.min(fs, 12),
            fill: '#94a3b8',
            textAlign: 'center',
          }),
        );
      } else {
        if (element.fill) {
          children.push(new Rect({ x: 0, y: 0, width: w, height: h, fill: element.fill, cornerRadius: radius }));
        }
        if (element.border) {
          children.push(new Rect({ x: 0, y: 0, width: w, height: h, stroke: element.color, strokeWidth: 1, cornerRadius: radius }));
        }
        const content = element.text.trim() ? element.text : element.field ? `{${element.field}}` : '';
        children.push(
          new Text({
            x: 0,
            y: 0,
            width: w,
            height: h,
            text: content || '（空）',
            fontSize: fs,
            fontWeight: element.bold ? 'bold' : 'normal',
            fill: element.color || '#000000',
            textAlign: element.align,
            verticalAlign: 'middle',
            textWrap: 'break',
          }),
        );
      }

      if (interactive) {
        children.push(
          new Rect({
            x: 0,
            y: 0,
            width: w,
            height: h,
            stroke: selectedId === element.id ? '#2563eb' : '#94a3b8',
            strokeWidth: selectedId === element.id ? 2 : 1,
            dashPattern: selectedId === element.id ? undefined : [4, 3],
          }),
        );
      }

      const group = new Group({
        x: gx,
        y: gy,
        width: w,
        height: h,
        children,
        draggable: interactive,
        cursor: interactive ? 'move' : 'default',
      });

      if (interactive) {
        group.on('tap', () => setSelectedId(element.id));
        group.on('drag.end', () => {
          const nx = ((group.x ?? gx) - ox) / scale;
          const ny = ((group.y ?? gy) - oy) / scale;
          setDraft((prev) => ({
            ...prev,
            elements: prev.elements.map((item) =>
              item.id === element.id ? { ...item, x_mm: round1(nx), y_mm: round1(ny) } : item,
            ),
          }));
        });
      } else {
        group.opacity = 0.45;
      }

      return group;
    };

    const cellElements = draft.elements.filter((element) => !isPageLevel(element));
    const pageElements = draft.elements.filter((element) => isPageLevel(element));

    if (isMatrix) {
      // 首格：可交互编辑
      cellElements.forEach((element) => tree.add(buildElement(element, cellLevelOrigin.x, cellLevelOrigin.y, true)));
      // 其余格：半透明预览
      for (let r = 0; r < rows; r += 1) {
        for (let c = 0; c < cols; c += 1) {
          if (r === 0 && c === 0) continue;
          const o = cellOrigin(r, c);
          cellElements.forEach((element) => tree.add(buildElement(element, o.x, o.y, false)));
        }
      }
    } else {
      draft.elements.forEach((element) =>
        tree.add(buildElement(element, cellLevelOrigin.x, cellLevelOrigin.y, true)),
      );
    }

    pageElements.forEach((element) => tree.add(buildElement(element, pageLevelOrigin.x, pageLevelOrigin.y, true)));
  }, [draft, selectedId, size, scale, origin.x, origin.y, page.w, page.h, loading]);

  // ------------------------------------------------------------------
  // 操作
  // ------------------------------------------------------------------
  const updateElement = (id: string, patch: Partial<TemplateElement>) => {
    setDraft((prev) => ({
      ...prev,
      elements: prev.elements.map((element) => (element.id === id ? { ...element, ...patch } : element)),
    }));
  };

  const addElement = (type: ElementType) => {
    const pageLevel = draft.kind === 'matrix' && false;
    const element = defaultElement(type, columnNames[0] ?? '', pageLevel);
    setDraft((prev) => ({ ...prev, elements: [...prev.elements, element] }));
    setSelectedId(element.id);
  };

  const removeElement = (id: string) => {
    setDraft((prev) => ({ ...prev, elements: prev.elements.filter((element) => element.id !== id) }));
    if (selectedId === id) setSelectedId(null);
  };

  const refreshTemplates = async () => {
    try {
      setTemplates(
        await listPdfTemplates(
          scope === 'project' && currentProjectId ? { projectId: currentProjectId } : { scope: 'global' },
        ),
      );
    } catch (err) {
      show(err instanceof Error ? err.message : '刷新模板列表失败', 'error');
    }
  };

  const selectTemplate = (id: string) => {
    if (!id) {
      setDraft({ ...EMPTY_DRAFT, scope });
      setSelectedId(null);
      return;
    }
    const template = templates.find((item) => item.id === id);
    if (!template) return;
    setDraft(toDraft(template));
    setSelectedId(null);
  };

  const buildPayload = (): PdfTemplateInput => ({
    name: draft.name.trim(),
    kind: draft.kind,
    description: draft.description,
    page: draft.page,
    matrix: draft.matrix,
    elements: draft.elements,
    scope: draft.id ? draft.scope : scope,
    project_id: draft.id
      ? draft.scope === 'project'
        ? currentProjectId
        : ''
      : scope === 'project'
        ? currentProjectId
        : '',
  });

  const handleSave = async () => {
    if (!draft.name.trim()) {
      show('请填写模板名称', 'error');
      return;
    }
    if (scope === 'project' && !currentProjectId) {
      show('请先在顶部选择一个考试项目', 'error');
      return;
    }
    setSaving(true);
    try {
      const payload = buildPayload();
      const saved = draft.id
        ? await updatePdfTemplate(draft.id, payload)
        : await createPdfTemplate(payload);
      setDraft(toDraft(saved));
      await refreshTemplates();
      show(scope === 'global' ? '已保存到全局模板库' : '已保存到本项目', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '保存模板失败', 'error');
    } finally {
      setSaving(false);
    }
  };

  /** 把全局模板库中的模板引用到当前项目：立即复制一份项目副本。 */
  const handleDuplicate = async () => {
    if (!draft.id) {
      show('请先选择一个全局模板', 'error');
      return;
    }
    if (!currentProjectId) {
      show('请先在顶部选择一个考试项目', 'error');
      return;
    }
    try {
      const copy = await duplicatePdfTemplate(draft.id, { project_id: currentProjectId });
      show(`已复制为项目副本「${copy.name}」`, 'success');
      setScope('project');
      setTimeout(() => setDraft(toDraft(copy)), 0);
    } catch (err) {
      show(err instanceof Error ? err.message : '引用失败', 'error');
    }
  };

  const handleDelete = async () => {
    if (!draft.id) {
      setDraft({ ...EMPTY_DRAFT, scope });
      setSelectedId(null);
      return;
    }
    if (!window.confirm(`确认删除模板「${draft.name}」？`)) return;
    try {
      await deletePdfTemplate(draft.id);
      const list = await listPdfTemplates(
        scope === 'project' && currentProjectId ? { projectId: currentProjectId } : { scope: 'global' },
      );
      setTemplates(list);
      setDraft(list.length > 0 ? toDraft(list[0]) : { ...EMPTY_DRAFT, scope });
      setSelectedId(null);
      show('模板已删除', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '删除模板失败', 'error');
    }
  };

  const handlePreview = async () => {
    if (!datasetId) {
      show('请选择用于预览的数据集', 'error');
      return;
    }
    setBusy(true);
    try {
      const blob = await previewPdfTemplate(buildPayload(), datasetId, 60);
      saveBlob(blob, `${draft.name}_预览.pdf`);
      show('已生成预览 PDF（仅第 1 页）', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '生成预览失败', 'error');
    } finally {
      setBusy(false);
    }
  };

  const handleRender = async () => {
    if (!draft.id) {
      show('请先保存模板后再导出完整 PDF', 'error');
      return;
    }
    if (!datasetId) {
      show('请选择数据集', 'error');
      return;
    }
    setBusy(true);
    try {
      const blob = await renderPdfTemplate(draft.id, datasetId);
      saveBlob(blob, `${draft.name}.pdf`);
      show('已导出 PDF', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '导出失败', 'error');
    } finally {
      setBusy(false);
    }
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
    <div className="page">
      <div className="page-header">
        <div>
          <h2 className="page-title">PDF 模板</h2>
          <p className="page-subtitle">
            以 PPT 式可交互画布插入参数化组件；模板可保存到全局库，其他考试项目引用后会立即复制一份本项目副本。
          </p>
        </div>
        <div className="row-actions">
          <div className="segmented">
            <button
              className={scope === 'project' ? 'seg active' : 'seg'}
              onClick={() => setScope('project')}
              disabled={!currentProjectId}
              title={currentProjectId ? '本项目专用模板' : '请先在顶部选择考试项目'}
            >
              本项目{currentProject ? `（${currentProject.name}）` : ''}
            </button>
            <button
              className={scope === 'global' ? 'seg active' : 'seg'}
              onClick={() => setScope('global')}
            >
              全局模板库
            </button>
          </div>
          <select value={draft.id ?? ''} onChange={(event) => selectTemplate(event.target.value)}>
            <option value="">＋ 新建模板</option>
            {templates.map((template) => (
              <option key={template.id} value={template.id}>
                {template.name}
              </option>
            ))}
          </select>
          <button className="btn btn-primary" onClick={handleSave} disabled={saving}>
            {saving ? '保存中…' : '保存模板'}
          </button>
          {scope === 'global' && draft.id ? (
            <button className="btn" onClick={handleDuplicate} disabled={!currentProjectId}>
              引用到本项目
            </button>
          ) : null}
          <button className="btn btn-danger" onClick={handleDelete}>
            删除
          </button>
        </div>
      </div>

      <div className="split-3">
        {/* --------------- 模板设置 --------------- */}
        <aside className="card flow-panel">
          <h3 className="card-title">模板设置</h3>
          <label className="field">
            <span className="field-label">模板名称</span>
            <input
              type="text"
              value={draft.name}
              onChange={(event) => setDraft((prev) => ({ ...prev, name: event.target.value }))}
            />
          </label>
          <label className="field">
            <span className="field-label">模板类型</span>
            <select
              value={draft.kind}
              onChange={(event) =>
                setDraft((prev) => ({ ...prev, kind: event.target.value as 'single' | 'matrix' }))
              }
            >
              <option value="single">单人单页（每行数据一页）</option>
              <option value="matrix">矩阵（每页铺满多行数据）</option>
            </select>
          </label>
          <label className="field">
            <span className="field-label">说明</span>
            <input
              type="text"
              value={draft.description}
              onChange={(event) => setDraft((prev) => ({ ...prev, description: event.target.value }))}
            />
          </label>

          <hr className="divider" />
          <h4 className="panel-subtitle">页面</h4>
          <div className="grid-2">
            <label className="field">
              <span className="field-label">纸张</span>
              <select
                value={draft.page.size}
                onChange={(event) =>
                  setDraft((prev) => ({ ...prev, page: { ...prev.page, size: event.target.value as PageSetup['size'] } }))
                }
              >
                {(['A4', 'A3', 'A5', 'Letter'] as const).map((item) => (
                  <option key={item} value={item}>
                    {item}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span className="field-label">方向</span>
              <select
                value={draft.page.orientation}
                onChange={(event) =>
                  setDraft((prev) => ({
                    ...prev,
                    page: { ...prev.page, orientation: event.target.value as PageSetup['orientation'] },
                  }))
                }
              >
                <option value="portrait">纵向</option>
                <option value="landscape">横向</option>
              </select>
            </label>
          </div>
          <div className="grid-4">
            {(['top', 'right', 'bottom', 'left'] as const).map((side) => (
              <label key={side} className="field">
                <span className="field-label">边距{side}</span>
                <input
                  type="number"
                  step={0.5}
                  value={draft.page.margin_mm[side]}
                  onChange={(event) =>
                    setDraft((prev) => ({
                      ...prev,
                      page: {
                        ...prev.page,
                        margin_mm: { ...prev.page.margin_mm, [side]: Number(event.target.value) || 0 },
                      },
                    }))
                  }
                />
              </label>
            ))}
          </div>

          {draft.kind === 'matrix' ? (
            <>
              <hr className="divider" />
              <h4 className="panel-subtitle">矩阵</h4>
              <div className="grid-2">
                <NumberField
                  label="行数"
                  value={draft.matrix.rows}
                  onChange={(value) => setDraft((prev) => ({ ...prev, matrix: { ...prev.matrix, rows: value } }))}
                />
                <NumberField
                  label="列数"
                  value={draft.matrix.cols}
                  onChange={(value) => setDraft((prev) => ({ ...prev, matrix: { ...prev.matrix, cols: value } }))}
                />
                <NumberField
                  label="水平间距(mm)"
                  value={draft.matrix.gap_x_mm}
                  step={0.5}
                  onChange={(value) => setDraft((prev) => ({ ...prev, matrix: { ...prev.matrix, gap_x_mm: value } }))}
                />
                <NumberField
                  label="垂直间距(mm)"
                  value={draft.matrix.gap_y_mm}
                  step={0.5}
                  onChange={(value) => setDraft((prev) => ({ ...prev, matrix: { ...prev.matrix, gap_y_mm: value } }))}
                />
                <NumberField
                  label="单元格高(mm)"
                  value={draft.matrix.cell_h_mm}
                  step={1}
                  onChange={(value) => setDraft((prev) => ({ ...prev, matrix: { ...prev.matrix, cell_h_mm: value } }))}
                />
                <NumberField
                  label="网格顶部偏移(mm)"
                  value={draft.matrix.origin_y_mm ?? 0}
                  step={1}
                  onChange={(value) => setDraft((prev) => ({ ...prev, matrix: { ...prev.matrix, origin_y_mm: value } }))}
                />
              </div>
              <p className="panel-hint">每页可容纳 {Math.max(1, Math.round(draft.matrix.rows)) * Math.max(1, Math.round(draft.matrix.cols))} 条数据</p>
            </>
          ) : null}
        </aside>

        {/* --------------- 画布 --------------- */}
        <section className="card flow-canvas-card">
          <div className="flow-canvas-toolbar">
            <span className="detail-meta">
              纸张 {page.w}×{page.h} mm · 缩放 {scale.toFixed(2)}×
            </span>
            <span className="detail-meta">
              {draft.kind === 'matrix' ? '蓝色格为可编辑单元格，其余为预览' : '拖动元素调整位置'}
            </span>
          </div>
          <div className="flow-canvas" ref={hostRef} />
        </section>

        {/* --------------- 元素面板 --------------- */}
        <aside className="card flow-panel flow-props">
          <h3 className="card-title">模板元素</h3>
          <div className="chip-row">
            {(['text', 'field', 'image', 'line', 'rect'] as ElementType[]).map((type) => (
              <button key={type} type="button" className="chip" onClick={() => addElement(type)}>
                ＋ {ELEMENT_LABELS[type]}
              </button>
            ))}
          </div>
          <p className="panel-hint">
            文本中可用 {'{字段名}'} 插入数据；{'{{index}}'}、{'{{page}}'}、{'{{count}}'} 为内置变量。
          </p>

          <hr className="divider" />

          {selected ? (
            <ElementEditor
              element={selected}
              kind={draft.kind}
              columns={columnNames}
              onChange={(patch) => updateElement(selected.id, patch)}
              onRemove={() => removeElement(selected.id)}
            />
          ) : (
            <EmptyState text="选中画布中的元素以编辑其属性" />
          )}

          <hr className="divider" />
          <h4 className="panel-subtitle">导出</h4>
          <label className="field">
            <span className="field-label">数据来源</span>
            <select value={datasetId} onChange={(event) => setDatasetId(event.target.value)}>
              <option value="">选择数据集…</option>
              {datasets.map((dataset) => (
                <option key={dataset.id} value={dataset.id}>
                  {dataset.name}（{dataset.row_count} 行）
                </option>
              ))}
            </select>
          </label>
          <div className="row-actions">
            <button className="btn" onClick={handlePreview} disabled={busy}>
              预览第 1 页
            </button>
            <button className="btn btn-primary" onClick={handleRender} disabled={busy}>
              导出完整 PDF
            </button>
          </div>
        </aside>
      </div>
    </div>
  );
}

const ELEMENT_LABELS: Record<ElementType, string> = {
  text: '文本',
  field: '字段',
  image: '图片',
  line: '线条',
  rect: '矩形',
};

function toDraft(template: PdfTemplate): TemplateDraft {
  return {
    id: template.id,
    name: template.name,
    kind: template.kind === 'matrix' ? 'matrix' : 'single',
    description: template.description,
    page: template.page,
    matrix: template.matrix,
    elements: template.elements,
    scope: template.scope === 'project' ? 'project' : 'global',
  };
}

// --------------------------------------------------------------------
// 元素属性编辑器
// --------------------------------------------------------------------

interface ElementEditorProps {
  element: TemplateElement;
  kind: 'single' | 'matrix';
  columns: string[];
  onChange: (patch: Partial<TemplateElement>) => void;
  onRemove: () => void;
}

function ElementEditor({ element, kind, columns, onChange, onRemove }: ElementEditorProps) {
  const numeric: [string, keyof TemplateElement, number][] = [
    ['X (mm)', 'x_mm', 0.5],
    ['Y (mm)', 'y_mm', 0.5],
    ['宽 (mm)', 'w_mm', 0.5],
    ['高 (mm)', 'h_mm', 0.5],
  ];
  return (
    <>
      <div className="step-head">
        <span className="step-badge" style={{ background: '#2563eb' }}>
          ▦
        </span>
        <div className="step-head-info">
          <div className="step-head-title">{ELEMENT_LABELS[element.type]}</div>
          <div className="detail-meta mono">{element.type}</div>
        </div>
      </div>

      <div className="grid-2">
        {numeric.map(([label, key, step]) => (
          <NumberField
            key={key}
            label={label}
            step={step}
            value={Number(element[key] ?? 0)}
            onChange={(value) => onChange({ [key]: value } as Partial<TemplateElement>)}
          />
        ))}
      </div>

      {element.type === 'text' || element.type === 'field' ? (
        <label className="field">
          <span className="field-label">文本内容</span>
          <textarea
            rows={2}
            value={element.text}
            placeholder="支持 {字段名} 占位符；留空则直接显示字段值"
            onChange={(event) => onChange({ text: event.target.value })}
          />
        </label>
      ) : null}

      {element.type === 'field' || element.type === 'image' ? (
        <label className="field">
          <span className="field-label">绑定字段</span>
          <input
            type="text"
            list="template-field-options"
            value={element.field}
            onChange={(event) => onChange({ field: event.target.value })}
          />
          <datalist id="template-field-options">
            {columns.map((column) => (
              <option key={column} value={column} />
            ))}
          </datalist>
        </label>
      ) : null}

      {element.type !== 'line' && element.type !== 'image' ? (
        <>
          <div className="grid-2">
            <NumberField
              label="字号(pt)"
              step={1}
              value={element.font_size}
              onChange={(value) => onChange({ font_size: value })}
            />
            <label className="field">
              <span className="field-label">对齐</span>
              <select
                value={element.align}
                onChange={(event) => onChange({ align: event.target.value as TemplateElement['align'] })}
              >
                <option value="left">左</option>
                <option value="center">居中</option>
                <option value="right">右</option>
              </select>
            </label>
          </div>
          <label className="checkbox-row">
            <input type="checkbox" checked={element.bold} onChange={(event) => onChange({ bold: event.target.checked })} />
            加粗
          </label>
        </>
      ) : null}

      <div className="grid-2">
        <label className="field">
          <span className="field-label">字体/边框颜色</span>
          <input type="color" value={toHex(element.color)} onChange={(event) => onChange({ color: event.target.value })} />
        </label>
        <label className="field">
          <span className="field-label">填充色</span>
          <div className="color-row">
            <input type="color" value={toHex(element.fill)} onChange={(event) => onChange({ fill: event.target.value })} />
            <button type="button" className="btn btn-sm" onClick={() => onChange({ fill: '' })}>
              清除
            </button>
          </div>
        </label>
      </div>

      <label className="checkbox-row">
        <input type="checkbox" checked={element.border} onChange={(event) => onChange({ border: event.target.checked })} />
        显示边框
      </label>

      <NumberField
        label="圆角(mm)"
        step={0.5}
        value={element.border_radius_mm}
        onChange={(value) => onChange({ border_radius_mm: value })}
      />

      {kind === 'matrix' ? (
        <label className="checkbox-row">
          <input
            type="checkbox"
            checked={element.page_level}
            onChange={(event) => onChange({ page_level: event.target.checked })}
          />
          整页元素（每页只画一次，如标题）
        </label>
      ) : null}

      <div className="row-actions">
        <button className="btn btn-sm btn-danger" onClick={onRemove}>
          删除元素
        </button>
      </div>
    </>
  );
}

function toHex(color: string): string {
  if (/^#[0-9a-fA-F]{6}$/.test(color)) return color;
  return '#000000';
}

interface NumberFieldProps {
  label: string;
  value: number;
  step?: number;
  onChange: (value: number) => void;
}

function NumberField({ label, value, step = 1, onChange }: NumberFieldProps) {
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      <input
        type="number"
        step={step}
        value={Number.isFinite(value) ? value : 0}
        onChange={(event) => onChange(Number(event.target.value) || 0)}
      />
    </label>
  );
}
