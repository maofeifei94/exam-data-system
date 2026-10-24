import { useEffect, useMemo, useRef, useState } from 'react';
import type { PointerEvent as ReactPointerEvent } from 'react';
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
import {
  DEFAULT_MATRIX,
  appendChild,
  contentSizeMm,
  createElement,
  elementIcon,
  elementLabel,
  findElement,
  findPath,
  isContainer,
  normalizeTemplate,
  pageSizeMm,
  removeElement,
  resizeRect,
  round1,
  toParentBox,
  updateElement,
  walkElements,
} from '../lib/templateModel';
import type { ElementType, ResizeHandle } from '../lib/templateModel';
import { fieldColor, hexToRgba, splitTemplateTokens } from '../lib/tokens';
import { isDesktop, pickFolder } from '../lib/native';

const MM_PER_PT = 0.352778;
const PAD = 34;
const MIN_ZOOM = 0.25;
const MAX_ZOOM = 8;

/** 照片后缀下拉选项；顺序即查找优先级。 */
const PHOTO_EXT_OPTIONS = ['jpg', 'jpeg', 'png', 'bmp', 'webp', 'gif', 'tif', 'tiff'];

type Tool = 'select' | 'hand';
type Selection = { type: 'page' } | { type: 'element'; id: string };
type InsertTarget = 'container' | 'page';

interface TemplateDraft {
  id: string | null;
  name: string;
  kind: 'single' | 'matrix';
  description: string;
  page: PageSetup;
  elements: TemplateElement[];
  scope: 'global' | 'project';
}

interface ViewState {
  zoom: number;
  panX: number;
  panY: number;
}

const DEFAULT_PAGE: PageSetup = {
  size: 'A4',
  orientation: 'portrait',
  margin_mm: { top: 15, right: 12, bottom: 12, left: 12 },
};

const EMPTY_DRAFT: TemplateDraft = {
  id: null,
  name: '新模板',
  kind: 'single',
  description: '',
  page: DEFAULT_PAGE,
  elements: [],
  scope: 'global',
};

const ELEMENT_LABELS: Record<ElementType, string> = {
  text: '文本',
  field: '字段',
  image: '图片',
  line: '线条',
  rect: '矩形',
  matrix: '矩阵',
};

const INSERT_TYPES: ElementType[] = ['text', 'field', 'image', 'line', 'rect'];

type RibbonTab = 'home' | 'insert' | 'layout' | 'view';

const RIBBON_TABS: Array<{ id: RibbonTab; label: string }> = [
  { id: 'home', label: '开始' },
  { id: 'insert', label: '插入' },
  { id: 'layout', label: '页面布局' },
  { id: 'view', label: '视图' },
];

interface ResizeHandleSpec {
  handle: ResizeHandle;
  /** 顶点在选框中的相对位置（0 / 0.5 / 1） */
  fx: number;
  fy: number;
  cursor: string;
}

const RESIZE_HANDLES: ResizeHandleSpec[] = [
  { handle: 'nw', fx: 0, fy: 0, cursor: 'nwse-resize' },
  { handle: 'n', fx: 0.5, fy: 0, cursor: 'ns-resize' },
  { handle: 'ne', fx: 1, fy: 0, cursor: 'nesw-resize' },
  { handle: 'e', fx: 1, fy: 0.5, cursor: 'ew-resize' },
  { handle: 'se', fx: 1, fy: 1, cursor: 'nwse-resize' },
  { handle: 's', fx: 0.5, fy: 1, cursor: 'ns-resize' },
  { handle: 'sw', fx: 0, fy: 1, cursor: 'nesw-resize' },
  { handle: 'w', fx: 0, fy: 0.5, cursor: 'ew-resize' },
];

const HANDLE_SIZE = 8;

// --------------------------------------------------------------------
// 画布：把模板文本渲染成「文字 + 字段胶囊」
// --------------------------------------------------------------------

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

function buildRichContent(
  content: string,
  width: number,
  height: number,
  element: TemplateElement,
  strokeMm: number,
): Array<Rect | Text> {
  const fontSize = Math.max(0.6, element.font_size * MM_PER_PT);
  const lineHeight = fontSize * 1.45;
  const pad = Math.max(0.25, fontSize * 0.16);
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
    const pillFontSize = Math.max(0.55, fontSize * 0.82);
    const pillWidth = measureTextWidth(label, pillFontSize) + pillFontSize * 1.4;
    if (cursorX + pillWidth > width - pad * 2 && cursorX > 0) {
      cursorX = 0;
      cursorY += lineHeight;
    }
    items.push({ kind: token.kind, text: label, x: cursorX, w: pillWidth, y: cursorY });
    cursorX += pillWidth + pillFontSize * 0.3;
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
          width: item.w + 0.5,
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
    const pillFontSize = Math.max(0.55, fontSize * 0.82);
    output.push(
      new Rect({
        x,
        y: pillY,
        width: item.w,
        height: pillHeight,
        fill: hexToRgba(color, 0.14),
        stroke: hexToRgba(color, 0.45),
        strokeWidth: strokeMm,
        cornerRadius: pillHeight * 0.3,
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
  /** 画布中双击元素后，让属性面板的 Token 编辑器立即进入编辑态 */
  const [editRequestId, setEditRequestId] = useState<string | null>(null);
  const [insertTarget, setInsertTarget] = useState<InsertTarget>('container');
  const [tool, setTool] = useState<Tool>('select');
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [size, setSize] = useState({ w: 620, h: 740 });
  const [view, setView] = useState<ViewState>({ zoom: 1, panX: 0, panY: 0 });
  const [ribbonTab, setRibbonTab] = useState<RibbonTab>('home');
  /** Leafer 正在拖动的元素：隐藏 HTML 选框，避免选框停留在旧位置 */
  const [movingId, setMovingId] = useState<string | null>(null);
  /** 拖动顶点时的实时选框（页面坐标，mm）；提交前不写回草稿，避免画布频繁重建 */
  const [resizePreview, setResizePreview] = useState<{
    x: number;
    y: number;
    w: number;
    h: number;
  } | null>(null);
  const resizeDragRef = useRef<{
    pointerId: number;
    handle: ResizeHandle;
    startX: number;
    startY: number;
    box: { x: number; y: number; w: number; h: number };
    last: { x: number; y: number; w: number; h: number };
  } | null>(null);

  const hostRef = useRef<HTMLDivElement | null>(null);
  const appRef = useRef<App | null>(null);
  const viewportRef = useRef<Group | null>(null);
  const viewRef = useRef(view);
  viewRef.current = view;
  const panDragRef = useRef<{
    pointerId: number;
    startX: number;
    startY: number;
    panX: number;
    panY: number;
  } | null>(null);

  const page = pageSizeMm(draft.page);
  const content = contentSizeMm(draft.page);

  const fit = useMemo(() => {
    const availW = Math.max(60, size.w - PAD * 2);
    const availH = Math.max(60, size.h - PAD * 2);
    const scale = Math.max(0.15, Math.min(4, Math.min(availW / page.w, availH / page.h)));
    return { scale, tx: (size.w - page.w * scale) / 2, ty: (size.h - page.h * scale) / 2 };
  }, [size, page.w, page.h]);

  const scale = fit.scale * view.zoom;
  const translateX = fit.tx + view.panX;
  const translateY = fit.ty + view.panY;

  const matrixElement = useMemo(() => {
    let found: TemplateElement | null = null;
    walkElements(draft.elements, (located) => {
      if (!found && located.element.type === 'matrix') found = located.element;
    });
    return found as TemplateElement | null;
  }, [draft.elements]);

  const selectedLocated =
    selection.type === 'element' ? findElement(draft.elements, selection.id) : null;
  const selectedElement = selectedLocated?.element ?? null;
  const selectedPath =
    selection.type === 'element' ? findPath(draft.elements, selection.id) : [];
  const isMatrix = draft.kind === 'matrix';

  const matrixTarget = selectedElement?.type === 'matrix' ? selectedElement : matrixElement;
  const matrixConfig = useMemo<MatrixSetup>(
    () => ({ ...DEFAULT_MATRIX, ...(matrixTarget?.matrix ?? {}) }),
    [matrixTarget],
  );

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
        const source =
          (initialTemplateId && list.find((item) => item.id === initialTemplateId)) || list[0];
        setTemplates(list);
        setDraft(source ? toDraft(source) : { ...EMPTY_DRAFT, scope });
        setCollapsed(new Set());
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
  // 初始化 Leafer 画布
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
      viewportRef.current = null;
    };
    // 依赖 loading：加载期间画布容器尚未挂载，需等真实布局渲染后再创建画布。
  }, [loading]);

  useEffect(() => {
    const app = appRef.current;
    if (!app) return;
    app.resize({ width: size.w, height: size.h });
  }, [size]);

  // 切换模板 / 纸张尺寸时回到「适应窗口」
  useEffect(() => {
    setView({ zoom: 1, panX: 0, panY: 0 });
  }, [draft.id, page.w, page.h]);

  // 切换选中元素或工具时放弃未提交的缩放预览
  useEffect(() => {
    resizeDragRef.current = null;
    setResizePreview(null);
  }, [selection, tool]);

  // ------------------------------------------------------------------
  // Ctrl + 滚轮缩放；普通滚轮平移
  // ------------------------------------------------------------------
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      if (event.ctrlKey || event.metaKey) {
        const rect = host.getBoundingClientRect();
        const cx = event.clientX - rect.left;
        const cy = event.clientY - rect.top;
        const factor = Math.exp(-event.deltaY * 0.0015);
        setView((prev) => {
          const oldScale = fit.scale * prev.zoom;
          const nextZoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, prev.zoom * factor));
          const nextScale = fit.scale * nextZoom;
          const k = nextScale / oldScale;
          const oldTx = fit.tx + prev.panX;
          const oldTy = fit.ty + prev.panY;
          return {
            zoom: nextZoom,
            panX: cx - (cx - oldTx) * k - fit.tx,
            panY: cy - (cy - oldTy) * k - fit.ty,
          };
        });
        return;
      }
      setView((prev) => ({
        ...prev,
        panX: prev.panX - event.deltaX,
        panY: prev.panY - event.deltaY,
      }));
    };
    host.addEventListener('wheel', onWheel, { passive: false });
    return () => host.removeEventListener('wheel', onWheel);
  }, [fit.scale, fit.tx, fit.ty]);

  // ------------------------------------------------------------------
  // 构建 / 重建画布场景
  // ------------------------------------------------------------------
  useEffect(() => {
    const app = appRef.current;
    if (!app || !app.tree) return;
    const tree = app.tree;
    tree.removeAll();

    const strokeMm = 1 / fit.scale;
    const currentView = viewRef.current;
    const viewport = new Group({
      x: fit.tx + currentView.panX,
      y: fit.ty + currentView.panY,
      scale: fit.scale * currentView.zoom,
    });
    viewportRef.current = viewport;

    const background = new Rect({ x: 0, y: 0, width: size.w, height: size.h, fill: '#e8ecf2' });
    background.on('tap', () => setSelection({ type: 'page' }));
    tree.add(background);
    tree.add(viewport);

    const paper = new Rect({
      x: 0,
      y: 0,
      width: page.w,
      height: page.h,
      fill: '#ffffff',
      stroke: '#cbd5e1',
      strokeWidth: strokeMm,
      cornerRadius: 0.6,
    });
    paper.on('tap', () => setSelection({ type: 'page' }));
    viewport.add(paper);

    const m = draft.page.margin_mm;
    viewport.add(
      new Rect({
        x: m.left,
        y: m.top,
        width: Math.max(1, page.w - m.left - m.right),
        height: Math.max(1, page.h - m.top - m.bottom),
        stroke: '#cbd5e1',
        strokeWidth: strokeMm,
        dashPattern: [4 * strokeMm, 4 * strokeMm],
      }),
    );

    const buildElementNode = (element: TemplateElement, interactive: boolean): Group => {
      const w = Math.max(0.5, element.w_mm);
      const h = Math.max(0.5, element.h_mm);
      const radius = Math.max(0, element.border_radius_mm);

      const node = new Group({
        x: element.x_mm,
        y: element.y_mm,
        opacity: interactive ? 1 : 0.35,
        hittable: interactive,
        draggable: interactive && tool === 'select',
        cursor: interactive ? (tool === 'select' ? 'move' : 'default') : 'default',
      });

      node.add(
        new Rect({ x: 0, y: 0, width: w, height: h, fill: 'rgba(0,0,0,0.001)', hittable: interactive }),
      );

      if (interactive) {
        node.on('tap', (event: { stop: () => void }) => {
          event.stop();
          setSelection({ type: 'element', id: element.id });
          setEditRequestId(null);
        });
        node.on('double_tap', (event: { stop: () => void }) => {
          event.stop();
          setSelection({ type: 'element', id: element.id });
          setEditRequestId(element.id);
        });
        node.on('drag.start', () => setMovingId(element.id));
        node.on('drag.end', () => {
          const nx = round1(Number(node.x ?? element.x_mm));
          const ny = round1(Number(node.y ?? element.y_mm));
          setDraft((prev) => ({
            ...prev,
            elements: updateElement(prev.elements, element.id, { x_mm: nx, y_mm: ny }),
          }));
          setMovingId(null);
        });
      }

      if (element.type === 'matrix') {
        const mx = { ...DEFAULT_MATRIX, ...(element.matrix ?? {}) };
        const cols = Math.max(1, Math.round(mx.cols));
        const rows = Math.max(1, Math.round(mx.rows));
        const cellW = w / cols;
        const cellH = h / rows;

        if (element.fill) {
          node.add(new Rect({ x: 0, y: 0, width: w, height: h, fill: element.fill }));
        }
        if (mx.show_grid) {
          for (let c = 1; c < cols; c += 1) {
            node.add(
              new Rect({
                x: c * cellW - strokeMm / 2,
                y: 0,
                width: strokeMm,
                height: h,
                fill: '#cbd5e1',
                hittable: false,
              }),
            );
          }
          for (let r = 1; r < rows; r += 1) {
            node.add(
              new Rect({
                x: 0,
                y: r * cellH - strokeMm / 2,
                width: w,
                height: strokeMm,
                fill: '#cbd5e1',
                hittable: false,
              }),
            );
          }
        }
        for (let r = 0; r < rows; r += 1) {
          for (let c = 0; c < cols; c += 1) {
            if (element.children.length === 0) continue;
            const editableCell = interactive && r === 0 && c === 0;
            // 只有左上角单元格可交互；其余单元格仅作为铺排预览。
            // 注意 hittable 必须为 true，否则子元素无法接收拖拽事件。
            const cell = new Group({ x: c * cellW, y: r * cellH, hittable: editableCell });
            element.children.forEach((child) => cell.add(buildElementNode(child, editableCell)));
            node.add(cell);
          }
        }
        if (mx.show_border) {
          node.add(
            new Rect({
              x: 0,
              y: 0,
              width: w,
              height: h,
              stroke: element.color || '#475569',
              strokeWidth: strokeMm,
              cornerRadius: radius,
              hittable: false,
            }),
          );
        }
      } else if (element.type === 'rect') {
        node.add(
          new Rect({
            x: 0,
            y: 0,
            width: w,
            height: h,
            fill: element.fill || undefined,
            stroke: element.border ? element.color : undefined,
            strokeWidth: element.border ? strokeMm : 0,
            cornerRadius: radius,
            hittable: false,
          }),
        );
        element.children.forEach((child) => node.add(buildElementNode(child, interactive)));
      } else if (element.type === 'line') {
        node.add(
          new Rect({
            x: 0,
            y: 0,
            width: w,
            height: Math.max(0.25, h),
            fill: element.color || '#000000',
            hittable: false,
          }),
        );
      } else if (element.type === 'image') {
        node.add(
          new Rect({
            x: 0,
            y: 0,
            width: w,
            height: h,
            fill: element.fill || '#f1f5f9',
            stroke: element.border ? element.color : '#cbd5e1',
            strokeWidth: strokeMm,
            cornerRadius: radius,
            hittable: false,
          }),
        );
        node.add(
          new Text({
            x: 0,
            y: 0,
            width: w,
            height: h,
            text: `▣ ${(element.photo_name || element.field || '照片').replace(/\{\{?([^}]+)\}?\}/g, '$1')}`,
            fontSize: Math.min(element.font_size * MM_PER_PT, 3.2),
            fill: '#94a3b8',
            textAlign: 'center',
            verticalAlign: 'middle',
            hittable: false,
          }),
        );
      } else {
        if (element.fill) {
          node.add(new Rect({ x: 0, y: 0, width: w, height: h, fill: element.fill, cornerRadius: radius, hittable: false }));
        }
        if (element.border) {
          node.add(
            new Rect({
              x: 0,
              y: 0,
              width: w,
              height: h,
              stroke: element.color,
              strokeWidth: strokeMm,
              cornerRadius: radius,
              hittable: false,
            }),
          );
        }
        const text = element.text.trim()
          ? element.text
          : element.field
            ? `{{${element.field}}}`
            : '（空）';
        buildRichContent(text, w, h, element, strokeMm).forEach((item) => {
          item.hittable = false;
          node.add(item);
        });
        element.children.forEach((child) => node.add(buildElementNode(child, interactive)));
      }

      return node;
    };

    draft.elements.forEach((element) => viewport.add(buildElementNode(element, true)));
  }, [
    draft.elements,
    draft.page,
    selectedElement,
    tool,
    fit.scale,
    fit.tx,
    fit.ty,
    page.w,
    page.h,
    size.w,
    size.h,
  ]);

  // 缩放 / 平移只改变视口变换，不重建场景
  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    viewport.x = translateX;
    viewport.y = translateY;
    viewport.scale = scale;
  }, [translateX, translateY, scale]);

  // ------------------------------------------------------------------
  // 视图操作
  // ------------------------------------------------------------------
  const zoomAt = (cx: number, cy: number, factor: number) => {
    setView((prev) => {
      const oldScale = fit.scale * prev.zoom;
      const nextZoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, prev.zoom * factor));
      const nextScale = fit.scale * nextZoom;
      const k = nextScale / oldScale;
      const oldTx = fit.tx + prev.panX;
      const oldTy = fit.ty + prev.panY;
      return {
        zoom: nextZoom,
        panX: cx - (cx - oldTx) * k - fit.tx,
        panY: cy - (cy - oldTy) * k - fit.ty,
      };
    });
  };

  const resetView = () => setView({ zoom: 1, panX: 0, panY: 0 });

  const handleCanvasPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (tool !== 'hand' && event.button !== 1) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    panDragRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      panX: view.panX,
      panY: view.panY,
    };
    event.preventDefault();
  };

  const handleCanvasPointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = panDragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    setView((prev) => ({
      ...prev,
      panX: drag.panX + (event.clientX - drag.startX),
      panY: drag.panY + (event.clientY - drag.startY),
    }));
  };

  const handleCanvasPointerUp = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (panDragRef.current?.pointerId === event.pointerId) {
      panDragRef.current = null;
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
    }
  };

  // ------------------------------------------------------------------
  // 草稿操作
  // ------------------------------------------------------------------
  const patchElement = (id: string, patch: Partial<TemplateElement>) => {
    setDraft((prev) => ({ ...prev, elements: updateElement(prev.elements, id, patch) }));
  };

  // ------------------------------------------------------------------
  // 顶点拖拽缩放：对角点固定，相邻两条边随顶点移动
  // ------------------------------------------------------------------
  const beginResize = (handle: ResizeHandle, event: ReactPointerEvent<HTMLDivElement>) => {
    if (!selectedElement || !selectedLocated || tool !== 'select') return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    const box = {
      x: selectedLocated.x,
      y: selectedLocated.y,
      w: selectedElement.w_mm,
      h: selectedElement.h_mm,
    };
    resizeDragRef.current = {
      pointerId: event.pointerId,
      handle,
      startX: event.clientX,
      startY: event.clientY,
      box,
      last: box,
    };
    setResizePreview(box);
  };

  const moveResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = resizeDragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const dx = (event.clientX - drag.startX) / scale;
    const dy = (event.clientY - drag.startY) / scale;
    const next = resizeRect(drag.box, drag.handle, dx, dy);
    drag.last = next;
    setResizePreview(next);
  };

  const endResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = resizeDragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    resizeDragRef.current = null;
    setResizePreview(null);
    if (!selectedElement || !selectedLocated) return;
    const box = drag.last;
    // 选框使用页面绝对坐标，元素自身坐标相对父容器，提交前必须换算回相对坐标。
    const local = toParentBox(selectedLocated, box);
    const w = Math.max(1, round1(box.w));
    const h = Math.max(0.5, round1(box.h));
    const patch: Partial<TemplateElement> = {
      x_mm: round1(local.x),
      y_mm: round1(local.y),
      w_mm: w,
      h_mm: h,
    };
    // 矩阵高度变化后同步单元格高度，保持右侧属性面板与画布一致。
    if (selectedElement.type === 'matrix') {
      const rows = Math.max(1, Math.round(selectedElement.matrix?.rows ?? 1));
      patch.matrix = {
        ...DEFAULT_MATRIX,
        ...(selectedElement.matrix ?? {}),
        cell_h_mm: round1(h / rows),
      };
    }
    patchElement(selectedElement.id, patch);
  };

  const patchMatrix = (patch: Partial<MatrixSetup>) => {
    if (!matrixTarget) return;
    setDraft((prev) => {
      const located = findElement(prev.elements, matrixTarget.id);
      if (!located) return prev;
      const next = { ...DEFAULT_MATRIX, ...(located.element.matrix ?? {}), ...patch };
      if (next.rows !== undefined) next.rows = Math.max(1, Math.round(next.rows));
      if (next.cols !== undefined) next.cols = Math.max(1, Math.round(next.cols));
      return {
        ...prev,
        elements: updateElement(prev.elements, located.element.id, { matrix: next }),
      };
    });
  };

  /** 功能区修改矩阵行数：同步单元格高度，保持矩阵总高度不变。 */
  const setMatrixRows = (value: number) => {
    if (!matrixTarget) return;
    const nextRows = Math.max(1, Math.round(value));
    const cellH = Math.max(4, matrixTarget.h_mm / nextRows);
    patchMatrix({ rows: nextRows, cell_h_mm: cellH });
    patchElement(matrixTarget.id, { h_mm: round1(cellH * nextRows) });
  };

  const updatePage = (patch: Partial<PageSetup>) => {
    setDraft((prev) => ({ ...prev, page: { ...prev.page, ...patch } }));
  };

  const removeSelectedElement = () => {
    if (selection.type !== 'element') return;
    const id = selection.id;
    const parent = findElement(draft.elements, id)?.parent ?? null;
    setDraft((prev) => ({ ...prev, elements: removeElement(prev.elements, id) }));
    setSelection(parent ? { type: 'element', id: parent.id } : { type: 'page' });
    setEditRequestId(null);
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === 'INPUT' ||
          target.tagName === 'TEXTAREA' ||
          target.tagName === 'SELECT' ||
          target.isContentEditable)
      ) {
        return;
      }
      if (event.key === 'Backspace' || event.key === 'Delete') {
        if (selection.type === 'element') {
          event.preventDefault();
          const parent = findElement(draft.elements, selection.id)?.parent ?? null;
          setDraft((prev) => ({ ...prev, elements: removeElement(prev.elements, selection.id) }));
          setSelection(parent ? { type: 'element', id: parent.id } : { type: 'page' });
        }
      } else if (event.key === 'Escape') {
        setSelection({ type: 'page' });
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [selection, draft.elements]);

  /** 新元素尺寸与落点：容器内则相对容器，否则相对页面。 */
  const placementFor = (
    type: ElementType,
    parent: TemplateElement | null,
    siblings: number,
  ): { x: number; y: number; w: number; h: number } => {
    if (parent?.type === 'matrix') {
      const mx = { ...DEFAULT_MATRIX, ...(parent.matrix ?? {}) };
      const cols = Math.max(1, Math.round(mx.cols));
      const rows = Math.max(1, Math.round(mx.rows));
      const cellW = parent.w_mm / cols;
      const cellH = parent.h_mm / rows;
      const w = Math.max(6, cellW - 6);
      const h = type === 'image' ? Math.max(10, cellH - 8) : type === 'line' ? 0.5 : 7;
      return { x: 3, y: 3 + (siblings % 6) * 9, w, h };
    }
    if (parent?.type === 'rect') {
      const w = Math.max(6, parent.w_mm - 6);
      const h = type === 'image' ? Math.max(10, parent.h_mm - 8) : 7;
      return { x: 3, y: 3 + (siblings % 6) * 9, w, h };
    }
    return { x: 8, y: 8 + (siblings % 6) * 4, w: 60, h: type === 'line' ? 0.5 : 8 };
  };

  const addElement = (type: ElementType) => {
    if (type === 'matrix') {
      if (matrixElement) {
        show('模板中已经存在矩阵，可直接在左侧结构树中编辑', 'error');
        return;
      }
      const mx = { ...DEFAULT_MATRIX };
      const element = createElement('matrix', {
        x: 0,
        y: 0,
        w: content.w,
        h: mx.rows * mx.cell_h_mm,
      });
      element.matrix = mx;
      element.children = [];
      setDraft((prev) => ({
        ...prev,
        kind: 'matrix',
        elements: [...prev.elements, element],
      }));
      setSelection({ type: 'element', id: element.id });
      setEditRequestId(null);
      return;
    }

    let parent: TemplateElement | null = null;
    if (insertTarget === 'container') {
      parent = selectedElement && isContainer(selectedElement) ? selectedElement : null;
    }
    const siblings = parent ? parent.children.length : draft.elements.length;
    const box = placementFor(type, parent, siblings);
    const element = createElement(type, {
      field: columnNames[0] ?? (type === 'image' ? '照片' : '姓名'),
      x: box.x,
      y: box.y,
      w: box.w,
      h: box.h,
    });
    setDraft((prev) => ({
      ...prev,
      elements: appendChild(prev.elements, parent?.id ?? null, element),
    }));
    setSelection({ type: 'element', id: element.id });
    setEditRequestId(null);
  };

  const addChild = (parent: TemplateElement, type: ElementType) => {
    const siblings = parent.children.length;
    const box = placementFor(type, parent, siblings);
    const element = createElement(type, {
      field: columnNames[0] ?? (type === 'image' ? '照片' : '姓名'),
      x: box.x,
      y: box.y,
      w: box.w,
      h: box.h,
    });
    setDraft((prev) => ({ ...prev, elements: appendChild(prev.elements, parent.id, element) }));
    setSelection({ type: 'element', id: element.id });
    setEditRequestId(null);
  };

  const toggleCollapsed = (id: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const setAllCollapsed = (collapsedAll: boolean) => {
    if (!collapsedAll) {
      setCollapsed(new Set());
      return;
    }
    const ids: string[] = [];
    walkElements(draft.elements, (located) => {
      if (isContainer(located.element)) ids.push(located.element.id);
    });
    setCollapsed(new Set(ids));
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
    matrix: matrixElement?.matrix ?? DEFAULT_MATRIX,
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

  const handleDeleteTemplate = async () => {
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

  const renderTreeRow = (element: TemplateElement, depth: number) => {
    const hasChildren = element.children.length > 0;
    const isCollapsed = collapsed.has(element.id);
    const active = selection.type === 'element' && selection.id === element.id;
    const meta =
      element.type === 'matrix'
        ? `${Math.max(1, Math.round(element.matrix?.rows ?? 0))}×${Math.max(1, Math.round(element.matrix?.cols ?? 0))}`
        : element.type === 'field'
          ? element.field || '未绑定'
          : element.type === 'image'
            ? (element.photo_name || element.field || '照片').replace(/\{\{?([^}]+)\}?\}/g, '$1')
            : hasChildren
              ? `${element.children.length} 项`
              : '';
    return (
      <div key={element.id} className="tpl-tree-node">
        <div
          role="treeitem"
          aria-selected={active}
          tabIndex={0}
          className={`tpl-tree-row${active ? ' active' : ''}`}
          style={{ paddingLeft: 6 + depth * 14 }}
          onClick={() => {
            setSelection({ type: 'element', id: element.id });
            setEditRequestId(null);
          }}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              setSelection({ type: 'element', id: element.id });
            }
          }}
          title={elementLabel(element)}
        >
          <button
            type="button"
            className={`tpl-tree-caret${hasChildren ? '' : ' empty'}`}
            aria-label={isCollapsed ? '展开' : '收起'}
            onClick={(event) => {
              event.stopPropagation();
              if (hasChildren) toggleCollapsed(element.id);
            }}
          >
            {hasChildren ? (isCollapsed ? '▸' : '▾') : '·'}
          </button>
          <span className="tpl-tree-icon">{elementIcon(element.type)}</span>
          <span className="tpl-tree-label">{elementLabel(element)}</span>
          {meta ? <span className="tpl-tree-meta">{meta}</span> : null}
        </div>
        {hasChildren && !isCollapsed
          ? element.children.map((child) => renderTreeRow(child, depth + 1))
          : null}
      </div>
    );
  };

  const panelTitle =
    selection.type === 'page'
      ? '页面'
      : selectedElement
        ? ELEMENT_LABELS[selectedElement.type]
        : '未选择';
  const panelIcon = selection.type === 'page' ? '▤' : selectedElement ? elementIcon(selectedElement.type) : '·';
  const breadcrumb =
    selection.type === 'page'
      ? ['页面']
      : ['页面', ...selectedPath.map((element) => elementLabel(element))];

  /** 选框使用页面绝对坐标（mm）；拖动顶点时优先显示实时预览。 */
  const overlayBox =
    resizePreview ??
    (selectedElement && selectedLocated
      ? {
          x: selectedLocated.x,
          y: selectedLocated.y,
          w: selectedElement.w_mm,
          h: selectedElement.h_mm,
        }
      : null);
  const overlayWpx = overlayBox ? Math.max(0, overlayBox.w * scale) : 0;
  const overlayHpx = overlayBox ? Math.max(0, overlayBox.h * scale) : 0;
  // 细小元素（如线条、默认高度 8mm 的文本）缩放手柄会互相重叠，按尺寸收窄手柄并隐藏中边手柄。
  const handleSize = Math.max(
    4,
    Math.min(HANDLE_SIZE, Math.min(overlayWpx || HANDLE_SIZE, overlayHpx || HANDLE_SIZE) * 0.5),
  );
  const visibleHandles = RESIZE_HANDLES.filter((spec) => {
    if ((spec.handle === 'e' || spec.handle === 'w') && overlayHpx < 24) return false;
    if ((spec.handle === 'n' || spec.handle === 's') && overlayWpx < 24) return false;
    return true;
  });

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
    <div className="page tpl-page">
      <div className="page-header">
        <div>
          <h2 className="page-title">PDF 模板</h2>
          <p className="page-subtitle">
            左侧为页面结构树，中间为 Leafer 画布，右侧编辑选中节点的属性。
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
          <button className="btn btn-danger" onClick={handleDeleteTemplate}>
            删除模板
          </button>
        </div>
      </div>

      <div className="tpl-shell">
        {/* --------------- 文档栏 --------------- */}
        <div className="tpl-docbar">
          <span className="tpl-doc-badge">PDF</span>
          <input
            className="tpl-doc-name"
            value={draft.name}
            aria-label="模板名称"
            onChange={(event) => setDraft((prev) => ({ ...prev, name: event.target.value }))}
          />
          <span className="tpl-doc-sub">
            {isMatrix ? '矩阵分页' : '单页'} · {page.w}×{page.h} mm
          </span>
          <div className="toolbar-fill" />
          <button className="btn btn-sm" onClick={handlePreview} disabled={busy}>
            预览
          </button>
          <button className="btn btn-sm" onClick={handleRender} disabled={busy || !draft.id}>
            导出 PDF
          </button>
          <button className="btn btn-sm btn-primary" onClick={handleSave} disabled={saving}>
            {saving ? '保存中…' : '保存模板'}
          </button>
        </div>

        {/* --------------- 一级菜单：WPS 式选项卡 --------------- */}
        <div className="tpl-tabs" role="tablist" aria-label="模板工具">
          {RIBBON_TABS.map((tab) => (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={ribbonTab === tab.id}
              className={`tpl-tab${ribbonTab === tab.id ? ' active' : ''}`}
              onClick={() => setRibbonTab(tab.id)}
            >
              {tab.label}
            </button>
          ))}
        </div>

        {/* --------------- 二级菜单：当前选项卡的功能区 --------------- */}
        <div className="tpl-ribbon" role="tabpanel">
          {ribbonTab === 'home' ? (
            <>
              <div className="tpl-ribbon-group">
                <div className="tpl-ribbon-items">
                  <button
                    type="button"
                    className={`tpl-tool big${tool === 'select' ? ' active' : ''}`}
                    onClick={() => setTool('select')}
                    title="选择工具：拖动元素移动，拖动顶点缩放"
                  >
                    <span className="tpl-tool-icon">↗</span>
                    选择
                  </button>
                  <button
                    type="button"
                    className={`tpl-tool big${tool === 'hand' ? ' active' : ''}`}
                    onClick={() => setTool('hand')}
                    title="手型平移（或按住鼠标中键拖动）"
                  >
                    <span className="tpl-tool-icon">✋</span>
                    平移
                  </button>
                </div>
                <span className="tpl-ribbon-label">工具</span>
              </div>

              <span className="tpl-ribbon-sep" />

              <div className="tpl-ribbon-group">
                <div className="tpl-ribbon-items">
                  <button
                    type="button"
                    className="tpl-tool big danger"
                    onClick={removeSelectedElement}
                    disabled={selection.type !== 'element'}
                    title="删除当前选中的元素"
                  >
                    <span className="tpl-tool-icon">⌫</span>
                    删除
                  </button>
                </div>
                <span className="tpl-ribbon-label">编辑</span>
              </div>

              <span className="tpl-ribbon-sep" />

              <div className="tpl-ribbon-group">
                <div className="tpl-ribbon-items">
                  <button
                    type="button"
                    className="tpl-tool big"
                    onClick={resetView}
                    title="按窗口大小显示整页"
                  >
                    <span className="tpl-tool-icon">⤢</span>
                    适应
                  </button>
                  <button
                    type="button"
                    className="tpl-tool"
                    onClick={() => zoomAt(size.w / 2, size.h / 2, 1 / 1.2)}
                    title="缩小"
                  >
                    －
                  </button>
                  <span className="tpl-zoom-readout">{Math.round(view.zoom * 100)}%</span>
                  <button
                    type="button"
                    className="tpl-tool"
                    onClick={() => zoomAt(size.w / 2, size.h / 2, 1.2)}
                    title="放大"
                  >
                    ＋
                  </button>
                </div>
                <span className="tpl-ribbon-label">缩放</span>
              </div>
            </>
          ) : null}

          {ribbonTab === 'insert' ? (
            <>
              <div className="tpl-ribbon-group">
                <div className="tpl-ribbon-items">
                  {INSERT_TYPES.map((type) => (
                    <button
                      key={type}
                      type="button"
                      className="tpl-tool big"
                      onClick={() => addElement(type)}
                      title={`插入${ELEMENT_LABELS[type]}`}
                    >
                      <span className="tpl-tool-icon">{elementIcon(type)}</span>
                      {ELEMENT_LABELS[type]}
                    </button>
                  ))}
                  <button
                    type="button"
                    className="tpl-tool big"
                    onClick={() => addElement('matrix')}
                    disabled={Boolean(matrixElement)}
                    title={matrixElement ? '模板中已有矩阵' : '插入矩阵'}
                  >
                    <span className="tpl-tool-icon">{elementIcon('matrix')}</span>
                    矩阵
                  </button>
                </div>
                <span className="tpl-ribbon-label">插入元素</span>
              </div>

              <span className="tpl-ribbon-sep" />

              <div className="tpl-ribbon-group">
                <div className="tpl-ribbon-items">
                  <div className="segmented compact">
                    <button
                      className={insertTarget === 'container' ? 'seg active' : 'seg'}
                      onClick={() => setInsertTarget('container')}
                      title="选中矩阵或矩形后，插入的元素成为其子元素"
                    >
                      所选容器
                    </button>
                    <button
                      className={insertTarget === 'page' ? 'seg active' : 'seg'}
                      onClick={() => setInsertTarget('page')}
                    >
                      页面顶层
                    </button>
                  </div>
                </div>
                <span className="tpl-ribbon-label">插入位置</span>
              </div>

              <span className="tpl-ribbon-sep" />

              <div className="tpl-ribbon-group">
                <div className="tpl-ribbon-items">
                  <label className={`tpl-toggle${matrixTarget ? '' : ' disabled'}`}>
                    <input
                      type="checkbox"
                      checked={matrixConfig.show_grid}
                      disabled={!matrixTarget}
                      onChange={(event) => patchMatrix({ show_grid: event.target.checked })}
                    />
                    网格线
                  </label>
                  <label className={`tpl-toggle${matrixTarget ? '' : ' disabled'}`}>
                    <input
                      type="checkbox"
                      checked={matrixConfig.show_border}
                      disabled={!matrixTarget}
                      onChange={(event) => patchMatrix({ show_border: event.target.checked })}
                    />
                    外边框
                  </label>
                </div>
                <span className="tpl-ribbon-label">矩阵</span>
              </div>
            </>
          ) : null}

          {ribbonTab === 'layout' ? (
            <>
              <div className="tpl-ribbon-group">
                <div className="tpl-ribbon-items">
                  <label className="tpl-ribbon-field">
                    <span>纸张</span>
                    <select
                      value={draft.page.size}
                      onChange={(event) =>
                        updatePage({ size: event.target.value as PageSetup['size'] })
                      }
                    >
                      {(['A4', 'A3', 'A5', 'Letter'] as const).map((item) => (
                        <option key={item} value={item}>
                          {item}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="tpl-ribbon-field">
                    <span>方向</span>
                    <select
                      value={draft.page.orientation}
                      onChange={(event) =>
                        updatePage({
                          orientation: event.target.value as PageSetup['orientation'],
                        })
                      }
                    >
                      <option value="portrait">纵向</option>
                      <option value="landscape">横向</option>
                    </select>
                  </label>
                </div>
                <span className="tpl-ribbon-label">页面设置</span>
              </div>

              <span className="tpl-ribbon-sep" />

              <div className="tpl-ribbon-group">
                <div className="tpl-ribbon-items">
                  {(['top', 'right', 'bottom', 'left'] as const).map((side) => (
                    <label key={side} className="tpl-ribbon-field">
                      <span>
                        {side === 'top'
                          ? '上'
                          : side === 'right'
                            ? '右'
                            : side === 'bottom'
                              ? '下'
                              : '左'}
                      </span>
                      <input
                        type="number"
                        step={0.5}
                        value={draft.page.margin_mm[side]}
                        onChange={(event) =>
                          updatePage({
                            margin_mm: {
                              ...draft.page.margin_mm,
                              [side]: Number(event.target.value) || 0,
                            },
                          })
                        }
                      />
                    </label>
                  ))}
                </div>
                <span className="tpl-ribbon-label">页边距 (mm)</span>
              </div>

              <span className="tpl-ribbon-sep" />

              <div className="tpl-ribbon-group">
                <div className="tpl-ribbon-items">
                  <label className={`tpl-ribbon-field${matrixTarget ? '' : ' disabled'}`}>
                    <span>行数</span>
                    <input
                      type="number"
                      min={1}
                      step={1}
                      value={matrixConfig.rows}
                      disabled={!matrixTarget}
                      onChange={(event) => setMatrixRows(Number(event.target.value) || 1)}
                    />
                  </label>
                  <label className={`tpl-ribbon-field${matrixTarget ? '' : ' disabled'}`}>
                    <span>列数</span>
                    <input
                      type="number"
                      min={1}
                      step={1}
                      value={matrixConfig.cols}
                      disabled={!matrixTarget}
                      onChange={(event) =>
                        patchMatrix({ cols: Math.max(1, Math.round(Number(event.target.value) || 1)) })
                      }
                    />
                  </label>
                </div>
                <span className="tpl-ribbon-label">矩阵</span>
              </div>
            </>
          ) : null}

          {ribbonTab === 'view' ? (
            <>
              <div className="tpl-ribbon-group">
                <div className="tpl-ribbon-items">
                  <button
                    type="button"
                    className={`tpl-tool big${tool === 'select' ? ' active' : ''}`}
                    onClick={() => setTool('select')}
                  >
                    <span className="tpl-tool-icon">↗</span>
                    选择
                  </button>
                  <button
                    type="button"
                    className={`tpl-tool big${tool === 'hand' ? ' active' : ''}`}
                    onClick={() => setTool('hand')}
                  >
                    <span className="tpl-tool-icon">✋</span>
                    平移
                  </button>
                </div>
                <span className="tpl-ribbon-label">显示</span>
              </div>

              <span className="tpl-ribbon-sep" />

              <div className="tpl-ribbon-group">
                <div className="tpl-ribbon-items">
                  <button type="button" className="tpl-tool big" onClick={resetView} title="适应页面">
                    <span className="tpl-tool-icon">⤢</span>
                    适应页面
                  </button>
                  <button
                    type="button"
                    className="tpl-tool big"
                    onClick={() => setView({ zoom: 1, panX: 0, panY: 0 })}
                    title="恢复到 100%"
                  >
                    <span className="tpl-tool-icon">1:1</span>
                    100%
                  </button>
                  <button
                    type="button"
                    className="tpl-tool"
                    onClick={() => zoomAt(size.w / 2, size.h / 2, 1 / 1.2)}
                  >
                    －
                  </button>
                  <span className="tpl-zoom-readout">{Math.round(view.zoom * 100)}%</span>
                  <button
                    type="button"
                    className="tpl-tool"
                    onClick={() => zoomAt(size.w / 2, size.h / 2, 1.2)}
                  >
                    ＋
                  </button>
                </div>
                <span className="tpl-ribbon-label">缩放</span>
              </div>

              <span className="tpl-ribbon-sep" />

              <div className="tpl-ribbon-group">
                <div className="tpl-ribbon-items">
                  <button type="button" className="tpl-tool" onClick={() => setAllCollapsed(false)}>
                    展开全部
                  </button>
                  <button type="button" className="tpl-tool" onClick={() => setAllCollapsed(true)}>
                    收起全部
                  </button>
                </div>
                <span className="tpl-ribbon-label">结构树</span>
              </div>
            </>
          ) : null}
        </div>

        <div className="tpl-main">
          {/* --------------- 左：页面结构树 --------------- */}
          <aside className="tpl-tree" aria-label="页面结构">
            <div className="tpl-panel-head">
              页面结构
              <div className="toolbar-fill" />
              <button type="button" className="tpl-head-btn" onClick={() => setAllCollapsed(false)}>
                展开
              </button>
              <button type="button" className="tpl-head-btn" onClick={() => setAllCollapsed(true)}>
                收起
              </button>
            </div>
            <div className="tpl-tree-body" role="tree">
              <div
                role="treeitem"
                aria-selected={selection.type === 'page'}
                tabIndex={0}
                className={`tpl-tree-row page-row${selection.type === 'page' ? ' active' : ''}`}
                onClick={() => setSelection({ type: 'page' })}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault();
                    setSelection({ type: 'page' });
                  }
                }}
              >
                <span className="tpl-tree-caret empty">·</span>
                <span className="tpl-tree-icon">▤</span>
                <span className="tpl-tree-label">页面</span>
                <span className="tpl-tree-meta">
                  {draft.page.size} {draft.page.orientation === 'portrait' ? '纵' : '横'}
                </span>
              </div>

              {draft.elements.length === 0 ? (
                <div className="tpl-tree-empty">还没有元素，使用上方工具栏插入。</div>
              ) : (
                draft.elements.map((element) => renderTreeRow(element, 0))
              )}
            </div>
            <div className="tpl-tree-foot">
              矩阵的子元素会复制到每个单元格；选中容器后插入的元素会成为其子元素。
            </div>
          </aside>

          {/* --------------- 中：画布 --------------- */}
          <section className="tpl-canvas-area">
            <div className="tpl-canvas-bar">
              <span className="tpl-canvas-path">
                {selection.type === 'page'
                  ? '页面'
                  : selectedElement
                    ? `页面 / ${selectedPath.map((element) => elementLabel(element)).join(' / ')}`
                    : '页面'}
              </span>
              <div className="toolbar-fill" />
              <span className="detail-meta">
                {tool === 'hand' ? '手型：拖动画布' : 'Ctrl + 滚轮缩放 · 滚轮平移'}
              </span>
            </div>
            <div className="tpl-canvas-stage">
              <div
                className={`tpl-canvas${tool === 'hand' ? ' hand' : ''}`}
                ref={hostRef}
                onPointerDown={handleCanvasPointerDown}
                onPointerMove={handleCanvasPointerMove}
                onPointerUp={handleCanvasPointerUp}
                onPointerCancel={handleCanvasPointerUp}
              />
              {tool === 'select' && overlayBox && movingId === null ? (
                <div
                  className="tpl-selection"
                  style={{
                    left: translateX + overlayBox.x * scale,
                    top: translateY + overlayBox.y * scale,
                    width: Math.max(0, overlayBox.w * scale),
                    height: Math.max(0, overlayBox.h * scale),
                  }}
                >
                  <div className="tpl-selection-frame" />
                  {visibleHandles.map((spec) => (
                    <div
                      key={spec.handle}
                      className={`tpl-handle tpl-handle-${spec.handle}`}
                      style={{
                        left: spec.fx * overlayWpx - handleSize / 2,
                        top: spec.fy * overlayHpx - handleSize / 2,
                        width: handleSize,
                        height: handleSize,
                        cursor: spec.cursor,
                      }}
                      title="拖动改变选中元素的范围"
                      onPointerDown={(event) => beginResize(spec.handle, event)}
                      onPointerMove={moveResize}
                      onPointerUp={endResize}
                      onPointerCancel={endResize}
                    />
                  ))}
                </div>
              ) : null}
            </div>
            <div className="tpl-statusbar">
              <span>
                纸张 {page.w}×{page.h} mm · 页边距 {draft.page.margin_mm.left}/{draft.page.margin_mm.top}/
                {draft.page.margin_mm.right}/{draft.page.margin_mm.bottom}
              </span>
              <div className="toolbar-fill" />
              <span className="tpl-zoom-readout">{Math.round(view.zoom * 100)}%</span>
              <button type="button" className="tpl-status-btn" onClick={() => zoomAt(size.w / 2, size.h / 2, 1 / 1.2)}>
                －
              </button>
              <button type="button" className="tpl-status-btn" onClick={() => zoomAt(size.w / 2, size.h / 2, 1.2)}>
                ＋
              </button>
              <button type="button" className="tpl-status-btn" onClick={resetView}>
                适应
              </button>
            </div>
          </section>

          {/* --------------- 右：属性面板 --------------- */}
          <aside className="tpl-props" aria-label="属性">
            <div className="tpl-props-head">
              <div className="tpl-props-title">
                <span className="tpl-tree-icon">{panelIcon}</span>
                {panelTitle}
              </div>
              <div className="tpl-breadcrumb">{breadcrumb.join(' / ')}</div>
            </div>

            {selection.type === 'page' ? (
              <PagePanel
                draft={draft}
                datasets={datasets}
                datasetId={datasetId}
                busy={busy}
                hasMatrix={Boolean(matrixElement)}
                onDataset={setDatasetId}
                onDraft={setDraft}
                onPage={updatePage}
                onPreview={handlePreview}
                onRender={handleRender}
              />
            ) : selectedElement ? (
              <ElementEditor
                element={selectedElement}
                parentType={selectedLocated?.parent?.type ?? null}
                kind={draft.kind}
                columns={columnNames}
                autoEdit={editRequestId === selectedElement.id}
                onChange={(patch) => patchElement(selectedElement.id, patch)}
                onMatrixChange={patchMatrix}
                onRemove={removeSelectedElement}
                onAddChild={(type) => addChild(selectedElement, type)}
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
    elements: normalizeTemplate(template),
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
  hasMatrix: boolean;
  onDataset: (id: string) => void;
  onDraft: (updater: (prev: TemplateDraft) => TemplateDraft) => void;
  onPage: (patch: Partial<PageSetup>) => void;
  onPreview: () => void;
  onRender: () => void;
}

function PagePanel({
  draft,
  datasets,
  datasetId,
  busy,
  hasMatrix,
  onDataset,
  onDraft,
  onPage,
  onPreview,
  onRender,
}: PagePanelProps) {
  return (
    <div className="tpl-prop-body">
      <div className="tpl-section">
        <div className="tpl-section-title">文档</div>
        <label className="field">
          <span className="field-label">模板类型</span>
          <select
            value={draft.kind}
            onChange={(event) =>
              onDraft((prev) => ({ ...prev, kind: event.target.value as 'single' | 'matrix' }))
            }
          >
            <option value="single">单人单页（每行数据一页）</option>
            <option value="matrix">矩阵分页（每页铺满多行数据）</option>
          </select>
        </label>
        <label className="field">
          <span className="field-label">说明</span>
          <input
            type="text"
            value={draft.description}
            placeholder="模板用途、适用考试等"
            onChange={(event) => onDraft((prev) => ({ ...prev, description: event.target.value }))}
          />
        </label>
        {draft.kind === 'matrix' && !hasMatrix ? (
          <p className="panel-hint warn">矩阵分页需要插入一个「矩阵」元素，数据才会按单元格铺排。</p>
        ) : null}
      </div>

      <div className="tpl-section">
        <div className="tpl-section-title">纸张</div>
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
      </div>

      <div className="tpl-section">
        <div className="tpl-section-title">预览与导出</div>
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
      </div>
    </div>
  );
}

// --------------------------------------------------------------------
// 右侧：元素属性
// --------------------------------------------------------------------

interface ElementEditorProps {
  element: TemplateElement;
  parentType: ElementType | null;
  kind: 'single' | 'matrix';
  columns: string[];
  autoEdit?: boolean;
  onChange: (patch: Partial<TemplateElement>) => void;
  onMatrixChange: (patch: Partial<MatrixSetup>) => void;
  onRemove: () => void;
  onAddChild: (type: ElementType) => void;
}

function ElementEditor({
  element,
  parentType,
  kind,
  columns,
  autoEdit,
  onChange,
  onMatrixChange,
  onRemove,
  onAddChild,
}: ElementEditorProps) {
  const isMatrix = element.type === 'matrix';
  const matrix = { ...DEFAULT_MATRIX, ...(element.matrix ?? {}) };
  const rows = Math.max(1, Math.round(matrix.rows));
  const cols = Math.max(1, Math.round(matrix.cols));
  const geometryLabel = parentType === 'matrix' ? '相对单元格' : parentType === 'rect' ? '相对父元素' : '页面坐标';
  const numeric: [string, keyof TemplateElement, number][] = [
    ['X (mm)', 'x_mm', 0.5],
    ['Y (mm)', 'y_mm', 0.5],
    ['宽 (mm)', 'w_mm', 0.5],
    ['高 (mm)', 'h_mm', 0.5],
  ];

  const setRows = (value: number) => {
    const nextRows = Math.max(1, Math.round(value));
    onMatrixChange({ rows: nextRows, cell_h_mm: Math.max(4, element.h_mm / nextRows) });
    onChange({ h_mm: round1(Math.max(4, element.h_mm / nextRows) * nextRows) });
  };

  const setCellHeight = (value: number) => {
    const cellH = Math.max(4, value);
    onMatrixChange({ cell_h_mm: cellH });
    onChange({ h_mm: round1(cellH * rows) });
  };

  return (
    <div className="tpl-prop-body">
      <div className="tpl-section">
        <div className="tpl-section-title">位置与尺寸</div>
        <p className="panel-hint">{geometryLabel}</p>
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
      </div>

      {isMatrix ? (
        <div className="tpl-section">
          <div className="tpl-section-title">矩阵</div>
          <div className="grid-2">
            <NumberField label="行数" value={rows} onChange={setRows} />
            <NumberField label="列数" value={cols} onChange={(value) => onMatrixChange({ cols: Math.max(1, Math.round(value)) })} />
            <NumberField label="单元格高 (mm)" value={matrix.cell_h_mm} step={1} onChange={setCellHeight} />
            <NumberField label="顶部留白 (mm)" value={matrix.origin_y_mm ?? 0} step={1} onChange={(value) => onMatrixChange({ origin_y_mm: value })} />
          </div>
          <label className="checkbox-row">
            <input
              type="checkbox"
              checked={matrix.show_grid}
              onChange={(event) => onMatrixChange({ show_grid: event.target.checked })}
            />
            显示网格线（仅内部分隔线）
          </label>
          <label className="checkbox-row">
            <input
              type="checkbox"
              checked={matrix.show_border}
              onChange={(event) => onMatrixChange({ show_border: event.target.checked })}
            />
            显示外边框（同时输出到 PDF）
          </label>
          <p className="panel-hint">
            每个单元格 {round1(element.w_mm / cols)}×{round1(element.h_mm / rows)} mm，共 {rows * cols} 条数据/页。
            内部添加的元素会复制到所有单元格。
          </p>
          <div className="tpl-mini-actions">
            <span>在此矩阵内插入</span>
            {(['field', 'text', 'image'] as ElementType[]).map((type) => (
              <button key={type} type="button" className="btn btn-sm" onClick={() => onAddChild(type)}>
                ＋{ELEMENT_LABELS[type]}
              </button>
            ))}
          </div>
        </div>
      ) : null}

      {element.type === 'rect' ? (
        <div className="tpl-section">
          <div className="tpl-section-title">容器</div>
          <p className="panel-hint">矩形可作为标题框等容器，内部元素随矩形一起移动。</p>
          <div className="tpl-mini-actions">
            <span>在此矩形内插入</span>
            {(['text', 'field', 'image'] as ElementType[]).map((type) => (
              <button key={type} type="button" className="btn btn-sm" onClick={() => onAddChild(type)}>
                ＋{ELEMENT_LABELS[type]}
              </button>
            ))}
          </div>
        </div>
      ) : null}

      {element.type === 'text' || element.type === 'field' ? (
        <div className="tpl-section">
          <div className="tpl-section-title">内容</div>
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
          {element.type === 'field' ? (
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
        </div>
      ) : null}

      {element.type === 'image' ? (
        <div className="tpl-section">
          <div className="tpl-section-title">照片</div>
          <div className="field">
            <span className="field-label">照片目录</span>
            <div className="photo-dir-row">
              <input
                type="text"
                value={element.photo_dir ?? ''}
                placeholder="选择照片所在文件夹"
                onChange={(event) => onChange({ photo_dir: event.target.value })}
              />
              <button
                type="button"
                className="btn btn-sm"
                onClick={async () => {
                  const picked = await pickFolder(element.photo_dir || undefined);
                  if (picked) onChange({ photo_dir: picked });
                }}
              >
                选择…
              </button>
            </div>
            {!isDesktop() ? (
              <span className="field-hint">浏览器环境无法弹出系统窗口，请手动填写目录</span>
            ) : null}
          </div>
          <div className="field">
            <span className="field-label">照片名</span>
            <TokenEditor
              value={element.photo_name ?? ''}
              mode="template"
              fields={columns}
              placeholder="字段组合，如 {{身份证号}}"
              onChange={(photo_name) => onChange({ photo_name })}
            />
            <span className="field-hint">
              可用 <span className="mono">{'{{字段名}}'}</span> 取数据值，也可直接输入固定文本
            </span>
          </div>
          <div className="field">
            <span className="field-label">照片后缀</span>
            <select
              multiple
              className="photo-ext-select"
              value={element.photo_exts ?? []}
              onChange={(event) =>
                onChange({
                  photo_exts: Array.from(event.target.selectedOptions).map(
                    (option) => option.value,
                  ),
                })
              }
            >
              {PHOTO_EXT_OPTIONS.map((ext) => (
                <option key={ext} value={ext}>
                  .{ext}
                </option>
              ))}
            </select>
            <span className="field-hint">按住 Ctrl / Cmd 可多选，按选择顺序查找文件</span>
          </div>
        </div>
      ) : null}

      {element.type !== 'line' && element.type !== 'image' && !isMatrix ? (
        <div className="tpl-section">
          <div className="tpl-section-title">文字</div>
          <div className="grid-2">
            <NumberField
              label="字号 (pt)"
              step={0.5}
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
            <input
              type="checkbox"
              checked={element.bold}
              onChange={(event) => onChange({ bold: event.target.checked })}
            />
            加粗
          </label>
        </div>
      ) : null}

      {!isMatrix ? (
        <div className="tpl-section">
          <div className="tpl-section-title">外观</div>
          <div className="grid-2">
            <label className="field">
              <span className="field-label">{element.type === 'line' ? '线条颜色' : '文字/边框颜色'}</span>
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
          {element.type !== 'line' ? (
            <>
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={element.border}
                  onChange={(event) => onChange({ border: event.target.checked })}
                />
                显示边框
              </label>
              <NumberField
                label="圆角 (mm)"
                step={0.5}
                value={element.border_radius_mm}
                onChange={(value) => onChange({ border_radius_mm: value })}
              />
            </>
          ) : null}
        </div>
      ) : null}

      {kind === 'matrix' && element.type === 'matrix' ? (
        <div className="tpl-section">
          <div className="tpl-section-title">分页</div>
          <p className="panel-hint">矩阵模板按单元格铺满数据；矩阵本身作为普通元素参与页面布局。</p>
        </div>
      ) : null}

      <div className="tpl-prop-foot">
        <button className="btn btn-sm btn-danger" onClick={onRemove}>
          删除元素
        </button>
      </div>
    </div>
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
