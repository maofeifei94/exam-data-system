import { describe, expect, it } from 'vitest';
import type { PdfTemplate, TemplateElement } from '../api/types';
import {
  DEFAULT_MATRIX,
  appendChild,
  contentSizeMm,
  createElement,
  findElement,
  findPath,
  isContainer,
  normalizeTemplate,
  pageSizeMm,
  removeElement,
  resizeRect,
  toParentBox,
  updateElement,
  walkElements,
} from './templateModel';

const BASE_PAGE = {
  size: 'A4' as const,
  orientation: 'portrait' as const,
  margin_mm: { top: 15, right: 12, bottom: 12, left: 12 },
};

function element(id: string, patch: Partial<TemplateElement> = {}): TemplateElement {
  return {
    id,
    type: 'text',
    x_mm: 10,
    y_mm: 20,
    w_mm: 40,
    h_mm: 10,
    text: 'hello',
    field: '',
    font_size: 11,
    bold: false,
    align: 'left',
    color: '#000000',
    fill: '',
    border: false,
    border_radius_mm: 0,
    children: [],
    ...patch,
  };
}

describe('pageSizeMm / contentSizeMm', () => {
  it('A4 纵向尺寸与内容区计算正确', () => {
    expect(pageSizeMm(BASE_PAGE)).toEqual({ w: 210, h: 297 });
    expect(contentSizeMm(BASE_PAGE)).toEqual({ w: 186, h: 270 });
  });

  it('横向时交换宽高', () => {
    const landscape = { ...BASE_PAGE, orientation: 'landscape' as const };
    expect(pageSizeMm(landscape)).toEqual({ w: 297, h: 210 });
  });

  it('未知纸张回退到 A4', () => {
    const unknown = { ...BASE_PAGE, size: 'B5' as unknown as 'A4' };
    expect(pageSizeMm(unknown)).toEqual({ w: 210, h: 297 });
  });
});

describe('resizeRect', () => {
  const box = { x: 10, y: 20, w: 100, h: 60 };

  it('拖动右下角：左上角固定，宽高随位移变化', () => {
    expect(resizeRect(box, 'se', 15, 10)).toEqual({ x: 10, y: 20, w: 115, h: 70 });
  });

  it('拖动左上角：右下角固定', () => {
    expect(resizeRect(box, 'nw', 10, 5)).toEqual({ x: 20, y: 25, w: 90, h: 55 });
    expect(resizeRect(box, 'nw', 10, 5).x + resizeRect(box, 'nw', 10, 5).w).toBe(110);
    expect(resizeRect(box, 'nw', 10, 5).y + resizeRect(box, 'nw', 10, 5).h).toBe(80);
  });

  it('拖动右上角：左下角固定', () => {
    const next = resizeRect(box, 'ne', 10, -6);
    expect(next).toEqual({ x: 10, y: 14, w: 110, h: 66 });
    expect(next.x).toBe(box.x);
    expect(next.y + next.h).toBe(box.y + box.h);
  });

  it('拖动左下角：右上角固定', () => {
    const next = resizeRect(box, 'sw', -8, 12);
    expect(next).toEqual({ x: 2, y: 20, w: 108, h: 72 });
    expect(next.x + next.w).toBe(box.x + box.w);
    expect(next.y).toBe(box.y);
  });

  it('边中点只改变一个方向', () => {
    expect(resizeRect(box, 'n', 0, -10)).toEqual({ x: 10, y: 10, w: 100, h: 70 });
    expect(resizeRect(box, 's', 0, 10)).toEqual({ x: 10, y: 20, w: 100, h: 70 });
    expect(resizeRect(box, 'w', -10, 0)).toEqual({ x: 0, y: 20, w: 110, h: 60 });
    expect(resizeRect(box, 'e', 10, 0)).toEqual({ x: 10, y: 20, w: 110, h: 60 });
  });

  it('缩到最小尺寸时固定对边，不会翻转', () => {
    const next = resizeRect(box, 'w', 500, 0, 1, 0.5);
    expect(next.w).toBe(1);
    expect(next.x).toBe(box.x + box.w - 1);

    const vertical = resizeRect(box, 'n', 0, 500, 1, 0.5);
    expect(vertical.h).toBe(0.5);
    expect(vertical.y).toBe(box.y + box.h - 0.5);
  });

  it('向右/向下拖动左侧或顶部顶点超过对边时同样收敛', () => {
    const next = resizeRect(box, 'nw', 999, 999);
    expect(next.x + next.w).toBe(box.x + box.w);
    expect(next.y + next.h).toBe(box.y + box.h);
    expect(next.w).toBe(1);
    expect(next.h).toBe(0.5);
  });
});

describe('toParentBox', () => {
  it('页面顶层元素：绝对坐标即相对坐标，原样返回', () => {
    const located = findElement([element('root', { x_mm: 10, y_mm: 20 })], 'root');
    expect(located).not.toBeNull();
    expect(toParentBox(located!, { x: 15, y: 25, w: 40, h: 10 })).toEqual({
      x: 15,
      y: 25,
      w: 40,
      h: 10,
    });
  });

  it('矩阵子元素：减去父矩阵的页面原点，得到相对单元格坐标', () => {
    const matrix = element('matrix', {
      type: 'matrix',
      x_mm: 12,
      y_mm: 24,
      w_mm: 180,
      h_mm: 200,
      children: [element('cellchild', { x_mm: 5, y_mm: 7 })],
    });
    const located = findElement([matrix], 'cellchild');
    expect(located?.x).toBe(17);
    expect(located?.y).toBe(31);

    // 拖顶点后选框的绝对位置为 (20, 40)，换算后应为相对单元格的 (8, 16)。
    expect(toParentBox(located!, { x: 20, y: 40, w: 40, h: 10 })).toEqual({
      x: 8,
      y: 16,
      w: 40,
      h: 10,
    });
  });

  it('多级嵌套：逐级累加的父原点全部扣除', () => {
    const tree = [
      element('rect', {
        x_mm: 10,
        y_mm: 20,
        children: [
          element('inner', {
            x_mm: 5,
            y_mm: 6,
            children: [element('deep', { x_mm: 1, y_mm: 1 })],
          }),
        ],
      }),
    ];
    const located = findElement(tree, 'deep');
    expect(located?.x).toBe(16);
    expect(located?.y).toBe(27);
    expect(toParentBox(located!, { x: 20, y: 30, w: 8, h: 4 })).toEqual({
      x: 5,
      y: 4,
      w: 8,
      h: 4,
    });
  });

  it('缩放提交后重新渲染：绝对位置与拖动预览一致，不会跳位', () => {
    const matrix = element('matrix', {
      type: 'matrix',
      x_mm: 21.7,
      y_mm: 28.3,
      w_mm: 180,
      h_mm: 200,
      children: [element('cellchild', { x_mm: 5, y_mm: 7, w_mm: 30, h_mm: 10 })],
    });
    const located = findElement([matrix], 'cellchild')!;
    const preview = resizeRect(
      { x: located.x, y: located.y, w: located.element.w_mm, h: located.element.h_mm },
      'se',
      12,
      8,
    );
    const relative = toParentBox(located, preview);
    const patch = updateElement([matrix], 'cellchild', {
      x_mm: relative.x,
      y_mm: relative.y,
      w_mm: relative.w,
      h_mm: relative.h,
    });
    const after = findElement(patch, 'cellchild')!;
    expect(after.x).toBeCloseTo(preview.x, 5);
    expect(after.y).toBeCloseTo(preview.y, 5);
    expect(after.element.w_mm).toBe(preview.w);
    expect(after.element.h_mm).toBe(preview.h);
  });
});

describe('createElement', () => {
  it('矩阵元素带默认矩阵配置与空子元素', () => {
    const matrix = createElement('matrix');
    expect(matrix.type).toBe('matrix');
    expect(matrix.matrix).toEqual(DEFAULT_MATRIX);
    expect(matrix.children).toEqual([]);
    expect(matrix.border).toBe(true);
  });

  it('图片默认绑定照片字段', () => {
    const image = createElement('image');
    expect(image.field).toBe('照片');
    expect(image.w_mm).toBe(26);
    expect(image.h_mm).toBe(32);
  });

  it('允许覆盖初始位置与尺寸', () => {
    const text = createElement('text', { x: 3, y: 4, w: 50, h: 9 });
    expect(text).toMatchObject({ x_mm: 3, y_mm: 4, w_mm: 50, h_mm: 9 });
  });
});

describe('元素树操作', () => {
  const tree = [
    element('root', {
      type: 'rect',
      children: [
        element('child', { x_mm: 2, y_mm: 3 }),
        element('nested', {
          type: 'rect',
          x_mm: 5,
          y_mm: 6,
          children: [element('deep', { x_mm: 1, y_mm: 1 })],
        }),
      ],
    }),
    element('sibling'),
  ];

  it('walkElements 累加父元素偏移', () => {
    const visited: Array<[string, number, number]> = [];
    walkElements(tree, (located) => visited.push([located.element.id, located.x, located.y]));

    expect(visited).toEqual([
      ['root', 10, 20],
      ['child', 12, 23],
      ['nested', 15, 26],
      ['deep', 16, 27],
      ['sibling', 10, 20],
    ]);
  });

  it('findElement 返回父元素与绝对坐标', () => {
    const located = findElement(tree, 'deep');
    expect(located?.parent?.id).toBe('nested');
    expect(located?.x).toBe(16);
    expect(located?.y).toBe(27);
    expect(findElement(tree, 'missing')).toBeNull();
  });

  it('findPath 返回从根到目标的路径', () => {
    expect(findPath(tree, 'deep').map((item) => item.id)).toEqual(['root', 'nested', 'deep']);
    expect(findPath(tree, 'missing')).toEqual([]);
  });

  it('updateElement 深度更新任意层级', () => {
    const next = updateElement(tree, 'deep', { w_mm: 77 });
    expect(findElement(next, 'deep')?.element.w_mm).toBe(77);
    expect(findElement(tree, 'deep')?.element.w_mm).toBe(40);
  });

  it('removeElement 删除嵌套元素并保留其他节点', () => {
    const next = removeElement(tree, 'nested');
    expect(findElement(next, 'nested')).toBeNull();
    expect(findElement(next, 'deep')).toBeNull();
    expect(findElement(next, 'child')).not.toBeNull();
  });

  it('appendChild 支持页面根与嵌套容器', () => {
    const atRoot = appendChild(tree, null, element('new-root'));
    expect(atRoot.map((item) => item.id)).toContain('new-root');
    expect(findElement(atRoot, 'new-root')).not.toBeNull();

    const nested = appendChild(tree, 'nested', element('new-child'));
    expect(findElement(nested, 'new-child')?.parent?.id).toBe('nested');
  });

  it('isContainer 只把矩阵与矩形视为容器', () => {
    expect(isContainer(element('m', { type: 'matrix' }))).toBe(true);
    expect(isContainer(element('r', { type: 'rect' }))).toBe(true);
    expect(isContainer(element('t'))).toBe(false);
  });
});

describe('normalizeTemplate', () => {
  const template = (patch: Partial<PdfTemplate>): PdfTemplate => ({
    id: 'tpl',
    name: 'demo',
    kind: 'single',
    description: '',
    page: BASE_PAGE,
    matrix: DEFAULT_MATRIX,
    elements: [],
    scope: 'global',
    project_id: '',
    source_id: '',
    created_at: '',
    updated_at: '',
    ...patch,
  });

  it('补全缺失的 children 与矩阵默认值', () => {
    const raw = [{ ...element('m', { type: 'matrix' }), children: undefined }] as unknown as TemplateElement[];
    const [matrix] = normalizeTemplate(template({ elements: raw }));
    expect(matrix.children).toEqual([]);
    expect(matrix.matrix).toEqual(DEFAULT_MATRIX);
  });

  it('旧版矩阵模板把扁平元素迁移到矩阵子元素', () => {
    const raw = [
      element('title', { page_level: true, y_mm: 5 }),
      element('cell', { y_mm: 2 }),
    ];
    const migrated = normalizeTemplate(
      template({
        kind: 'matrix',
        matrix: { ...DEFAULT_MATRIX, rows: 2, cols: 3, cell_h_mm: 40 },
        elements: raw,
      }),
    );

    expect(migrated.map((item) => item.id)).toEqual(['title', expect.stringContaining('el_')]);
    const matrix = migrated[1];
    expect(matrix.type).toBe('matrix');
    expect(matrix.children.map((item) => item.id)).toEqual(['cell']);
    expect(matrix.h_mm).toBe(80);
  });
});
