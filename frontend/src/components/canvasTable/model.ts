/**
 * 画布表格的纯计算层：虚拟滚动窗口、列宽估算、命中测试、选区与复制。
 *
 * 这里刻意不依赖 DOM，方便在 node 环境下单测；
 * 真正把像素画到 canvas 上的逻辑在 paint.ts。
 */

export type CellValue = string | number | boolean | null | undefined;
export type TableRows = CellValue[][];

export const DEFAULT_ROW_HEIGHT = 24;
export const DEFAULT_HEADER_HEIGHT = 44;
export const MIN_COLUMN_WIDTH = 76;
export const MAX_COLUMN_WIDTH = 320;
export const CELL_PADDING_X = 9;
export const MIN_ROW_HEADER_WIDTH = 46;
export const MAX_ROW_HEADER_WIDTH = 88;
export const OVERSCAN_ROWS = 4;

const HEADER_FONT_SIZE = 13;
const BODY_FONT_SIZE = 12.5;

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/** 粗略估算文本宽度：CJK 按 1em，窄字符 0.32em，其余约 0.55em。 */
export function estimateTextWidth(text: string, fontSize = BODY_FONT_SIZE): number {
  let width = 0;
  for (const ch of text) {
    const code = ch.codePointAt(0) ?? 0;
    if (code >= 0x2e80) {
      width += fontSize;
    } else if (ch === ' ') {
      width += fontSize * 0.32;
    } else if (/[ilI.,:;'|!]/.test(ch)) {
      width += fontSize * 0.32;
    } else if (/[A-Z0-9mwMW@#%&]/.test(ch)) {
      width += fontSize * 0.66;
    } else {
      width += fontSize * 0.55;
    }
  }
  return width;
}

/** 单元格的显示文本：空值显示为空（与 Excel 一致），布尔显示 TRUE / FALSE。 */
export function cellText(value: CellValue): string {
  if (value === null || value === undefined) return '';
  if (typeof value === 'boolean') return value ? 'TRUE' : 'FALSE';
  return String(value);
}

/**
 * 计算列宽时抽取的行下标：数据量小时全量，数据量大时取开头 160 行 + 均匀分布 80 行。
 * 保证 10 万行也只测量约 240 行，避免初始化卡顿。
 */
export function sampleRowIndexes(rowCount: number): number[] {
  if (rowCount <= 240) {
    return Array.from({ length: Math.max(0, rowCount) }, (_, index) => index);
  }
  const seen = new Set<number>();
  for (let i = 0; i < 160; i += 1) seen.add(i);
  for (let i = 0; i < 80; i += 1) seen.add(Math.round((i * (rowCount - 1)) / 79));
  return [...seen];
}

/** 依据表头 + 抽样单元格估算每列宽度（像素）。 */
export function computeColumnWidths(
  columns: string[],
  rows: TableRows,
  fontSize = BODY_FONT_SIZE,
): number[] {
  const indexes = sampleRowIndexes(rows.length);
  return columns.map((name, col) => {
    // 表头是加粗两行（字母 + 字段名），留一点余量
    let width = estimateTextWidth(name, HEADER_FONT_SIZE) * 1.08;
    for (const rowIndex of indexes) {
      const text = cellText(rows[rowIndex]?.[col]);
      if (!text) continue;
      const measured = estimateTextWidth(text, fontSize);
      if (measured > width) width = measured;
    }
    const padded = Math.ceil(width + CELL_PADDING_X * 2 + 16);
    return clamp(padded, MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
  });
}

/** Excel 风格的列名：A、B … Z、AA、AB … */
export function columnLabel(index: number): string {
  let label = '';
  let n = index + 1;
  while (n > 0) {
    const rest = (n - 1) % 26;
    label = String.fromCharCode(65 + rest) + label;
    n = Math.floor((n - 1) / 26);
  }
  return label;
}

/** 行号列宽度随行数位数增长。 */
export function rowHeaderWidth(rowCount: number): number {
  const digits = String(Math.max(1, rowCount)).length;
  return clamp(digits * 8 + 28, MIN_ROW_HEADER_WIDTH, MAX_ROW_HEADER_WIDTH);
}

/** 列左边界的累计坐标，长度 = 列数 + 1。 */
export function columnOffsets(widths: number[]): number[] {
  const offsets = new Array<number>(widths.length + 1);
  offsets[0] = 0;
  for (let i = 0; i < widths.length; i += 1) {
    offsets[i + 1] = offsets[i] + widths[i];
  }
  return offsets;
}

export interface VirtualRows {
  /** 渲染起始行（含） */
  start: number;
  /** 渲染结束行（不含） */
  end: number;
}

/** 只渲染视口内的行（含少量 overscan），这是 10 万行不卡顿的关键。 */
export function virtualRows(
  scrollTop: number,
  bodyHeight: number,
  rowHeight: number,
  rowCount: number,
  overscan = OVERSCAN_ROWS,
): VirtualRows {
  if (rowCount <= 0 || bodyHeight <= 0 || rowHeight <= 0) {
    return { start: 0, end: 0 };
  }
  const firstVisible = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const visibleCount = Math.ceil(bodyHeight / rowHeight) + 1;
  return {
    start: clamp(firstVisible - overscan, 0, rowCount),
    end: clamp(firstVisible + visibleCount + overscan, 0, rowCount),
  };
}

/** 当前横向视口内可见的列下标。 */
export function visibleColumns(
  offsets: number[],
  scrollLeft: number,
  bodyWidth: number,
): number[] {
  const count = Math.max(0, offsets.length - 1);
  if (count === 0 || bodyWidth <= 0) return [];
  const result: number[] = [];
  const left = Math.max(0, scrollLeft);
  const right = scrollLeft + bodyWidth;
  for (let col = 0; col < count; col += 1) {
    if (offsets[col + 1] <= left) continue;
    if (offsets[col] >= right) break;
    result.push(col);
  }
  return result;
}

/** 命中哪一列；落在表格空白处返回 null。 */
export function columnAt(offsets: number[], x: number): number | null {
  const count = Math.max(0, offsets.length - 1);
  if (count === 0 || x < 0 || x >= offsets[count]) return null;
  let low = 0;
  let high = count - 1;
  while (low < high) {
    const mid = (low + high + 1) >> 1;
    if (offsets[mid] <= x) low = mid;
    else high = mid - 1;
  }
  return low;
}

export interface CellPos {
  row: number;
  col: number;
}

/** 与 Excel 相同的选区模型：锚点 + 焦点，拖动时焦点移动。 */
export interface Selection {
  anchor: CellPos;
  focus: CellPos;
}

export interface CellRange {
  r1: number;
  r2: number;
  c1: number;
  c2: number;
}

export function normalizeSelection(selection: Selection): CellRange {
  return {
    r1: Math.min(selection.anchor.row, selection.focus.row),
    r2: Math.max(selection.anchor.row, selection.focus.row),
    c1: Math.min(selection.anchor.col, selection.focus.col),
    c2: Math.max(selection.anchor.col, selection.focus.col),
  };
}

export function selectedCellCount(range: CellRange | null): number {
  if (!range) return 0;
  return (range.r2 - range.r1 + 1) * (range.c2 - range.c1 + 1);
}

export function clampSelection(
  selection: Selection,
  rowCount: number,
  colCount: number,
): Selection {
  const maxRow = Math.max(0, rowCount - 1);
  const maxCol = Math.max(0, colCount - 1);
  return {
    anchor: {
      row: clamp(selection.anchor.row, 0, maxRow),
      col: clamp(selection.anchor.col, 0, maxCol),
    },
    focus: {
      row: clamp(selection.focus.row, 0, maxRow),
      col: clamp(selection.focus.col, 0, maxCol),
    },
  };
}

export interface HitTestOptions {
  /** 相对表格可视区的坐标 */
  x: number;
  y: number;
  scrollLeft: number;
  scrollTop: number;
  rowHeaderWidth: number;
  headerHeight: number;
  rowHeight: number;
  offsets: number[];
  rowCount: number;
  columnCount: number;
}

export type TableHit =
  | { kind: 'outside' }
  | { kind: 'corner' }
  | { kind: 'column-header'; col: number }
  | { kind: 'row-header'; row: number }
  | { kind: 'cell'; row: number; col: number };

export function hitTest(options: HitTestOptions): TableHit {
  const {
    x,
    y,
    scrollLeft,
    scrollTop,
    rowHeaderWidth,
    headerHeight,
    rowHeight,
    offsets,
    rowCount,
    columnCount,
  } = options;
  if (x < 0 || y < 0) return { kind: 'outside' };

  const inHeader = y < headerHeight;
  const inGutter = x < rowHeaderWidth;

  if (inHeader && inGutter) return { kind: 'corner' };

  if (inHeader) {
    const col = columnAt(offsets, x - rowHeaderWidth + scrollLeft);
    if (col === null || col >= columnCount) return { kind: 'outside' };
    return { kind: 'column-header', col };
  }

  if (inGutter) {
    const row = Math.floor((y - headerHeight + scrollTop) / rowHeight);
    if (row < 0 || row >= rowCount) return { kind: 'outside' };
    return { kind: 'row-header', row };
  }

  const col = columnAt(offsets, x - rowHeaderWidth + scrollLeft);
  const row = Math.floor((y - headerHeight + scrollTop) / rowHeight);
  if (col === null || col >= columnCount || row < 0 || row >= rowCount) {
    return { kind: 'outside' };
  }
  return { kind: 'cell', row, col };
}

/** 鼠标是否落在列边框上（用于拖拽调整列宽）。 */
export function findColumnBoundary(options: {
  x: number;
  y: number;
  scrollLeft: number;
  rowHeaderWidth: number;
  headerHeight: number;
  offsets: number[];
  tolerance?: number;
}): number | null {
  const { x, y, scrollLeft, rowHeaderWidth, headerHeight, offsets } = options;
  const tolerance = options.tolerance ?? 3;
  if (y < 0 || y >= headerHeight || x < rowHeaderWidth) return null;
  const contentX = x - rowHeaderWidth + scrollLeft;
  const count = Math.max(0, offsets.length - 1);
  for (let col = 0; col < count; col += 1) {
    if (Math.abs(contentX - offsets[col + 1]) <= tolerance) return col;
  }
  return null;
}

/** 键盘 / 点击移动选区；extend 为 true 时保持锚点（Shift+点击 / Shift+方向键）。 */
export function moveFocus(
  selection: Selection,
  deltaRow: number,
  deltaCol: number,
  extend: boolean,
  rowCount: number,
  columnCount: number,
): Selection {
  if (rowCount <= 0 || columnCount <= 0) return selection;
  const focus: CellPos = {
    row: clamp(selection.focus.row + deltaRow, 0, rowCount - 1),
    col: clamp(selection.focus.col + deltaCol, 0, columnCount - 1),
  };
  return extend ? { anchor: selection.anchor, focus } : { anchor: focus, focus };
}

export function selectAll(rowCount: number, columnCount: number): Selection | null {
  if (rowCount <= 0 || columnCount <= 0) return null;
  return {
    anchor: { row: 0, col: 0 },
    focus: { row: rowCount - 1, col: columnCount - 1 },
  };
}

/** 选区导出为 TSV，可直接粘到 Excel / WPS。 */
export function selectionToTsv(rows: TableRows, selection: Selection): string {
  const range = normalizeSelection(selection);
  const lines: string[] = [];
  for (let row = range.r1; row <= range.r2; row += 1) {
    const cells: string[] = [];
    for (let col = range.c1; col <= range.c2; col += 1) {
      cells.push(cellText(rows[row]?.[col]));
    }
    lines.push(cells.join('\t'));
  }
  return lines.join('\n');
}

/** 键盘移动后，把目标单元格滚进视口所需的新滚动位置。 */
export function revealCell(options: {
  row: number;
  col: number;
  offsets: number[];
  widths: number[];
  rowHeight: number;
  scrollLeft: number;
  scrollTop: number;
  bodyWidth: number;
  bodyHeight: number;
}): { left: number; top: number } {
  const {
    row,
    col,
    offsets,
    widths,
    rowHeight,
    scrollLeft,
    scrollTop,
    bodyWidth,
    bodyHeight,
  } = options;
  let left = scrollLeft;
  let top = scrollTop;
  const cellX = offsets[col] ?? 0;
  const cellW = widths[col] ?? 0;
  if (cellX < left) left = cellX;
  else if (cellX + cellW > left + bodyWidth) left = cellX + cellW - bodyWidth;
  const cellY = row * rowHeight;
  if (cellY < top) top = cellY;
  else if (cellY + rowHeight > top + bodyHeight) top = cellY + rowHeight - bodyHeight;
  return { left: Math.max(0, left), top: Math.max(0, top) };
}
