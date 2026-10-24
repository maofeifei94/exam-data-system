import type { MatrixSetup, PageSetup, PdfTemplate, TemplateElement } from '../api/types';

export type ElementType = TemplateElement['type'];

export const DEFAULT_MATRIX: MatrixSetup = {
  rows: 5,
  cols: 6,
  gap_x_mm: 0,
  gap_y_mm: 0,
  cell_h_mm: 42,
  origin_y_mm: 0,
  show_grid: true,
  show_border: true,
};

export function newElementId(): string {
  return `el_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 7)}`;
}

export function pageSizeMm(page: PageSetup): { w: number; h: number } {
  const sizes: Record<string, [number, number]> = {
    A3: [297, 420],
    A4: [210, 297],
    A5: [148, 210],
    Letter: [215.9, 279.4],
  };
  const [w, h] = sizes[page.size] ?? sizes.A4;
  return page.orientation === 'landscape' ? { w: h, h: w } : { w, h };
}

export function round1(value: number): number {
  return Math.round(value * 10) / 10;
}

/** 矩形框的 8 个缩放顶点：四角 + 四边中点。 */
export type ResizeHandle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w';

export interface RectBox {
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * 拖动矩形框顶点后的新矩形：
 * - 对角点保持不动；
 * - 相邻两条边随顶点一起移动，矩形始终保持轴对齐；
 * - 宽高不小于 minW / minH。
 */
export function resizeRect(
  box: RectBox,
  handle: ResizeHandle,
  dx: number,
  dy: number,
  minW = 1,
  minH = 0.5,
): RectBox {
  let { x, y, w, h } = box;

  if (handle.includes('w')) {
    x += dx;
    w -= dx;
  }
  if (handle.includes('e')) w += dx;
  if (handle.includes('n')) {
    y += dy;
    h -= dy;
  }
  if (handle.includes('s')) h += dy;

  // 触到最小尺寸时固定对边，避免继续拖动导致翻转
  if (w < minW) {
    if (handle.includes('w')) x = box.x + box.w - minW;
    w = minW;
  }
  if (h < minH) {
    if (handle.includes('n')) y = box.y + box.h - minH;
    h = minH;
  }

  return { x, y, w, h };
}

const BASE_ELEMENT: Omit<TemplateElement, 'id' | 'type' | 'x_mm' | 'y_mm' | 'w_mm' | 'h_mm'> = {
  text: '',
  field: '',
  font_size: 11,
  bold: false,
  align: 'left',
  color: '#000000',
  fill: '',
  border: false,
  border_radius_mm: 0,
  children: [],
  photo_dir: '',
  photo_name: '',
  photo_exts: [],
};

/** 照片元素的默认候选后缀，按顺序查找。 */
export const DEFAULT_PHOTO_EXTS = ['jpg', 'jpeg', 'png'];

/** 照片元素默认按「照片」字段取文件名。 */
export const DEFAULT_PHOTO_NAME = '{{照片}}';

export function createElement(
  type: ElementType,
  options: { field?: string; x?: number; y?: number; w?: number; h?: number } = {},
): TemplateElement {
  const base: TemplateElement = {
    ...BASE_ELEMENT,
    id: newElementId(),
    type,
    x_mm: options.x ?? 6,
    y_mm: options.y ?? 6,
    w_mm: options.w ?? 60,
    h_mm: options.h ?? 8,
    children: [],
  };
  switch (type) {
    case 'rect':
      return { ...base, w_mm: options.w ?? 60, h_mm: options.h ?? 30, fill: '#ffffff', border: true };
    case 'line':
      return { ...base, w_mm: options.w ?? 120, h_mm: options.h ?? 1 };
    case 'image':
      return {
        ...base,
        w_mm: options.w ?? 26,
        h_mm: options.h ?? 32,
        field: '',
        photo_dir: '',
        photo_name: DEFAULT_PHOTO_NAME,
        photo_exts: [...DEFAULT_PHOTO_EXTS],
        border: true,
      };
    case 'field':
      return { ...base, w_mm: options.w ?? 40, h_mm: options.h ?? 7, field: options.field ?? '姓名' };
    case 'matrix':
      return {
        ...base,
        w_mm: options.w ?? 120,
        h_mm: options.h ?? 90,
        matrix: { ...DEFAULT_MATRIX },
        fill: '',
        border: true,
      };
    default:
      return { ...base, w_mm: options.w ?? 60, h_mm: options.h ?? 8, text: '文本内容' };
  }
}

/**
 * 让元素树补全 children / matrix 默认值，兼容旧数据。
 * 旧版矩阵把单元格间距算进矩阵宽高；新版网格是连续栅格、无间距，
 * 因此这里收紧矩阵宽高，保证单元格尺寸与旧模板一致。
 */
export function normalizeTree(elements: TemplateElement[] | undefined): TemplateElement[] {
  return (elements ?? []).map((element) => {
    if (element.type === 'matrix') {
      const mx = { ...DEFAULT_MATRIX, ...(element.matrix ?? {}) };
      const cols = Math.max(1, Math.round(mx.cols));
      const rows = Math.max(1, Math.round(mx.rows));
      const gapX = Math.max(0, mx.gap_x_mm ?? 0);
      const gapY = Math.max(0, mx.gap_y_mm ?? 0);
      return {
        ...element,
        w_mm: Math.max(1, round1(element.w_mm - gapX * (cols - 1))),
        h_mm: Math.max(1, round1(element.h_mm - gapY * (rows - 1))),
        matrix: { ...mx, rows, cols, gap_x_mm: 0, gap_y_mm: 0 },
        children: normalizeTree(element.children),
        page_level: false,
      };
    }
    if (element.type === 'image') {
      // 旧模板用 field 绑定单个字段取照片文件名；新结构改为名称模板 + 候选后缀。
      const legacyField = (element.field ?? '').trim();
      const photoName = (element.photo_name ?? '').trim();
      return {
        ...element,
        children: normalizeTree(element.children),
        page_level: false,
        photo_dir: element.photo_dir ?? '',
        photo_name: photoName || (legacyField ? `{{${legacyField}}}` : DEFAULT_PHOTO_NAME),
        photo_exts: element.photo_exts?.length ? element.photo_exts : [...DEFAULT_PHOTO_EXTS],
      };
    }
    return {
      ...element,
      children: normalizeTree(element.children),
      page_level: false,
    };
  });
}

/**
 * 旧版模板是「扁平元素 + 页面级 matrix 配置」，这里迁移成嵌套元素：
 * 页面标题等留在根，单元格内容挂到 matrix 元素下。
 */
export function normalizeTemplate(template: PdfTemplate): TemplateElement[] {
  const raw = template.elements ?? [];
  if (raw.some((element) => element.type === 'matrix')) return normalizeTree(raw);
  if (template.kind !== 'matrix') return normalizeTree(raw);

  const page = pageSizeMm(template.page);
  const contentW = Math.max(10, page.w - template.page.margin_mm.left - template.page.margin_mm.right);
  const mx = { ...DEFAULT_MATRIX, ...(template.matrix ?? {}) };
  const cols = Math.max(1, mx.cols);
  const rows = Math.max(1, mx.rows);
  // 新版矩阵是连续栅格，单元格之间没有间隔
  const cellW = Math.max(1, contentW / cols);
  const cellH = mx.cell_h_mm > 0 ? mx.cell_h_mm : 30;
  const matrixW = cellW * cols;
  const matrixH = cellH * rows;

  const pageElements: TemplateElement[] = [];
  const cellElements: TemplateElement[] = [];
  raw.forEach((element) => {
    if (element.page_level) pageElements.push(element);
    else cellElements.push(element);
  });

  const matrix = createElement('matrix', {
    x: 0,
    y: mx.origin_y_mm ?? 0,
    w: matrixW,
    h: matrixH,
  });
  matrix.matrix = { ...mx, gap_x_mm: 0, gap_y_mm: 0 };
  matrix.children = normalizeTree(cellElements);
  return normalizeTree([...pageElements, matrix]);
}

export interface LocatedElement {
  element: TemplateElement;
  parent: TemplateElement | null;
  /** 元素左上角在页面中的绝对位置（mm），矩阵子元素按第一个单元格计算 */
  x: number;
  y: number;
  depth: number;
}

/**
 * 把页面绝对坐标的矩形换算回元素自身的相对坐标。
 * 元素 x_mm / y_mm 始终相对父容器（矩阵单元格、矩形框），
 * 因此缩放提交时必须先减去父元素的绝对原点，否则嵌套元素会在松手瞬间跳位。
 */
export function toParentBox(located: LocatedElement, box: RectBox): RectBox {
  const originX = located.x - located.element.x_mm;
  const originY = located.y - located.element.y_mm;
  return { ...box, x: box.x - originX, y: box.y - originY };
}

export function walkElements(
  elements: TemplateElement[],
  visit: (located: LocatedElement) => void,
  parent: TemplateElement | null = null,
  offsetX = 0,
  offsetY = 0,
  depth = 0,
): void {
  elements.forEach((element) => {
    const x = offsetX + element.x_mm;
    const y = offsetY + element.y_mm;
    visit({ element, parent, x, y, depth });
    if (element.children.length > 0) {
      walkElements(element.children, visit, element, x, y, depth + 1);
    }
  });
}

/** 从页面根到目标元素的路径（含目标本身）。 */
export function findPath(elements: TemplateElement[], id: string): TemplateElement[] {
  for (const element of elements) {
    if (element.id === id) return [element];
    const childPath = findPath(element.children, id);
    if (childPath.length > 0) return [element, ...childPath];
  }
  return [];
}

export function isContainer(element: TemplateElement): boolean {
  return element.type === 'matrix' || element.type === 'rect';
}

export function contentSizeMm(page: PageSetup): { w: number; h: number } {
  const size = pageSizeMm(page);
  return {
    w: Math.max(10, size.w - page.margin_mm.left - page.margin_mm.right),
    h: Math.max(10, size.h - page.margin_mm.top - page.margin_mm.bottom),
  };
}

export function findElement(elements: TemplateElement[], id: string): LocatedElement | null {
  let found: LocatedElement | null = null;
  walkElements(elements, (located) => {
    if (located.element.id === id) found = located;
  });
  return found;
}

export function updateElement(
  elements: TemplateElement[],
  id: string,
  patch: Partial<TemplateElement>,
): TemplateElement[] {
  return elements.map((element) => {
    if (element.id === id) return { ...element, ...patch };
    if (element.children.length === 0) return element;
    return { ...element, children: updateElement(element.children, id, patch) };
  });
}

export function removeElement(elements: TemplateElement[], id: string): TemplateElement[] {
  return elements
    .filter((element) => element.id !== id)
    .map((element) =>
      element.children.length === 0
        ? element
        : { ...element, children: removeElement(element.children, id) },
    );
}

/** 在指定容器（null=页面根）中追加子元素。 */
export function appendChild(
  elements: TemplateElement[],
  parentId: string | null,
  child: TemplateElement,
): TemplateElement[] {
  if (!parentId) return [...elements, child];
  return elements.map((element) => {
    if (element.id === parentId) return { ...element, children: [...element.children, child] };
    if (element.children.length === 0) return element;
    return { ...element, children: appendChild(element.children, parentId, child) };
  });
}

export function elementLabel(element: TemplateElement): string {
  if (element.type === 'matrix') return `矩阵 ${element.matrix?.rows ?? 0}×${element.matrix?.cols ?? 0}`;
  if (element.type === 'image') {
    const name = (element.photo_name ?? element.field ?? '').replace(/\{\{?([^}]+)\}?\}/g, '$1');
    return `图片 · ${name || '照片'}`;
  }
  if (element.type === 'rect') return '矩形框';
  if (element.type === 'line') return '线条';
  if (element.type === 'field') return `字段 ${element.field || ''}`;
  const text = (element.text || element.field || '文本').replace(/\{\{?([^}]+)\}?\}/g, '$1');
  return text.length > 12 ? `${text.slice(0, 12)}…` : text;
}

export function elementIcon(type: ElementType): string {
  switch (type) {
    case 'matrix':
      return '▦';
    case 'rect':
      return '▭';
    case 'image':
      return '▣';
    case 'line':
      return '╱';
    case 'field':
      return '⌗';
    default:
      return 'T';
  }
}

export function cloneElementWithNewIds(element: TemplateElement): TemplateElement {
  return {
    ...element,
    id: newElementId(),
    children: element.children.map(cloneElementWithNewIds),
  };
}
