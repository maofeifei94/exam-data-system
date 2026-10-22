import { describe, expect, it } from 'vitest';
import type { FlowStep } from '../api/types';
import {
  BRANCH_GAP_X,
  GAP_Y,
  NODE_H,
  NODE_W,
  PAD,
  branchSiblings,
  computeLayout,
  insertRelativeStep,
  insertStepAt,
  isMainStep,
} from './flowModel';

function step(id: string, extra: Partial<FlowStep> = {}): FlowStep {
  return { id, operator: 'filter_rows', label: id, enabled: true, params: {}, ...extra };
}

describe('isMainStep', () => {
  it('没有挂载父节点的步骤属于主链', () => {
    const steps = [step('a')];
    expect(isMainStep(steps[0], steps)).toBe(true);
  });

  it('父节点存在时属于分支', () => {
    const steps = [step('a'), step('b', { branch_of: 'a' })];
    expect(isMainStep(steps[1], steps)).toBe(false);
  });

  it('父节点已被删除时重新回到主链', () => {
    const steps = [step('b', { branch_of: 'missing' })];
    expect(isMainStep(steps[0], steps)).toBe(true);
  });
});

describe('computeLayout', () => {
  it('主链按顺序纵向排列，PDF 锚点位于最后一个主链节点下方', () => {
    const steps = [step('a'), step('b'), step('c')];
    const { positions, pdfPos } = computeLayout(steps);

    expect(positions.get('a')).toEqual({ x: 110, y: PAD + NODE_H + GAP_Y });
    expect(positions.get('b')).toEqual({ x: 110, y: PAD + NODE_H + GAP_Y + NODE_H + GAP_Y });
    expect(positions.get('c')!.x).toBe(110);
    expect(positions.get('c')!.y).toBe(
      PAD + NODE_H + GAP_Y + 2 * (NODE_H + GAP_Y),
    );
    expect(pdfPos).toEqual({ x: 110, y: positions.get('c')!.y + NODE_H + GAP_Y });
  });

  it('保留用户拖拽后的自定义坐标', () => {
    const steps = [step('a', { pos: { x: 40, y: 300 } })];
    const { positions, pdfPos } = computeLayout(steps);
    expect(positions.get('a')).toEqual({ x: 40, y: 300 });
    expect(pdfPos.y).toBe(300 + NODE_H + GAP_Y);
  });

  it('空流程的 PDF 锚点仍位于首个节点位置下方', () => {
    const { pdfPos } = computeLayout([]);
    expect(pdfPos).toEqual({ x: 110, y: PAD + NODE_H + GAP_Y });
  });

  it('分支节点排在父节点右侧，首条分支与父节点顶部对齐', () => {
    const steps = [step('a'), step('b', { branch_of: 'a' })];
    const { positions } = computeLayout(steps);
    expect(positions.get('b')).toEqual({
      x: positions.get('a')!.x + NODE_W + BRANCH_GAP_X,
      y: positions.get('a')!.y,
    });
  });

  it('同一父节点的多条分支纵向依次排开', () => {
    const steps = [
      step('a'),
      step('b1', { branch_of: 'a' }),
      step('b2', { branch_of: 'a' }),
      step('b3', { branch_of: 'a' }),
    ];
    const { positions } = computeLayout(steps);
    const y1 = positions.get('b1')!.y;
    const y2 = positions.get('b2')!.y;
    const y3 = positions.get('b3')!.y;
    expect(y1).toBe(positions.get('a')!.y);
    expect(y2).toBe(y1 + NODE_H + 30);
    expect(y3).toBe(y2 + NODE_H + 30);
  });

  it('支持分支节点的子分支（多级挂载）', () => {
    const steps = [step('a'), step('b', { branch_of: 'a' }), step('c', { branch_of: 'b' })];
    const { positions } = computeLayout(steps);
    expect(positions.get('c')).toEqual({
      x: positions.get('b')!.x + NODE_W + BRANCH_GAP_X,
      y: positions.get('b')!.y,
    });
  });

  it('父节点缺失的孤儿分支按主链排布，不会丢失坐标', () => {
    const steps = [step('a'), step('orphan', { branch_of: 'missing' })];
    const { positions } = computeLayout(steps);
    expect(positions.has('orphan')).toBe(true);
    expect(positions.get('orphan')!.y).toBe(positions.get('a')!.y + NODE_H + GAP_Y);
  });
});

describe('insertStepAt', () => {
  const steps = [step('a'), step('b')];

  it('缺省索引时追加到末尾', () => {
    expect(insertStepAt(steps, step('c')).map((item) => item.id)).toEqual(['a', 'b', 'c']);
  });

  it('支持头部与中间插入', () => {
    expect(insertStepAt(steps, step('head'), 0).map((item) => item.id)).toEqual([
      'head',
      'a',
      'b',
    ]);
    expect(insertStepAt(steps, step('mid'), 1).map((item) => item.id)).toEqual([
      'a',
      'mid',
      'b',
    ]);
  });

  it('索引越界时收敛到合法范围', () => {
    expect(insertStepAt(steps, step('end'), 99).map((item) => item.id)).toEqual([
      'a',
      'b',
      'end',
    ]);
    expect(insertStepAt(steps, step('start'), -5).map((item) => item.id)).toEqual([
      'start',
      'a',
      'b',
    ]);
  });

  it('不修改原数组', () => {
    const original = [...steps];
    insertStepAt(steps, step('c'), 0);
    expect(steps).toEqual(original);
  });
});

describe('insertRelativeStep', () => {
  it('在锚点前插入主链步骤', () => {
    const steps = [step('a'), step('b')];
    const next = insertRelativeStep(steps, step('x'), 'b', 'before');
    expect(next.map((item) => item.id)).toEqual(['a', 'x', 'b']);
    expect(next[1].branch_of).toBeUndefined();
  });

  it('在锚点后插入主链步骤', () => {
    const steps = [step('a'), step('b')];
    const next = insertRelativeStep(steps, step('x'), 'a', 'after');
    expect(next.map((item) => item.id)).toEqual(['a', 'x', 'b']);
  });

  it('添加分支时挂载到锚点并生成分支标识', () => {
    const steps = [step('a'), step('b')];
    const next = insertRelativeStep(steps, step('x'), 'a', 'branch');
    expect(next.map((item) => item.id)).toEqual(['a', 'x', 'b']);
    expect(next[1].branch_of).toBe('a');
    expect(next[1].branch).toBe('branch_a');
  });

  it('在分支节点前插入时继承同一分支组', () => {
    const steps = [step('a'), step('b1', { branch_of: 'a', branch: 'branch_a' })];
    const next = insertRelativeStep(steps, step('x'), 'b1', 'before');
    expect(next.map((item) => item.id)).toEqual(['a', 'x', 'b1']);
    expect(next[1].branch_of).toBe('a');
    expect(next[1].branch).toBe('branch_a');
  });

  it('在分支节点后插入时继承同一分支组', () => {
    const steps = [
      step('a'),
      step('b1', { branch_of: 'a', branch: 'branch_a' }),
      step('b2', { branch_of: 'a', branch: 'branch_a' }),
    ];
    const next = insertRelativeStep(steps, step('x'), 'b1', 'after');
    expect(next.map((item) => item.id)).toEqual(['a', 'b1', 'x', 'b2']);
    expect(next[2].branch_of).toBe('a');
  });

  it('锚点不存在时退化为追加到末尾', () => {
    const steps = [step('a')];
    const next = insertRelativeStep(steps, step('x'), 'missing', 'branch');
    expect(next.map((item) => item.id)).toEqual(['a', 'x']);
    expect(next[1].branch_of).toBeUndefined();
  });
});

describe('branchSiblings', () => {
  it('只返回直接挂载在父节点下的分支，保持数组顺序', () => {
    const steps = [
      step('a'),
      step('b1', { branch_of: 'a' }),
      step('b2', { branch_of: 'other' }),
      step('b3', { branch_of: 'a' }),
    ];
    expect(branchSiblings(steps, 'a').map((item) => item.id)).toEqual(['b1', 'b3']);
    expect(branchSiblings(steps, 'none')).toEqual([]);
  });
});
