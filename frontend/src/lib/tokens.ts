/** 字段胶囊使用的共享解析与配色。 */

export const FIELD_COLORS = [
  '#2563eb',
  '#7c3aed',
  '#0ea5e9',
  '#16a34a',
  '#d97706',
  '#db2777',
  '#0891b2',
  '#4f46e5',
];

export function fieldColor(name: string): string {
  let hash = 0;
  for (let i = 0; i < name.length; i += 1) {
    hash = (hash * 31 + name.charCodeAt(i)) >>> 0;
  }
  return FIELD_COLORS[hash % FIELD_COLORS.length];
}

/** 把 #rrggbb 转成带透明度的 rgba()，用于字段胶囊的底色 / 边框。 */
export function hexToRgba(hex: string, alpha: number): string {
  const match = /^#?([0-9a-fA-F]{6})$/.exec(hex.trim());
  if (!match) return `rgba(100, 116, 139, ${alpha})`;
  const value = parseInt(match[1], 16);
  const r = (value >> 16) & 255;
  const g = (value >> 8) & 255;
  const b = value & 255;
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

export interface ExprToken {
  kind: 'text' | 'field';
  text: string;
  /** 中括号里的字段名 */
  field?: string;
}

/** 把 [字段名] 表达式拆成文本与字段片段，用于圆角胶囊渲染。 */
export function splitExprTokens(value: string): ExprToken[] {
  const tokens: ExprToken[] = [];
  const re = /\[([^\]]+)\]/g;
  let last = 0;
  let match: RegExpExecArray | null;
  while ((match = re.exec(value))) {
    if (match.index > last) {
      tokens.push({ kind: 'text', text: value.slice(last, match.index) });
    }
    tokens.push({ kind: 'field', text: match[0], field: match[1].trim() });
    last = match.index + match[0].length;
  }
  if (last < value.length) tokens.push({ kind: 'text', text: value.slice(last) });
  return tokens;
}

export interface TemplateToken {
  kind: 'text' | 'field' | 'builtin';
  /** 原始文本；字段片段包含 {{ }} 或 { } 包裹 */
  text: string;
  /** 字段名或内置变量名 */
  field?: string;
}

/**
 * PDF 模板占位符：`{{字段}}`（推荐）、`{字段}`（兼容旧模板）与内置变量。
 * 返回的字段片段保留原始文本，便于编辑时还原。
 */
export function splitTemplateTokens(value: string): TemplateToken[] {
  const tokens: TemplateToken[] = [];
  const re = /\{\{\s*([^{}]+?)\s*\}\}|\{\s*([^{}]+?)\s*\}/g;
  let last = 0;
  let match: RegExpExecArray | null;
  while ((match = re.exec(value))) {
    if (match.index > last) {
      tokens.push({ kind: 'text', text: value.slice(last, match.index) });
    }
    const name = (match[1] ?? match[2] ?? '').trim();
    const builtin = ['index', 'no', 'page', 'pages', 'count'].includes(name);
    tokens.push({
      kind: builtin ? 'builtin' : 'field',
      text: match[0],
      field: name,
    });
    last = match.index + match[0].length;
  }
  if (last < value.length) tokens.push({ kind: 'text', text: value.slice(last) });
  return tokens;
}

/** 从文本中提取全部模板字段名（忽略内置变量）。 */
export function templateFields(value: string): string[] {
  const names: string[] = [];
  for (const token of splitTemplateTokens(value)) {
    if (token.kind === 'field' && token.field && !names.includes(token.field)) {
      names.push(token.field);
    }
  }
  return names;
}
