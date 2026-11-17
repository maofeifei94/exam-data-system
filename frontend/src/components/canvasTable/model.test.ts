import { describe, expect, it } from 'vitest';
import {
  MAX_COLUMN_WIDTH,
  MIN_COLUMN_WIDTH,
  cellText,
  columnAt,
  columnLabel,
  columnOffsets,
  computeColumnWidths,
  estimateTextWidth,
  findColumnBoundary,
  hitTest,
  moveFocus,
  normalizeSelection,
  revealCell,
  rowHeaderWidth,
  sampleRowIndexes,
  selectAll,
  selectionToTsv,
  virtualRows,
} from './model';

describe('virtualRows', () => {
  it('10 万行时只渲染视口内的行', () => {
    const window = virtualRows(0, 700, 24, 100_000);
    expect(window.start).toBe(0);
    expect(window.end).toBeLessThanOrEqual(40);

    const scrolled = virtualRows(1_000_000, 700, 24, 100_000);
    expect(scrolled.end - scrolled.start).toBeLessThanOrEqual(40);
    expect(scrolled.start).toBeGreaterThan(41_000);
  });

  it('滚动到底部时不越过总行数', () => {
    const window = virtualRows(100_000 * 24 - 700, 700, 24, 100_000);
    expect(window.end).toBe(100_000);
  });

  it('没有数据时返回空窗口', () => {
    expect(virtualRows(0, 700, 24, 0)).toEqual({ start: 0, end: 0 });
    expect(virtualRows(0, 0, 24, 100)).toEqual({ start: 0, end: 0 });
  });
});

describe('computeColumnWidths', () => {
  it('10 万行只抽样少量行，不会全表扫描', () => {
    const indexes = sampleRowIndexes(100_000);
    expect(indexes.length).toBeLessThanOrEqual(240);
    expect(Math.max(...indexes)).toBe(99_999);
  });

  it('列宽被限制在最小 / 最大范围内', () => {
    const rows = Array.from({ length: 10 }, () => ['短', 'x'.repeat(200)]);
    const widths = computeColumnWidths(['姓名', '备注'], rows);
    expect(widths).toHaveLength(2);
    widths.forEach((width) => {
      expect(width).toBeGreaterThanOrEqual(MIN_COLUMN_WIDTH);
      expect(width).toBeLessThanOrEqual(MAX_COLUMN_WIDTH);
    });
    expect(widths[1]).toBe(MAX_COLUMN_WIDTH);
  });

  it('中文列名比等长英文更宽', () => {
    const chinese = computeColumnWidths(['身份证号码'], [])[0];
    const english = computeColumnWidths(['idcardno'], [])[0];
    expect(chinese).toBeGreaterThan(english);
  });
});

describe('坐标换算', () => {
  it('Excel 风格列名', () => {
    expect(columnLabel(0)).toBe('A');
    expect(columnLabel(25)).toBe('Z');
    expect(columnLabel(26)).toBe('AA');
    expect(columnLabel(701)).toBe('ZZ');
    expect(columnLabel(702)).toBe('AAA');
  });

  it('行号列宽度随行数位数增长', () => {
    expect(rowHeaderWidth(9)).toBe(46);
    expect(rowHeaderWidth(100_000)).toBe(76);
  });

  it('列累计坐标与命中列一致', () => {
    const offsets = columnOffsets([100, 80, 120]);
    expect(offsets).toEqual([0, 100, 180, 300]);
    expect(columnAt(offsets, 0)).toBe(0);
    expect(columnAt(offsets, 99)).toBe(0);
    expect(columnAt(offsets, 100)).toBe(1);
    expect(columnAt(offsets, 299)).toBe(2);
    expect(columnAt(offsets, 300)).toBeNull();
    expect(columnAt(offsets, -1)).toBeNull();
  });

  it('文本宽度估算区分中英文', () => {
    expect(estimateTextWidth('身份证号')).toBeGreaterThan(estimateTextWidth('abcde'));
  });
});

describe('hitTest', () => {
  const base = {
    scrollLeft: 0,
    scrollTop: 0,
    rowHeaderWidth: 50,
    headerHeight: 44,
    rowHeight: 24,
    offsets: [0, 100, 200],
    rowCount: 100,
    columnCount: 2,
  };

  it('区分角部 / 列头 / 行号 / 单元格', () => {
    expect(hitTest({ ...base, x: 10, y: 10 }).kind).toBe('corner');
    expect(hitTest({ ...base, x: 160, y: 10 })).toEqual({ kind: 'column-header', col: 1 });
    expect(hitTest({ ...base, x: 10, y: 100 })).toEqual({ kind: 'row-header', row: 2 });
    expect(hitTest({ ...base, x: 160, y: 100 })).toEqual({ kind: 'cell', row: 2, col: 1 });
  });

  it('命中计算考虑滚动偏移', () => {
    const hit = hitTest({ ...base, scrollLeft: 100, scrollTop: 240, x: 60, y: 50 });
    expect(hit).toEqual({ kind: 'cell', row: 10, col: 1 });
  });

  it('表格右侧空白区域不算命中', () => {
    expect(hitTest({ ...base, x: 400, y: 100 }).kind).toBe('outside');
  });

  it('落在列边框上时返回可拖拽的列', () => {
    expect(
      findColumnBoundary({
        x: 150,
        y: 20,
        scrollLeft: 0,
        rowHeaderWidth: 50,
        headerHeight: 44,
        offsets: [0, 100, 200],
      }),
    ).toBe(0);
    expect(
      findColumnBoundary({
        x: 150,
        y: 60,
        scrollLeft: 0,
        rowHeaderWidth: 50,
        headerHeight: 44,
        offsets: [0, 100, 200],
      }),
    ).toBeNull();
  });
});

describe('选区', () => {
  it('锚点与焦点顺序任意，都能归一化', () => {
    expect(
      normalizeSelection({ anchor: { row: 5, col: 3 }, focus: { row: 2, col: 1 } }),
    ).toEqual({ r1: 2, r2: 5, c1: 1, c2: 3 });
  });

  it('移动焦点时夹在数据范围内', () => {
    const selection = { anchor: { row: 0, col: 0 }, focus: { row: 0, col: 0 } };
    expect(moveFocus(selection, -1, -1, false, 10, 5)).toEqual({
      anchor: { row: 0, col: 0 },
      focus: { row: 0, col: 0 },
    });
    const extended = moveFocus(selection, 3, 2, true, 10, 5);
    expect(extended.anchor).toEqual({ row: 0, col: 0 });
    expect(extended.focus).toEqual({ row: 3, col: 2 });
    const moved = moveFocus(selection, 3, 2, false, 10, 5);
    expect(moved.anchor).toEqual({ row: 3, col: 2 });
  });

  it('全选覆盖所有行列', () => {
    expect(selectAll(100_000, 12)).toEqual({
      anchor: { row: 0, col: 0 },
      focus: { row: 99_999, col: 11 },
    });
    expect(selectAll(0, 12)).toBeNull();
  });

  it('复制选区导出 TSV，空值留空、布尔转大写', () => {
    const rows = [
      ['姓名', '成绩', '通过'],
      ['张三', 98.5, true],
      [null, null, false],
    ];
    const tsv = selectionToTsv(rows, {
      anchor: { row: 0, col: 0 },
      focus: { row: 2, col: 2 },
    });
    expect(tsv).toBe('姓名\t成绩\t通过\n张三\t98.5\tTRUE\n\t\tFALSE');
  });

  it('复制时越界单元格按空字符串处理', () => {
    const tsv = selectionToTsv([['a']], {
      anchor: { row: 0, col: 0 },
      focus: { row: 1, col: 1 },
    });
    expect(tsv).toBe('a\t\n\t');
  });
});

describe('revealCell', () => {
  const options = {
    offsets: [0, 100, 200],
    widths: [100, 100, 100],
    rowHeight: 24,
    scrollLeft: 0,
    scrollTop: 0,
    bodyWidth: 300,
    bodyHeight: 240,
  };

  it('目标在视口内时不改变滚动位置', () => {
    expect(revealCell({ ...options, row: 5, col: 1 })).toEqual({ left: 0, top: 0 });
  });

  it('目标在右侧 / 下方时滚动到可见位置', () => {
    expect(revealCell({ ...options, row: 20, col: 2 })).toEqual({ left: 0, top: 264 });
    // 目标在视口上方时回到顶部，列已在视口内则横向不动
    expect(
      revealCell({ ...options, row: 0, col: 2, scrollLeft: 150, scrollTop: 100 }),
    ).toEqual({ left: 150, top: 0 });
  });
});

describe('cellText', () => {
  it('空值显示为空，布尔显示 TRUE / FALSE', () => {
    expect(cellText(null)).toBe('');
    expect(cellText(undefined)).toBe('');
    expect(cellText(0)).toBe('0');
    expect(cellText(true)).toBe('TRUE');
    expect(cellText(false)).toBe('FALSE');
  });
});
