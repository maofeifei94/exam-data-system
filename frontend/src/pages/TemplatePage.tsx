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
import { TokenEditor } from '../components/TokenEditor';
import { useToast } from '../components/Toast';
import { useProject } from '../context/ProjectContext';
import { fieldColor, hexToRgba, splitTemplateTokens } from '../lib/tokens';

const MM_PER_PT = 0.352778;
const PAD = 22;

type ElementType = TemplateElement['type'];

type Selection = { type: 'page' } | { type: 'matrix' } | { type: 'element'; id: string } | null;

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
  show_grid: true,
  show_border: true,
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

const ELEMENT_LABELS: Record<ElementType, string> = {
  text: '文本',
  field: '字段',
  image: '图片',
  line: '线条',
  rect: '矩形',
};

const ELEMENT_ICONS: Record<ElementType, string> = {
  text: 'T',
  field: '⌗',
  image: '▣',
  line: '╱',
  rect: '▭',
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

/** 近似文本宽度：中文按 1 个字号，ASCII 按 0.55，其他按 0.85。 */
function measureTextWidth(text: string, fontSize: number): number {
  let units = 0;
  for (const ch of text) {
    const code = ch.charCodeAt(0);
    if (code >= 0x2e80) units += 1;
    else if (code < 0x80) units += 0.55;
    else units += 0.85;
  }
  return units * fontSize;
}

interface RichItem {
  kind: 'text' | 'field' | 'builtin';
  text: string;
  x: number;
  w: number;
  y: number;
}

/**
 * 把模板文本渲染为“字段胶囊 + 文本”的可视化内容。
 * 展示态不出现 `{{}}`，字段以有颜色的圆角矩形封装。
 */
function buildRichContent(
  content: string,
  width: number,
  height: number,
  fontSize: number,
  element: TemplateElement,
): Array<Rect | Text> {
  const lineHeight = fontSize * 1.45;
  const pad = Math.max(1, fontSize * 0.15);
  const items: RichItem[] = [];
  let cursorX = 0;
  let cursorY = 0;
  let run = '';
  let runX = 0;

  const flush = () => {
    if (!run) return;
    items.push({ kind: 'text', text: run, x: runX, w: measureTextWidth(run, fontSize), y: cursorY });
    run = '';
  };

  for (const token of splitTemplateTokens(content)) {
    if (token.kind === 'text') {
      const lines = token.text.split('\n');
      lines.forEach((line, lineIndex) => {
        if (lineIndex > 0) {
          flush();
          cursorX = 0;
          cursorY += lineHeight;
        }
        for (const ch of line) {
          const charWidth = measureTextWidth(ch, fontSize);
          if (cursorX + charWidth > width - pad * 2 && cursorX > 0) {
            flush();
            cursorX = 0;
            cursorY += lineHeight;
          }
          if (!run) runX = cursorX;
          run += ch;
          cursorX += charWidth;
        }
      });
      continue;
    }

    flush();
    const label = token.field || '空字段';
    const pillFontSize = Math.max(6, fontSize * 0.82);
    const pillWidth = measureTextWidth(label, pillFontSize) + pillFontSize * 1.25;
    if (cursorX + pillWidth > width - pad * 2 && cursorX > 0) {
      cursorX = 0;
      cursorY += lineHeight;
    }
    items.push({ kind: token.kind, text: label, x: cursorX, w: pillWidth, y: cursorY });
    cursorX += pillWidth + pillFontSize * 0.28;
  }
  flush();

  if (items.length === 0) return [];

  const lineWidths = new Map<number, number>();
  for (const item of items) {
    lineWidths.set(item.y, Math.max(lineWidths.get(item.y) ?? 0, item.x + item.w));
  }
  const contentHeight = Math.max(...items.map((item) => item.y)) + lineHeight;
  const top = Math.max(0, (height - contentHeight) / 2);
  const output: Array<Rect | Text> = [];

  for (const item of items) {
    const lineWidth = lineWidths.get(item.y) ?? item.w;
    const alignOffset =
      element.align === 'center'
        ? (width - lineWidth) / 2
        : element.align === 'right'
          ? width - lineWidth
          : 0;
    const x = alignOffset + item.x + pad;
    const y = top + item.y;

    if (item.kind === 'text') {
      output.push(
        new Text({
          x,
          y,
          width: item.w + 2,
          height: lineHeight,
          text: item.text,
          fontSize,
          fill: element.color || '#000000',
          fontWeight: element.bold ? 'bold' : 'normal',
          textWrap: 'none',
        }),
      );
      continue;
    }

    const color = item.kind === 'builtin' ? '#64748b' : fieldColor(item.text);
    const pillHeight = lineHeight * 0.82;
    const pillY = y + (lineHeight - pillHeight) / 2;
    const pillFontSize = Math.max(6, fontSize * 0.82);
    output.push(
      new Rect({
        x,
        y: pillY,
        width: item.w,
        height: pillHeight,
        fill: hexToRgba(color, 0.13),
        stroke: hexToRgba(color, 0.4),
        strokeWidth: 1,
        cornerRadius: Math.max(3, pillHeight * 0.32),
      }),
      new Text({
        x,
        y: pillY,
        width: item.w,
        height: pillHeight,
        text: item.text,
        fontSize: pillFontSize,
        fill: color,
        fontWeight: 'bold',
        textAlign: 'center',
        verticalAlign: 'middle',
        textWrap: 'none',
      }),
    );
  }

  return output;
}

function saveBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = filename;
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
}

export default function TemplatePage({
  initialScope = 'global',
  initialTemplateId,
  onSaved,
}: {
  initialScope?: 'global' | 'project';
  initialTemplateId?: string;
  onSaved?: (template: PdfTemplate) => void;
}) {
  const { show } = useToast();
  const { currentProject, currentProjectId } = useProject();

  const [templates, setTemplates] = useState<PdfTemplate[]>([]);
  const [datasets, setDatasets] = useState<Dataset[]>([]);
  const [fields, setFields] = useState<StandardField[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [busy, setBusy] = useState(false);
  const [datasetId, setDatasetId] = useState('');
  const [scope, setScope] = useState<'project' | 'global'>(initialScope);

  const [draft, setDraft] = useState<TemplateDraft>(EMPTY_DRAFT);
  const [selection, setSelection] = useState<Selection>({ type: 'page' });
  /** 插入元素时的落点：矩阵模板可选择插入单元格或整页 */
  const [insertTarget, setInsertTarget] = useState<'cell' | 'page'>('cell');
  /** 画布中双击元素后，让属性面板的 Token 编辑器立即进入编辑态 */
  const [editRequestId, setEditRequestId] = useState<string | null>(null);
  const [size, setSize] = useState({ w: 620, h: 740 });

  const hostRef = useRef<HTMLDivElement | null>(null);
  const appRef = useRef<App | null>(null);

  const page = pageSizeMm(draft.page);
  const scale = useMemo(() => {
    const s = Math.min((size.w - PAD * 2) / page.w, (size.h - PAD * 2) / page.h);
    return Math.max(0.2, Math.min(4, s));
  }, [size, page.w, page.h]);

  const origin = useMemo(
    () => ({
      x: Math.max(PAD, (size.w - page.w * scale) / 2),
      y: Math.max(PAD, (size.h - page.h * scale) / 2),
    }),
    [size.w, page.w, scale, page.h, size.h],
  );

  const isMatrix = draft.kind === 'matrix';
  const selectedElement =
    selection?.type === 'element'
      ? draft.elements.find((element) => element.id === selection.id) ?? null
      : null;

  const cellElements = isMatrix
    ? draft.elements.filter((element) => !element.page_level)
    : draft.elements;
  const pageElements = isMatrix
    ? draft.elements.filter((element) => element.page_level)
    : [];

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
        const preferred =
          (initialTemplateId && list.find((item) => item.id === initialTemplateId)) || list[0];
        setDraft(preferred ? toDraft(preferred) : { ...EMPTY_DRAFT, scope });
        setSelection({ type: 'page' });
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
    background.on('tap', () => setSelection({ type: 'page' }));
    tree.add(background);

    // 纸张
    const paper = new Rect({
      x: origin.x,
      y: origin.y,
      width: page.w * scale,
      height: page.h * scale,
      fill: '#ffffff',
      stroke: '#cbd5e1',
      strokeWidth: 1,
    });
    paper.hittable = true;
    paper.on('tap', () => setSelection({ type: 'page' }));
    tree.add(paper);

    const m = draft.page.margin_mm;
    const contentX = origin.x + m.left * scale;
    const contentY = origin.y + m.top * scale;
    const contentW = Math.max(10, (page.w - m.left - m.right) * scale);
    const contentH = Math.max(10, (page.h - m.top - m.bottom) * scale);

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

    const isElementSelected = (id: string) => selection?.type === 'element' && selection.id === id;

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
      const selected = isElementSelected(element.id);
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
        children.push(new Rect({ x: 0, y: 0, width: w, height: Math.max(1, h), fill: element.color || '#000000' }));
      } else if (element.type === 'image') {
        children.push(
          new Rect({ x: 0, y: 0, width: w, height: h, fill: element.fill || '#f1f5f9', stroke: '#cbd5e1', strokeWidth: 1 }),
          new Text({
            x: 0,
            y: h / 2 - fs / 2,
            width: w,
            text: `▣ ${element.field || '照片'}`,
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
        const content = element.text.trim()
          ? element.text
          : element.field
            ? `{{${element.field}}}`
            : '';
        children.push(...buildRichContent(content || '（空）', w, h, fs, element));
      }

      if (interactive) {
        children.push(
          new Rect({
            x: 0,
            y: 0,
            width: w,
            height: h,
            stroke: selected ? '#0f766e' : '#94a3b8',
            strokeWidth: selected ? 2 : 1,
            dashPattern: selected ? undefined : [4, 3],
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
        group.on('tap', () => {
          setSelection({ type: 'element', id: element.id });
          setEditRequestId(null);
        });
        group.on('double_tap', () => {
          setSelection({ type: 'element', id: element.id });
          setEditRequestId(element.id);
        });
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
        group.opacity = 0.4;
      }

      return group;
    };

    if (isMatrix) {
      const mx = draft.matrix;
      const cols = Math.max(1, Math.round(mx.cols));
      const rows = Math.max(1, Math.round(mx.rows));
      const cellW = Math.max(4, (contentW - mx.gap_x_mm * scale * (cols - 1)) / cols);
      const cellH =
        mx.cell_h_mm > 0
          ? mx.cell_h_mm * scale
          : Math.max(4, (contentH - mx.origin_y_mm * scale - mx.gap_y_mm * scale * (rows - 1)) / rows);
      const gapX = mx.gap_x_mm * scale;
      const gapY = mx.gap_y_mm * scale;

      const cellOrigin = (r: number, c: number) => ({
        x: contentX + c * (cellW + gapX),
        y: contentY + mx.origin_y_mm * scale + r * (cellH + gapY),
      });

      // 网格辅助线：可单独关闭，仅影响编辑器
      if (mx.show_grid) {
        for (let r = 0; r < rows; r += 1) {
          for (let c = 0; c < cols; c += 1) {
            const o = cellOrigin(r, c);
            tree.add(
              new Rect({
                x: o.x,
                y: o.y,
                width: cellW,
                height: cellH,
                fill: r === 0 && c === 0 ? 'rgba(15,118,110,0.06)' : 'rgba(15,118,110,0.015)',
                stroke: '#d7e0da',
                strokeWidth: 1,
                dashPattern: [3, 3],
              }),
            );
          }
        }
      }

      const matrixW = cols * cellW + (cols - 1) * gapX;
      const matrixH = rows * cellH + (rows - 1) * gapY;
      const matrixSelected = selection?.type === 'matrix';

      // 矩阵外轮廓：整块可点击选中
      const outline = new Rect({
        x: contentX,
        y: contentY + mx.origin_y_mm * scale,
        width: matrixW,
        height: matrixH,
        fill: 'rgba(0,0,0,0.001)',
        stroke: matrixSelected ? '#0f766e' : '#9aa8a0',
        strokeWidth: matrixSelected ? 2 : 1,
        dashPattern: matrixSelected ? undefined : [6, 4],
      });
      outline.on('tap', () => setSelection({ type: 'matrix' }));
      tree.add(outline);

      // 单元格边框：可单独关闭，并同步影响 PDF 输出
      if (mx.show_border) {
        for (let r = 0; r < rows; r += 1) {
          for (let c = 0; c < cols; c += 1) {
            const o = cellOrigin(r, c);
            tree.add(
              new Rect({
                x: o.x,
                y: o.y,
                width: cellW,
                height: cellH,
                stroke: '#c3cdc6',
                strokeWidth: 1,
              }),
            );
          }
        }
      }

      // 第一格可编辑，其余格作为预览
      const first = cellOrigin(0, 0);
      cellElements.forEach((element) => tree.add(buildElement(element, first.x, first.y, true)));
      for (let r = 0; r < rows; r += 1) {
        for (let c = 0; c < cols; c += 1) {
          if (r === 0 && c === 0) continue;
          const o = cellOrigin(r, c);
          cellElements.forEach((element) => tree.add(buildElement(element, o.x, o.y, false)));
        }
      }

      pageElements.forEach((element) => tree.add(buildElement(element, contentX, contentY, true)));
    } else {
      draft.elements.forEach((element) => tree.add(buildElement(element, contentX, contentY, true)));
    }
  }, [draft, selection, size, scale, origin.x, origin.y, page.w, page.h, loading, isMatrix]);

  // ------------------------------------------------------------------
  // 操作
  // ------------------------------------------------------------------
  const updateElement = (id: string, patch: Partial<TemplateElement>) => {
    setDraft((prev) => ({
      ...prev,
      elements: prev.elements.map((element) => (element.id === id ? { ...element, ...patch } : element)),
    }));
  };

  const updateMatrix = (patch: Partial<MatrixSetup>) => {
    setDraft((prev) => ({ ...prev, matrix: { ...prev.matrix, ...patch } }));
  };

  const updatePage = (patch: Partial<PageSetup>) => {
    setDraft((prev) => ({ ...prev, page: { ...prev.page, ...patch } }));
  };

  const addElement = (type: ElementType) => {
    const pageLevel = isMatrix && insertTarget === 'page';
    const element = defaultElement(type, columnNames[0] ?? '', pageLevel);
    const index = draft.elements.length;
    // 轻微错位，避免新元素完全重叠
    element.x_mm = round1(6 + (index % 5) * 4);
    element.y_mm = round1(6 + (index % 7) * 5);
    setDraft((prev) => ({ ...prev, elements: [...prev.elements, element] }));
    setSelection({ type: 'element', id: element.id });
    setEditRequestId(null);
  };

  const removeElement = (id: string) => {
    setDraft((prev) => ({ ...prev, elements: prev.elements.filter((element) => element.id !== id) }));
    if (selection?.type === 'element' && selection.id === id) setSelection({ type: 'page' });
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
      setSelection({ type: 'page' });
      return;
    }
    const template = templates.find((item) => item.id === id);
    if (!template) return;
    setDraft(toDraft(template));
    setSelection({ type: 'page' });
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
      onSaved?.(saved);
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
      setSelection({ type: 'page' });
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
      setSelection({ type: 'page' });
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

  const elementLabel = (element: TemplateElement): string => {
    if (element.type === 'text') {
      const text = element.text.replace(/\{\{([^}]*)\}\}/g, '$1').trim();
      return text ? (text.length > 16 ? `${text.slice(0, 16)}…` : text) : '（空文本）';
    }
    return element.field || ELEMENT_LABELS[element.type];
  };

  const elementRow = (element: TemplateElement) => (
    <button
      key={element.id}
      className={`tpl-tree-row depth-2 ${
        selection?.type === 'element' && selection.id === element.id ? 'active' : ''
      }`}
      onClick={() => setSelection({ type: 'element', id: element.id })}
      title={elementLabel(element)}
    >
      <span className="tpl-tree-icon">{ELEMENT_ICONS[element.type]}</span>
      <span className="tpl-tree-label">{elementLabel(element)}</span>
    </button>
  );

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
            左侧为页面结构树，中间为画布，右侧编辑当前选中节点；顶部工具栏插入元素。
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

      <div className="tpl-shell">
        {/* --------------- 顶部工具栏（WPS 式） --------------- */}
        <div className="tpl-toolbar">
          <div className="tpl-tool-group">
            {(['text', 'field', 'image', 'line', 'rect'] as ElementType[]).map((type) => (
              <button key={type} type="button" className="tpl-tool" onClick={() => addElement(type)}>
                <span className="tpl-tool-icon">{ELEMENT_ICONS[type]}</span>
                {ELEMENT_LABELS[type]}
              </button>
            ))}
          </div>

          {isMatrix ? (
            <>
              <span className="tpl-tool-sep" />
              <div className="segmented compact">
                <button
                  className={insertTarget === 'cell' ? 'seg active' : 'seg'}
                  onClick={() => setInsertTarget('cell')}
                >
                  插入单元格
                </button>
                <button
                  className={insertTarget === 'page' ? 'seg active' : 'seg'}
                  onClick={() => setInsertTarget('page')}
                >
                  插入整页
                </button>
              </div>
              <label className="tpl-toggle">
                <input
                  type="checkbox"
                  checked={draft.matrix.show_grid}
                  onChange={(event) => updateMatrix({ show_grid: event.target.checked })}
                />
                网格
              </label>
              <label className="tpl-toggle">
                <input
                  type="checkbox"
                  checked={draft.matrix.show_border}
                  onChange={(event) => updateMatrix({ show_border: event.target.checked })}
                />
                边框
              </label>
            </>
          ) : null}

          <div className="toolbar-fill" />

          {selectedElement ? (
            <button className="tpl-tool danger" onClick={() => removeElement(selectedElement.id)}>
              删除元素
            </button>
          ) : null}
          <button className="btn btn-sm btn-primary" onClick={handleSave} disabled={saving}>
            {saving ? '保存中…' : '保存模板'}
          </button>
        </div>

        <div className="tpl-main">
          {/* --------------- 左：页面结构树 --------------- */}
          <aside className="tpl-tree card">
            <div className="tpl-panel-head">页面结构</div>
            <div className="tpl-tree-body">
              <button
                className={`tpl-tree-row ${selection?.type === 'page' ? 'active' : ''}`}
                onClick={() => setSelection({ type: 'page' })}
              >
                <span className="tpl-tree-icon">▤</span>
                <span className="tpl-tree-label">页面</span>
                <span className="tpl-tree-meta">
                  {draft.page.size} {draft.page.orientation === 'portrait' ? '纵' : '横'}
                </span>
              </button>

              {isMatrix ? (
                <>
                  <div className="tpl-tree-group">
                    <span className="tpl-tree-group-label">整页元素</span>
                    {pageElements.length === 0 ? (
                      <span className="tpl-tree-empty">空</span>
                    ) : (
                      pageElements.map(elementRow)
                    )}
                  </div>

                  <button
                    className={`tpl-tree-row depth-1 ${selection?.type === 'matrix' ? 'active' : ''}`}
                    onClick={() => setSelection({ type: 'matrix' })}
                  >
                    <span className="tpl-tree-icon">▦</span>
                    <span className="tpl-tree-label">矩阵</span>
                    <span className="tpl-tree-meta">
                      {Math.max(1, Math.round(draft.matrix.rows))}×
                      {Math.max(1, Math.round(draft.matrix.cols))}
                    </span>
                  </button>

                  <div className="tpl-tree-group">
                    <span className="tpl-tree-group-label">单元格元素</span>
                    {cellElements.length === 0 ? (
                      <span className="tpl-tree-empty">空</span>
                    ) : (
                      cellElements.map(elementRow)
                    )}
                  </div>
                </>
              ) : (
                <div className="tpl-tree-group">
                  <span className="tpl-tree-group-label">页面元素</span>
                  {cellElements.length === 0 ? (
                    <span className="tpl-tree-empty">空</span>
                  ) : (
                    cellElements.map(elementRow)
                  )}
                </div>
              )}
            </div>
            <p className="panel-hint">点击任意节点，在右侧编辑其属性。</p>
          </aside>

          {/* --------------- 中：画布 --------------- */}
          <section className="tpl-canvas-area">
            <div className="flow-canvas-toolbar">
              <span className="detail-meta">
                纸张 {page.w}×{page.h} mm · 缩放 {scale.toFixed(2)}×
              </span>
              <span className="detail-meta">
                {isMatrix ? '点击矩阵外轮廓选中矩阵，蓝色格为可编辑单元格' : '拖动元素调整位置，双击可编辑'}
              </span>
            </div>
            <div className="tpl-canvas" ref={hostRef} />
          </section>

          {/* --------------- 右：属性面板 --------------- */}
          <aside className="tpl-props card">
            {selection?.type === 'page' ? (
              <PagePanel
                draft={draft}
                datasets={datasets}
                datasetId={datasetId}
                busy={busy}
                onDataset={setDatasetId}
                onDraft={setDraft}
                onPage={updatePage}
                onPreview={handlePreview}
                onRender={handleRender}
              />
            ) : selection?.type === 'matrix' ? (
              <MatrixPanel matrix={draft.matrix} onChange={updateMatrix} />
            ) : selectedElement ? (
              <ElementEditor
                element={selectedElement}
                kind={draft.kind}
                columns={columnNames}
                autoEdit={editRequestId === selectedElement.id}
                onChange={(patch) => updateElement(selectedElement.id, patch)}
                onRemove={() => removeElement(selectedElement.id)}
              />
            ) : (
              <EmptyState text="在左侧结构树或画布中选择节点" />
            )}
          </aside>
        </div>
      </div>
    </div>
  );
}

function toDraft(template: PdfTemplate): TemplateDraft {
  return {
    id: template.id,
    name: template.name,
    kind: template.kind === 'matrix' ? 'matrix' : 'single',
    description: template.description,
    page: template.page,
    matrix: {
      ...DEFAULT_MATRIX,
      ...(template.matrix ?? {}),
      show_grid: template.matrix?.show_grid ?? true,
      show_border: template.matrix?.show_border ?? true,
    },
    elements: template.elements,
    scope: template.scope === 'project' ? 'project' : 'global',
  };
}

// --------------------------------------------------------------------
// 右侧：页面属性
// --------------------------------------------------------------------

interface PagePanelProps {
  draft: TemplateDraft;
  datasets: Dataset[];
  datasetId: string;
  busy: boolean;
  onDataset: (id: string) => void;
  onDraft: (updater: (prev: TemplateDraft) => TemplateDraft) => void;
  onPage: (patch: Partial<PageSetup>) => void;
  onPreview: () => void;
  onRender: () => void;
}

function PagePanel({ draft, datasets, datasetId, busy, onDataset, onDraft, onPage, onPreview, onRender }: PagePanelProps) {
  return (
    <>
      <div className="tpl-panel-head">
        <span className="tpl-tree-icon">▤</span> 页面
      </div>

      <label className="field">
        <span className="field-label">模板名称</span>
        <input
          type="text"
          value={draft.name}
          onChange={(event) => onDraft((prev) => ({ ...prev, name: event.target.value }))}
        />
      </label>
      <label className="field">
        <span className="field-label">模板类型</span>
        <select
          value={draft.kind}
          onChange={(event) =>
            onDraft((prev) => ({ ...prev, kind: event.target.value as 'single' | 'matrix' }))
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
          onChange={(event) => onDraft((prev) => ({ ...prev, description: event.target.value }))}
        />
      </label>

      <hr className="divider" />
      <div className="grid-2">
        <label className="field">
          <span className="field-label">纸张</span>
          <select
            value={draft.page.size}
            onChange={(event) => onPage({ size: event.target.value as PageSetup['size'] })}
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
            onChange={(event) => onPage({ orientation: event.target.value as PageSetup['orientation'] })}
          >
            <option value="portrait">纵向</option>
            <option value="landscape">横向</option>
          </select>
        </label>
      </div>
      <div className="grid-4">
        {(['top', 'right', 'bottom', 'left'] as const).map((side) => (
          <label key={side} className="field">
            <span className="field-label">
              {side === 'top' ? '上' : side === 'right' ? '右' : side === 'bottom' ? '下' : '左'}边距
            </span>
            <input
              type="number"
              step={0.5}
              value={draft.page.margin_mm[side]}
              onChange={(event) =>
                onPage({
                  margin_mm: { ...draft.page.margin_mm, [side]: Number(event.target.value) || 0 },
                })
              }
            />
          </label>
        ))}
      </div>

      <hr className="divider" />
      <label className="field">
        <span className="field-label">预览数据源</span>
        <select value={datasetId} onChange={(event) => onDataset(event.target.value)}>
          <option value="">选择数据集…</option>
          {datasets.map((dataset) => (
            <option key={dataset.id} value={dataset.id}>
              {dataset.name}（{dataset.row_count} 行）
            </option>
          ))}
        </select>
      </label>
      <div className="row-actions">
        <button className="btn" onClick={onPreview} disabled={busy}>
          预览第 1 页
        </button>
        <button className="btn btn-primary" onClick={onRender} disabled={busy}>
          导出完整 PDF
        </button>
      </div>
    </>
  );
}

// --------------------------------------------------------------------
// 右侧：矩阵属性
// --------------------------------------------------------------------

function MatrixPanel({ matrix, onChange }: { matrix: MatrixSetup; onChange: (patch: Partial<MatrixSetup>) => void }) {
  return (
    <>
      <div className="tpl-panel-head">
        <span className="tpl-tree-icon">▦</span> 矩阵
      </div>
      <p className="panel-hint">矩阵内部按行列自动划分网格，单元格大小由行数与间距推算。</p>
      <div className="grid-2">
        <NumberField label="行数" value={matrix.rows} onChange={(value) => onChange({ rows: value })} />
        <NumberField label="列数" value={matrix.cols} onChange={(value) => onChange({ cols: value })} />
        <NumberField
          label="水平间距(mm)"
          value={matrix.gap_x_mm}
          step={0.5}
          onChange={(value) => onChange({ gap_x_mm: value })}
        />
        <NumberField
          label="垂直间距(mm)"
          value={matrix.gap_y_mm}
          step={0.5}
          onChange={(value) => onChange({ gap_y_mm: value })}
        />
        <NumberField
          label="单元格高(mm)"
          value={matrix.cell_h_mm}
          step={1}
          onChange={(value) => onChange({ cell_h_mm: value })}
        />
        <NumberField
          label="网格顶部偏移(mm)"
          value={matrix.origin_y_mm ?? 0}
          step={1}
          onChange={(value) => onChange({ origin_y_mm: value })}
        />
      </div>
      <label className="checkbox-row">
        <input
          type="checkbox"
          checked={matrix.show_grid}
          onChange={(event) => onChange({ show_grid: event.target.checked })}
        />
        显示网格
      </label>
      <label className="checkbox-row">
        <input
          type="checkbox"
          checked={matrix.show_border}
          onChange={(event) => onChange({ show_border: event.target.checked })}
        />
        显示边框（同时输出到 PDF）
      </label>
      <p className="panel-hint">
        每页可容纳{' '}
        {Math.max(1, Math.round(matrix.rows)) * Math.max(1, Math.round(matrix.cols))} 条数据
      </p>
    </>
  );
}

// --------------------------------------------------------------------
// 元素属性编辑器
// --------------------------------------------------------------------

interface ElementEditorProps {
  element: TemplateElement;
  kind: 'single' | 'matrix';
  columns: string[];
  autoEdit?: boolean;
  onChange: (patch: Partial<TemplateElement>) => void;
  onRemove: () => void;
}

function ElementEditor({ element, kind, columns, autoEdit, onChange, onRemove }: ElementEditorProps) {
  const numeric: [string, keyof TemplateElement, number][] = [
    ['X (mm)', 'x_mm', 0.5],
    ['Y (mm)', 'y_mm', 0.5],
    ['宽 (mm)', 'w_mm', 0.5],
    ['高 (mm)', 'h_mm', 0.5],
  ];
  return (
    <>
      <div className="tpl-panel-head">
        <span className="tpl-tree-icon">{ELEMENT_ICONS[element.type]}</span>
        {ELEMENT_LABELS[element.type]}
        <span className="tpl-tree-meta mono">{element.page_level ? '整页' : '单元格'}</span>
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
        <div className="field">
          <span className="field-label">文本内容</span>
          <TokenEditor
            mode="template"
            multiline
            fields={columns}
            value={element.text}
            autoEdit={autoEdit}
            placeholder="输入文本，并用 {{字段名}} 插入字段"
            onChange={(text) => onChange({ text })}
          />
          <span className="field-hint">
            双击胶囊或按 Backspace 可编辑原始 <span className="mono">{'{{字段}}'}</span> 语法
          </span>
        </div>
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
