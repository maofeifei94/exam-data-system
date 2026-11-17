/**
 * 画布表格的绘制层：只画视口内的行，每帧的绘制量与总行数无关。
 *
 * 所有内容都画在同一张 canvas 上（表头、行号列、网格、文字、选区），
 * 不创建任何 DOM 节点，因此 10 万行与 100 行没有性能差异。
 */
import {
  CELL_PADDING_X,
  cellText,
  columnLabel,
  columnOffsets,
  normalizeSelection,
  virtualRows,
  visibleColumns,
  type CellValue,
  type Selection,
} from './model';

export interface TablePalette {
  background: string;
  grid: string;
  headerBackground: string;
  headerSelected: string;
  headerText: string;
  headerTextSelected: string;
  headerBorder: string;
  headerBorderStrong: string;
  bodyText: string;
  mutedText: string;
  selectionFill: string;
  selectionBorder: string;
  handleFill: string;
  handleBorder: string;
  resizeGuide: string;
  cornerMark: string;
}

/** Excel 浅色主题。 */
export const LIGHT_PALETTE: TablePalette = {
  background: '#ffffff',
  grid: '#d9dee3',
  headerBackground: '#f5f6f7',
  headerSelected: '#217346',
  headerText: '#5c636a',
  headerTextSelected: '#ffffff',
  headerBorder: '#d2d7dc',
  headerBorderStrong: '#b9c0c7',
  bodyText: '#1f2328',
  mutedText: '#8b929a',
  selectionFill: 'rgba(33, 115, 70, 0.12)',
  selectionBorder: '#217346',
  handleFill: '#217346',
  handleBorder: '#ffffff',
  resizeGuide: '#217346',
  cornerMark: '#c3c9cf',
};

/** 深色主题：保留 Excel 绿色强调，背景改为深灰。 */
export const DARK_PALETTE: TablePalette = {
  background: '#1b1f1d',
  grid: '#333b37',
  headerBackground: '#242a27',
  headerSelected: '#2f7a54',
  headerText: '#b9c2bc',
  headerTextSelected: '#ffffff',
  headerBorder: '#39423e',
  headerBorderStrong: '#4a544f',
  bodyText: '#e6e9e7',
  mutedText: '#8f9a94',
  selectionFill: 'rgba(64, 170, 110, 0.16)',
  selectionBorder: '#3fae74',
  handleFill: '#3fae74',
  handleBorder: '#1b1f1d',
  resizeGuide: '#3fae74',
  cornerMark: '#4a544f',
};

export interface DrawTableOptions {
  ctx: CanvasRenderingContext2D;
  /** CSS 像素尺寸（已按 dpr 缩放） */
  width: number;
  height: number;
  dpr: number;
  columns: string[];
  rows: CellValue[][];
  columnWidths: number[];
  rowHeaderWidth: number;
  headerHeight: number;
  rowHeight: number;
  scrollLeft: number;
  scrollTop: number;
  selection: Selection | null;
  resizingColumn: number | null;
  palette: TablePalette;
  fontFamily: string;
}

const BODY_FONT_SIZE = 12.5;
const measureCache = new Map<string, number>();
const fitCache = new Map<string, string>();
const CACHE_LIMIT = 20000;

function measure(ctx: CanvasRenderingContext2D, text: string): number {
  const key = `${ctx.font}\u0000${text}`;
  const cached = measureCache.get(key);
  if (cached !== undefined) return cached;
  const width = ctx.measureText(text).width;
  if (measureCache.size > CACHE_LIMIT) measureCache.clear();
  measureCache.set(key, width);
  return width;
}

/** 超出列宽时截断并加省略号；结果缓存，滚动时不再重复测量。 */
function fitText(ctx: CanvasRenderingContext2D, text: string, maxWidth: number): string {
  if (maxWidth <= 0) return '';
  if (measure(ctx, text) <= maxWidth) return text;
  const key = `${ctx.font}\u0000${Math.round(maxWidth)}\u0000${text}`;
  const cached = fitCache.get(key);
  if (cached !== undefined) return cached;

  let low = 0;
  let high = text.length;
  while (low < high) {
    const mid = (low + high + 1) >> 1;
    if (measure(ctx, `${text.slice(0, mid)}…`) <= maxWidth) low = mid;
    else high = mid - 1;
  }
  const result = low > 0 ? `${text.slice(0, low)}…` : '';
  if (fitCache.size > CACHE_LIMIT) fitCache.clear();
  fitCache.set(key, result);
  return result;
}

function crisp(value: number): number {
  return Math.round(value) + 0.5;
}

export function drawTable(options: DrawTableOptions): void {
  const { ctx, width, height, dpr, palette, rows, columns, columnWidths } = options;
  if (width <= 0 || height <= 0) return;

  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = palette.background;
  ctx.fillRect(0, 0, width, height);

  const gutterWidth = Math.min(options.rowHeaderWidth, width);
  const headerHeight = Math.min(options.headerHeight, height);
  const bodyWidth = width - gutterWidth;
  const bodyHeight = height - headerHeight;
  if (bodyWidth <= 0 || bodyHeight <= 0) return;

  const offsets = columnOffsets(columnWidths);
  const visibleCols = visibleColumns(offsets, options.scrollLeft, bodyWidth);
  const { start, end } = virtualRows(
    options.scrollTop,
    bodyHeight,
    options.rowHeight,
    rows.length,
  );
  const range = options.selection ? normalizeSelection(options.selection) : null;

  const bodyFont = `${BODY_FONT_SIZE}px ${options.fontFamily}`;

  ctx.save();
  ctx.beginPath();
  ctx.rect(gutterWidth, headerHeight, bodyWidth, bodyHeight);
  ctx.clip();

  // ---- 选区底色（画在文字下方） ----
  if (range) {
    ctx.fillStyle = palette.selectionFill;
    for (let row = start; row < end; row += 1) {
      if (row < range.r1 || row > range.r2) continue;
      const y = headerHeight + row * options.rowHeight - options.scrollTop;
      for (const col of visibleCols) {
        if (col < range.c1 || col > range.c2) continue;
        const x = gutterWidth + offsets[col] - options.scrollLeft;
        ctx.fillRect(x, y, columnWidths[col], options.rowHeight);
      }
    }
  }

  // ---- 网格线（一次 stroke，减少绘制调用） ----
  ctx.strokeStyle = palette.grid;
  ctx.lineWidth = 1;
  ctx.beginPath();
  let lastX = -1;
  for (const col of visibleCols) {
    const candidates = [offsets[col], offsets[col + 1]];
    for (const boundary of candidates) {
      const x = gutterWidth + boundary - options.scrollLeft;
      const rounded = Math.round(x);
      if (rounded === lastX || rounded <= gutterWidth || rounded >= width) continue;
      lastX = rounded;
      const lineX = crisp(x);
      ctx.moveTo(lineX, headerHeight);
      ctx.lineTo(lineX, height);
    }
  }
  for (let row = start; row < end; row += 1) {
    const y = headerHeight + row * options.rowHeight - options.scrollTop;
    const lineY = crisp(y);
    ctx.moveTo(gutterWidth, lineY);
    ctx.lineTo(width, lineY);
    if (row === end - 1) {
      const bottom = crisp(y + options.rowHeight);
      ctx.moveTo(gutterWidth, bottom);
      ctx.lineTo(width, bottom);
    }
  }
  ctx.stroke();

  // ---- 单元格文字 ----
  ctx.font = bodyFont;
  ctx.textBaseline = 'middle';
  const textY = options.rowHeight / 2 + 0.5;
  for (let row = start; row < end; row += 1) {
    const y = headerHeight + row * options.rowHeight - options.scrollTop;
    for (const col of visibleCols) {
      const value = rows[row]?.[col];
      const text = cellText(value);
      if (!text) continue;
      const x = gutterWidth + offsets[col] - options.scrollLeft;
      const maxWidth = columnWidths[col] - CELL_PADDING_X * 2;
      const numeric = typeof value === 'number';
      ctx.fillStyle = palette.bodyText;
      ctx.textAlign = numeric ? 'right' : 'left';
      const textX = numeric
        ? x + columnWidths[col] - CELL_PADDING_X
        : x + CELL_PADDING_X;
      ctx.fillText(fitText(ctx, text, maxWidth), textX, y + textY);
    }
  }

  // ---- 选区边框 + 填充柄 ----
  if (range) {
    const x1 = gutterWidth + offsets[range.c1] - options.scrollLeft;
    const x2 = gutterWidth + offsets[range.c2 + 1] - options.scrollLeft;
    const y1 = headerHeight + range.r1 * options.rowHeight - options.scrollTop;
    const y2 = headerHeight + (range.r2 + 1) * options.rowHeight - options.scrollTop;
    ctx.strokeStyle = palette.selectionBorder;
    ctx.lineWidth = 2;
    ctx.strokeRect(
      Math.round(x1) + 1,
      Math.round(y1) + 1,
      Math.max(0, Math.round(x2 - x1) - 2),
      Math.max(0, Math.round(y2 - y1) - 2),
    );
    // Excel 右下角的填充柄
    if (x2 > gutterWidth - 4 && x2 < width + 6 && y2 > headerHeight - 4 && y2 < height + 6) {
      const handleSize = 7;
      const handleX = Math.round(x2) - handleSize / 2;
      const handleY = Math.round(y2) - handleSize / 2;
      ctx.fillStyle = palette.handleFill;
      ctx.fillRect(handleX, handleY, handleSize, handleSize);
      ctx.strokeStyle = palette.handleBorder;
      ctx.lineWidth = 1;
      ctx.strokeRect(handleX + 0.5, handleY + 0.5, handleSize - 1, handleSize - 1);
    }
  }
  ctx.restore();

  // ---- 列标题（冻结） ----
  ctx.save();
  ctx.beginPath();
  ctx.rect(gutterWidth, 0, bodyWidth, headerHeight);
  ctx.clip();
  ctx.fillStyle = palette.headerBackground;
  ctx.fillRect(gutterWidth, 0, bodyWidth, headerHeight);

  if (range) {
    for (const col of visibleCols) {
      if (col < range.c1 || col > range.c2) continue;
      const x = gutterWidth + offsets[col] - options.scrollLeft;
      ctx.fillStyle = palette.headerSelected;
      ctx.fillRect(x, 0, columnWidths[col], headerHeight);
    }
  }

  const labelFont = `10.5px ${options.fontFamily}`;
  const headerFont = `600 13px ${options.fontFamily}`;
  for (const col of visibleCols) {
    const x = gutterWidth + offsets[col] - options.scrollLeft;
    const active = range !== null && col >= range.c1 && col <= range.c2;
    const centerX = x + columnWidths[col] / 2;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.font = labelFont;
    ctx.fillStyle = active
      ? palette.headerTextSelected
      : palette.mutedText;
    ctx.globalAlpha = active ? 0.78 : 1;
    ctx.fillText(columnLabel(col), centerX, headerHeight * 0.3);
    ctx.globalAlpha = 1;
    ctx.font = headerFont;
    ctx.fillStyle = active ? palette.headerTextSelected : palette.headerText;
    const name = columns[col] ?? '';
    ctx.fillText(fitText(ctx, name, columnWidths[col] - 12), centerX, headerHeight * 0.68);
  }

  ctx.strokeStyle = palette.headerBorder;
  ctx.lineWidth = 1;
  ctx.beginPath();
  let lastHeaderX = -1;
  for (const col of visibleCols) {
    const x = gutterWidth + offsets[col + 1] - options.scrollLeft;
    const rounded = Math.round(x);
    if (rounded === lastHeaderX) continue;
    lastHeaderX = rounded;
    const lineX = crisp(x);
    ctx.moveTo(lineX, 0);
    ctx.lineTo(lineX, headerHeight);
  }
  ctx.stroke();
  ctx.restore();

  // ---- 行号列（冻结） ----
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, headerHeight, gutterWidth, bodyHeight);
  ctx.clip();
  ctx.fillStyle = palette.headerBackground;
  ctx.fillRect(0, headerHeight, gutterWidth, bodyHeight);

  if (range) {
    for (let row = start; row < end; row += 1) {
      if (row < range.r1 || row > range.r2) continue;
      const y = headerHeight + row * options.rowHeight - options.scrollTop;
      ctx.fillStyle = palette.headerSelected;
      ctx.fillRect(0, y, gutterWidth, options.rowHeight);
    }
  }

  ctx.font = `11.5px ${options.fontFamily}`;
  ctx.textAlign = 'right';
  ctx.textBaseline = 'middle';
  for (let row = start; row < end; row += 1) {
    const y = headerHeight + row * options.rowHeight - options.scrollTop;
    const active = range !== null && row >= range.r1 && row <= range.r2;
    ctx.fillStyle = active
      ? palette.headerTextSelected
      : palette.mutedText;
    ctx.fillText(String(row + 1), gutterWidth - 8, y + options.rowHeight / 2 + 0.5);
  }

  ctx.strokeStyle = palette.headerBorder;
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let row = start; row < end; row += 1) {
    const y = headerHeight + row * options.rowHeight - options.scrollTop;
    const lineY = crisp(y);
    ctx.moveTo(0, lineY);
    ctx.lineTo(gutterWidth, lineY);
    if (row === end - 1) {
      const bottom = crisp(y + options.rowHeight);
      ctx.moveTo(0, bottom);
      ctx.lineTo(gutterWidth, bottom);
    }
  }
  ctx.stroke();
  ctx.restore();

  // ---- 左上角 ----
  ctx.fillStyle = palette.headerBackground;
  ctx.fillRect(0, 0, gutterWidth, headerHeight);
  // 点这里全选（与 Excel 一致），画一个小三角提示
  ctx.fillStyle = palette.cornerMark;
  ctx.beginPath();
  ctx.moveTo(gutterWidth - 9, headerHeight - 3);
  ctx.lineTo(gutterWidth - 3, headerHeight - 3);
  ctx.lineTo(gutterWidth - 3, headerHeight - 9);
  ctx.closePath();
  ctx.fill();

  // ---- 冻结分隔线 ----
  ctx.strokeStyle = palette.headerBorderStrong;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(gutterWidth - 0.5, 0);
  ctx.lineTo(gutterWidth - 0.5, height);
  ctx.moveTo(0, headerHeight - 0.5);
  ctx.lineTo(width, headerHeight - 0.5);
  ctx.stroke();

  // ---- 拖拽调整列宽时的指示线 ----
  if (options.resizingColumn !== null) {
    const x = gutterWidth + offsets[options.resizingColumn + 1] - options.scrollLeft;
    if (x >= gutterWidth && x <= width) {
      ctx.strokeStyle = palette.resizeGuide;
      ctx.lineWidth = 2;
      ctx.setLineDash([5, 3]);
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, height);
      ctx.stroke();
      ctx.setLineDash([]);
    }
  }
}
