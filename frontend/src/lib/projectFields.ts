import type { ProjectField, StandardField } from '../api/types';
import { normalizeDtype } from '../api/types';

/**
 * 合并「全局字段库」与「项目本地字段」。
 *
 * 同名时项目字段优先：项目里改过的类型 / 说明应当覆盖全局默认值，
 * 但全局独有的字段仍然可选，避免项目模板不完整时无法导入。
 */
export function mergeFieldLists(
  globals: StandardField[],
  projectFields: ProjectField[],
): StandardField[] {
  const byName = new Map<string, StandardField>();
  for (const field of globals) {
    byName.set(field.name, field);
  }
  for (const field of projectFields) {
    byName.set(field.name, {
      id: field.id,
      name: field.name,
      label: field.label || field.name,
      dtype: normalizeDtype(field.dtype),
      precision: field.precision,
      group_id: field.group_id,
      description: field.description,
      system: false,
    });
  }
  return [...byName.values()];
}
