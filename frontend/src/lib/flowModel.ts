import type { FlowStep } from '../api/types';

/** 节点尺寸与间距（像素） */
export const NODE_W = 232;
export const NODE_H = 82;
export const GAP_Y = 74;
export const PAD = 32;
/** 分支节点相对父节点右侧的横向偏移 */
export const BRANCH_GAP_X = 130;

export interface Point {
  x: number;
  y: number;
}

export type InsertMode = 'before' | 'after' | 'branch';

/** 主链步骤：没有挂载父节点，或父节点已被删除。 */
export function isMainStep(step: FlowStep, steps: FlowStep[]): boolean {
  return !step.branch_of || !steps.some((item) => item.id === step.branch_of);
}

/**
 * 统一计算节点坐标与 PDF 锚点位置。
 * 主链纵向排列；分支挂载在父节点右侧，同一父节点的分支按数组顺序纵向排成一条支链。
 */
export function computeLayout(steps: FlowStep[]): { positions: Map<string, Point>; pdfPos: Point } {
  const positions = new Map<string, Point>();
  const mainSteps = steps.filter((step) => isMainStep(step, steps));

  mainSteps.forEach((step, index) => {
    positions.set(
      step.id,
      step.pos
        ? { x: step.pos.x, y: step.pos.y }
        : { x: 110, y: PAD + NODE_H + GAP_Y + index * (NODE_H + GAP_Y) },
    );
  });

  const lastMain = mainSteps[mainSteps.length - 1];
  const lastPos = lastMain ? positions.get(lastMain.id) : null;
  const pdfPos: Point = {
    x: 110,
    y: lastPos ? lastPos.y + NODE_H + GAP_Y : PAD + NODE_H + GAP_Y,
  };

  let pending = steps.filter((step) => !positions.has(step.id));
  let guard = 0;
  while (pending.length > 0 && guard < steps.length) {
    guard += 1;
    const next: FlowStep[] = [];
    for (const step of pending) {
      const parentPos = step.branch_of ? positions.get(step.branch_of) : null;
      if (!parentPos) {
        next.push(step);
        continue;
      }
      const placedSiblings = steps.filter(
        (item) => item.branch_of === step.branch_of && positions.has(item.id),
      ).length;
      positions.set(
        step.id,
        step.pos
          ? { x: step.pos.x, y: step.pos.y }
          : {
              x: parentPos.x + NODE_W + BRANCH_GAP_X,
              y: parentPos.y + placedSiblings * (NODE_H + 30),
            },
      );
    }
    pending = next;
  }

  // 极端情况下（父节点形成环）把剩余节点放在画布右侧，保证可见。
  pending.forEach((step, index) => {
    positions.set(step.id, { x: 110 + NODE_W + BRANCH_GAP_X, y: PAD + index * (NODE_H + 30) });
  });

  return { positions, pdfPos };
}

/** 在指定位置插入步骤，索引越界时自动收敛到合法范围。 */
export function insertStepAt(steps: FlowStep[], step: FlowStep, index?: number): FlowStep[] {
  const next = [...steps];
  const at = index === undefined ? next.length : Math.min(Math.max(index, 0), next.length);
  next.splice(at, 0, step);
  return next;
}

/**
 * 相对某个节点插入步骤：
 * - before / after：插入到锚点所在支链的相邻位置；
 * - branch：挂到锚点右侧，父节点即锚点。
 */
export function insertRelativeStep(
  steps: FlowStep[],
  step: FlowStep,
  anchorId: string,
  mode: InsertMode,
): FlowStep[] {
  const index = steps.findIndex((item) => item.id === anchorId);
  if (index < 0) return insertStepAt(steps, step);

  if (mode === 'branch') {
    return insertStepAt(
      steps,
      { ...step, branch_of: anchorId, branch: `branch_${anchorId}` },
      index + 1,
    );
  }

  // 在分支节点内部前后插入时，继续留在同一条支链。
  const anchor = steps[index];
  const inherited = anchor.branch_of
    ? { branch_of: anchor.branch_of, branch: anchor.branch }
    : {};
  return insertStepAt(
    steps,
    { ...step, ...inherited },
    mode === 'before' ? index : index + 1,
  );
}

/** 同一父节点下的分支链（按数组顺序）。 */
export function branchSiblings(steps: FlowStep[], parentId: string): FlowStep[] {
  return steps.filter((step) => step.branch_of === parentId);
}
