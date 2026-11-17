import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type UIEvent as ReactUIEvent,
} from 'react';
import { FONT_SANS, useThemeMode } from '../theme';
import { EmptyState } from './EmptyState';
import { Spinner } from './Spinner';
import {
  DEFAULT_HEADER_HEIGHT,
  DEFAULT_ROW_HEIGHT,
  MAX_COLUMN_WIDTH,
  MIN_COLUMN_WIDTH,
  clamp,
  columnAt,
  columnOffsets,
  computeColumnWidths,
  findColumnBoundary,
  hitTest,
  moveFocus,
  revealCell,
  rowHeaderWidth,
  selectAll,
  selectionToTsv,
  type Selection,
  type TableRows,
} from './canvasTable/model';
import { DARK_PALETTE, LIGHT_PALETTE, drawTable } from './canvasTable/paint';

export interface CanvasTableProps {
  columns: string[];
  rows: TableRows;
  loading?: boolean;
  emptyText?: string;
  /** 表格高度：数字按像素，字符串直接作为 CSS 高度（如 '100%'） */
  maxHeight?: number | string;
  rowHeight?: number;
  className?: string;
}

type DragState =
  | { mode: 'selection'; axis: 'cell' | 'column' | 'row' }
  | { mode: 'resize'; col: number; startX: number; startWidth: number }
  | null;

/**
 * Excel 风格的画布表格：冻结表头 / 行号列、鼠标滚轮滚动、单元格选区、拖拽列宽。
 *
 * 10 万行也不卡顿的原因：canvas 上只绘制视口内的几十行，
 * 数据本身不做任何 DOM 映射，滚动时只重绘、不重建节点。
 */
export function CanvasTable({
  columns,
  rows,
  loading = false,
  emptyText = '暂无数据',
  maxHeight = 420,
  rowHeight = DEFAULT_ROW_HEIGHT,
  className,
}: CanvasTableProps) {
  const { mode } = useThemeMode();
  const palette = mode === 'dark' ? DARK_PALETTE : LIGHT_PALETTE;

  const rootRef = useRef<HTMLDivElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  /** 视口尺寸（不含滚动条） */
  const viewportRef = useRef({ width: 0, height: 0 });
  const scrollStateRef = useRef({ left: 0, top: 0 });
  const selectionRef = useRef<Selection | null>(null);
  const dragRef = useRef<DragState>(null);
  const resizingColRef = useRef<number | null>(null);
  const frameRef = useRef(0);

  const [resizedWidths, setResizedWidths] = useState<Record<number, number>>({});

  const autoWidths = useMemo(() => computeColumnWidths(columns, rows), [columns, rows]);
  const widths = useMemo(
    () => autoWidths.map((width, index) => resizedWidths[index] ?? width),
    [autoWidths, resizedWidths],
  );

  const gutterWidth = useMemo(() => rowHeaderWidth(rows.length), [rows.length]);
  const offsets = useMemo(() => columnOffsets(widths), [widths]);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext('2d');
    if (!canvas || !ctx) return;
    let { width, height } = viewportRef.current;
    // 兜底：首次绘制时 ResizeObserver 可能还没回调
    if (width <= 0 || height <= 0) {
      const el = scrollRef.current;
      if (!el) return;
      width = el.clientWidth;
      height = el.clientHeight;
      viewportRef.current = { width, height };
    }
    if (width <= 0 || height <= 0) return;

    const dpr = Math.min(window.devicePixelRatio || 1, 2.5);
    const pixelWidth = Math.max(1, Math.round(width * dpr));
    const pixelHeight = Math.max(1, Math.round(height * dpr));
    if (canvas.width !== pixelWidth) canvas.width = pixelWidth;
    if (canvas.height !== pixelHeight) canvas.height = pixelHeight;
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;

    drawTable({
      ctx,
      width,
      height,
      dpr,
      columns,
      rows,
      columnWidths: widths,
      rowHeaderWidth: gutterWidth,
      headerHeight: DEFAULT_HEADER_HEIGHT,
      rowHeight,
      scrollLeft: scrollStateRef.current.left,
      scrollTop: scrollStateRef.current.top,
      selection: selectionRef.current,
      resizingColumn: resizingColRef.current,
      palette,
      fontFamily: FONT_SANS,
    });
  }, [columns, rows, widths, gutterWidth, rowHeight, palette]);

  const drawRef = useRef(draw);
  const scheduleDraw = useCallback(() => {
    if (frameRef.current) return;
    frameRef.current = window.requestAnimationFrame(() => {
      frameRef.current = 0;
      drawRef.current();
    });
  }, []);

  useEffect(() => {
    drawRef.current = draw;
    scheduleDraw();
  }, [draw, scheduleDraw]);

  // 数据变化：重置列宽调整、选区与滚动位置。
  useEffect(() => {
    setResizedWidths({});
    selectionRef.current =
      rows.length > 0 && columns.length > 0
        ? { anchor: { row: 0, col: 0 }, focus: { row: 0, col: 0 } }
        : null;
    scrollStateRef.current = { left: 0, top: 0 };
    const el = scrollRef.current;
    if (el) {
      el.scrollTop = 0;
      el.scrollLeft = 0;
    }
    scheduleDraw();
  }, [autoWidths, rows, columns, scheduleDraw]);

  // 尺寸变化（含滚动条出现 / 消失）时同步画布像素尺寸。
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const sync = () => {
      viewportRef.current = { width: el.clientWidth, height: el.clientHeight };
      scheduleDraw();
    };
    sync();
    const observer = new ResizeObserver(sync);
    observer.observe(el);
    return () => observer.disconnect();
  }, [scheduleDraw]);

  useEffect(() => {
    return () => {
      if (frameRef.current) {
        window.cancelAnimationFrame(frameRef.current);
        // 必须清零：StrictMode 会挂载两次，残留的 id 会永久阻止重新调度
        frameRef.current = 0;
      }
    };
  }, []);

  // 拖拽可能在表格外松开，统一在 window 上结束。
  useEffect(() => {
    const finish = () => {
      if (dragRef.current || resizingColRef.current !== null) {
        dragRef.current = null;
        resizingColRef.current = null;
        scheduleDraw();
      }
    };
    window.addEventListener('mouseup', finish);
    return () => window.removeEventListener('mouseup', finish);
  }, [scheduleDraw]);

  const localPoint = (event: ReactMouseEvent): { x: number; y: number } => {
    const el = scrollRef.current;
    if (!el) return { x: 0, y: 0 };
    const rect = el.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };

  const hitOptions = (x: number, y: number) => ({
    x,
    y,
    scrollLeft: scrollStateRef.current.left,
    scrollTop: scrollStateRef.current.top,
    rowHeaderWidth: gutterWidth,
    headerHeight: DEFAULT_HEADER_HEIGHT,
    rowHeight,
    offsets,
    rowCount: rows.length,
    columnCount: columns.length,
  });

  /** 拖拽选区时把坐标换算成行列（越界自动夹到最近单元格）。 */
  const cellPosAt = (x: number, y: number) => {
    const { left, top } = scrollStateRef.current;
    const contentX = x - gutterWidth + left;
    const rawCol = columnAt(offsets, contentX);
    const col =
      rawCol ?? clamp(contentX < 0 ? 0 : columns.length - 1, 0, Math.max(0, columns.length - 1));
    const row = clamp(
      Math.floor((y - DEFAULT_HEADER_HEIGHT + top) / rowHeight),
      0,
      Math.max(0, rows.length - 1),
    );
    return { row, col };
  };

  const copySelection = useCallback(() => {
    const selection = selectionRef.current;
    if (!selection) return;
    const text = selectionToTsv(rows, selection);
    void navigator.clipboard?.writeText(text).catch(() => undefined);
  }, [rows]);

  const onMouseDown = (event: ReactMouseEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    scrollRef.current?.focus({ preventScroll: true });
    const { x, y } = localPoint(event);
    const boundary = findColumnBoundary({
      x,
      y,
      scrollLeft: scrollStateRef.current.left,
      rowHeaderWidth: gutterWidth,
      headerHeight: DEFAULT_HEADER_HEIGHT,
      offsets,
    });

    if (boundary !== null) {
      dragRef.current = {
        mode: 'resize',
        col: boundary,
        startX: event.clientX,
        startWidth: widths[boundary],
      };
      resizingColRef.current = boundary;
      scheduleDraw();
      event.preventDefault();
      return;
    }

    if (rows.length === 0 || columns.length === 0) return;
    const hit = hitTest(hitOptions(x, y));

    if (hit.kind === 'corner') {
      selectionRef.current = selectAll(rows.length, columns.length);
      dragRef.current = { mode: 'selection', axis: 'cell' };
      scheduleDraw();
      event.preventDefault();
      return;
    }

    if (hit.kind === 'column-header') {
      selectionRef.current = {
        anchor: { row: 0, col: hit.col },
        focus: { row: rows.length - 1, col: hit.col },
      };
      dragRef.current = { mode: 'selection', axis: 'column' };
      scheduleDraw();
      event.preventDefault();
      return;
    }

    if (hit.kind === 'row-header') {
      selectionRef.current = {
        anchor: { row: hit.row, col: 0 },
        focus: { row: hit.row, col: columns.length - 1 },
      };
      dragRef.current = { mode: 'selection', axis: 'row' };
      scheduleDraw();
      event.preventDefault();
      return;
    }

    if (hit.kind === 'cell') {
      const current = selectionRef.current;
      const pos = { row: hit.row, col: hit.col };
      selectionRef.current =
        event.shiftKey && current
          ? { anchor: current.anchor, focus: pos }
          : { anchor: pos, focus: pos };
      dragRef.current = { mode: 'selection', axis: 'cell' };
      scheduleDraw();
      event.preventDefault();
    }
  };

  const onMouseMove = (event: ReactMouseEvent<HTMLDivElement>) => {
    const el = scrollRef.current;
    if (!el) return;
    const drag = dragRef.current;

    if (drag?.mode === 'resize') {
      const nextWidth = clamp(
        drag.startWidth + (event.clientX - drag.startX),
        MIN_COLUMN_WIDTH,
        MAX_COLUMN_WIDTH,
      );
      setResizedWidths((prev) => ({ ...prev, [drag.col]: nextWidth }));
      return;
    }

    if (drag?.mode === 'selection' && selectionRef.current) {
      const point = localPoint(event);
      const pos = cellPosAt(point.x, point.y);
      const selection = selectionRef.current;
      if (drag.axis === 'cell') {
        selectionRef.current = { anchor: selection.anchor, focus: pos };
      } else if (drag.axis === 'column') {
        selectionRef.current = {
          anchor: { row: 0, col: selection.anchor.col },
          focus: { row: rows.length - 1, col: pos.col },
        };
      } else {
        selectionRef.current = {
          anchor: { row: selection.anchor.row, col: 0 },
          focus: { row: pos.row, col: columns.length - 1 },
        };
      }
      scheduleDraw();
      return;
    }

    const { x, y } = localPoint(event);
    const boundary = findColumnBoundary({
      x,
      y,
      scrollLeft: scrollStateRef.current.left,
      rowHeaderWidth: gutterWidth,
      headerHeight: DEFAULT_HEADER_HEIGHT,
      offsets,
    });
    el.style.cursor =
      boundary !== null ? 'col-resize' : hitTest(hitOptions(x, y)).kind === 'cell' ? 'cell' : 'default';
  };

  const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const el = scrollRef.current;
    const selection = selectionRef.current;
    if (!el || !selection || rows.length === 0 || columns.length === 0) return;

    const meta = event.ctrlKey || event.metaKey;
    const key = event.key;
    let next: Selection | null = null;
    const pageRows = Math.max(1, Math.floor((el.clientHeight - DEFAULT_HEADER_HEIGHT) / rowHeight) - 1);

    switch (key) {
      case 'ArrowUp':
        next = moveFocus(selection, -1, 0, event.shiftKey, rows.length, columns.length);
        break;
      case 'ArrowDown':
        next = moveFocus(selection, 1, 0, event.shiftKey, rows.length, columns.length);
        break;
      case 'ArrowLeft':
        next = moveFocus(selection, 0, -1, event.shiftKey, rows.length, columns.length);
        break;
      case 'ArrowRight':
        next = moveFocus(selection, 0, 1, event.shiftKey, rows.length, columns.length);
        break;
      case 'Home':
        next = meta
          ? { anchor: { row: 0, col: 0 }, focus: { row: 0, col: 0 } }
          : moveFocus(
              selection,
              0,
              -selection.focus.col,
              event.shiftKey,
              rows.length,
              columns.length,
            );
        break;
      case 'End':
        next = moveFocus(
          selection,
          0,
          columns.length - 1 - selection.focus.col,
          event.shiftKey,
          rows.length,
          columns.length,
        );
        break;
      case 'PageUp':
        next = moveFocus(selection, -pageRows, 0, event.shiftKey, rows.length, columns.length);
        break;
      case 'PageDown':
        next = moveFocus(selection, pageRows, 0, event.shiftKey, rows.length, columns.length);
        break;
      default:
        if (meta && (key === 'c' || key === 'C')) {
          copySelection();
          event.preventDefault();
          return;
        }
        if (meta && (key === 'a' || key === 'A')) {
          selectionRef.current = selectAll(rows.length, columns.length);
          scheduleDraw();
          event.preventDefault();
          return;
        }
        return;
    }

    if (!next) return;
    selectionRef.current = next;
    event.preventDefault();

    const target = revealCell({
      row: next.focus.row,
      col: next.focus.col,
      offsets,
      widths,
      rowHeight,
      scrollLeft: el.scrollLeft,
      scrollTop: el.scrollTop,
      bodyWidth: el.clientWidth - gutterWidth,
      bodyHeight: el.clientHeight - DEFAULT_HEADER_HEIGHT,
    });
    if (target.left !== el.scrollLeft) el.scrollLeft = target.left;
    if (target.top !== el.scrollTop) el.scrollTop = target.top;
    scrollStateRef.current = { left: el.scrollLeft, top: el.scrollTop };
    scheduleDraw();
  };

  const onScroll = (event: ReactUIEvent<HTMLDivElement>) => {
    const el = event.currentTarget;
    scrollStateRef.current = { left: el.scrollLeft, top: el.scrollTop };
    scheduleDraw();
  };

  const height = typeof maxHeight === 'number' ? `${maxHeight}px` : maxHeight;
  const showEmpty = !loading && rows.length === 0;

  return (
    <div
      ref={rootRef}
      className={`canvas-table${className ? ` ${className}` : ''}`}
      style={{ height }}
    >
      <canvas ref={canvasRef} className="canvas-table__canvas" aria-hidden="true" />
      <div
        ref={scrollRef}
        className="canvas-table__scroll"
        role="grid"
        aria-label="数据表格"
        aria-rowcount={rows.length}
        aria-colcount={columns.length}
        tabIndex={0}
        onScroll={onScroll}
        onMouseDown={onMouseDown}
        onMouseMove={onMouseMove}
        onKeyDown={onKeyDown}
      >
        <div
          className="canvas-table__spacer"
          style={{
            width: offsets[offsets.length - 1] + gutterWidth,
            height: rows.length * rowHeight + DEFAULT_HEADER_HEIGHT,
          }}
        />
      </div>
      {loading ? (
        <div className="canvas-table__overlay">
          <Spinner />
          <span className="table-state-text">加载中…</span>
        </div>
      ) : null}
      {showEmpty ? (
        <div className="canvas-table__overlay">
          <EmptyState text={emptyText} />
        </div>
      ) : null}
    </div>
  );
}

export default CanvasTable;
