import type { ProjectField, StandardField } from '../api/types';
import { normalizeDtype } from '../api/types';

/**
 * 把项目本地字段转换成「标准字段」的形状。
 *
 * 项目内只允许引用本项目字段：全局字段必须先通过「项目模板」复制成本项目字段后才会出现，
 * 因此这里不做全局字段合并，避免全局字段被隐式引用。
 */
export function projectFieldsAsStandard(fields: ProjectField[]): StandardField[] {
  return fields.map((field) => ({
    id: field.id,
    name: field.name,
    label: field.label || field.name,
    dtype: normalizeDtype(field.dtype),
    precision: field.precision,
    group_id: field.group_id,
    description: field.description,
    system: false,
  }));
}
