import { useEffect, useMemo, useState } from 'react';
import { Modal } from 'antd';
import {
  copyProjectFields,
  deletePdfTemplate,
  deleteProjectAlias,
  deleteProjectField,
  duplicatePdfTemplate,
  listFieldGroups,
  listPdfTemplates,
  listProjectAliases,
  listProjectFields,
  listStandardFields,
  saveProjectAlias,
  updateProjectField,
} from '../api/client';
import type {
  ColumnDtype,
  FieldGroup,
  PdfTemplate,
  ProjectAlias,
  ProjectField,
  ProjectFieldUpdate,
  StandardField,
} from '../api/types';
import { PRECISION_DTYPES, normalizeDtype } from '../api/types';
import { EmptyState } from '../components/EmptyState';
import { Spinner } from '../components/Spinner';
import { useToast } from '../components/Toast';
import { useProject } from '../context/ProjectContext';
import { InlineSelect, PrecisionInput } from './AliasPage';

const DTYPES: ColumnDtype[] = ['text', 'integer', 'decimal', 'percent', 'date', 'datetime', 'boolean'];

const DTYPE_LABELS: Record<string, string> = {
  text: '文本',
  integer: '整数',
  decimal: '小数',
  percent: '百分比',
  date: '日期',
  datetime: '日期时间',
  boolean: '布尔',
};

interface Props {
  onEditTemplate: (id: string) => void;
  onNewTemplate: () => void;
}

function dtypeNeedsPrecision(dtype: ColumnDtype): boolean {
  return PRECISION_DTYPES.includes(normalizeDtype(dtype));
}

const DTYPE_OPTIONS = DTYPES.map((dtype) => ({ value: dtype, label: DTYPE_LABELS[dtype] }));

export default function ProjectTemplatesPage({ onEditTemplate, onNewTemplate }: Props) {
  const { show } = useToast();
  const { currentProject, currentProjectId } = useProject();

  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [tab, setTab] = useState<'fields' | 'pdf'>('fields');

  const [fields, setFields] = useState<ProjectField[]>([]);
  const [aliases, setAliases] = useState<ProjectAlias[]>([]);
  const [templates, setTemplates] = useState<PdfTemplate[]>([]);
  const [globalFields, setGlobalFields] = useState<StandardField[]>([]);
  const [groups, setGroups] = useState<FieldGroup[]>([]);
  const [globalTemplates, setGlobalTemplates] = useState<PdfTemplate[]>([]);

  // 从全局字段库挑选
  const [pickFieldsOpen, setPickFieldsOpen] = useState(false);
  const [pickedFieldIds, setPickedFieldIds] = useState<string[]>([]);

  // 从全局模板库复制
  const [pickTemplatesOpen, setPickTemplatesOpen] = useState(false);
  const [pickedTemplateIds, setPickedTemplateIds] = useState<string[]>([]);

  // 卡片内联编辑草稿（字段名 / 新增别名）
  const [nameDrafts, setNameDrafts] = useState<Record<string, string>>({});
  const [aliasDrafts, setAliasDrafts] = useState<Record<string, string>>({});

  const load = async (projectId: string) => {
    setLoading(true);
    try {
      const [
        projectFieldList,
        projectAliasList,
        projectTemplates,
        allFields,
        groupList,
        allTemplates,
      ] = await Promise.all([
        listProjectFields(projectId),
        listProjectAliases(projectId),
        listPdfTemplates({ projectId }),
        listStandardFields(),
        listFieldGroups(),
        listPdfTemplates({ scope: 'global' }),
      ]);
      setFields(projectFieldList);
      setAliases(projectAliasList);
      setTemplates(projectTemplates);
      setGlobalFields(allFields);
      setGroups(groupList);
      setGlobalTemplates(allTemplates);
    } catch (err) {
      show(err instanceof Error ? err.message : '加载项目模板失败', 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (!currentProjectId) {
      setFields([]);
      setAliases([]);
      setTemplates([]);
      setGlobalFields([]);
      setGroups([]);
      setGlobalTemplates([]);
      return;
    }
    void load(currentProjectId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentProjectId]);

  const existingNames = useMemo(() => new Set(fields.map((field) => field.name)), [fields]);

  /** 某字段在本项目内的别名（卡片渲染用）。 */
  const fieldAliases = (name: string) => aliases.filter((alias) => alias.standard === name);

  /** 全局字段按分组拆开，「未分组」排最后，供挑选弹层分区展示。 */
  const groupedGlobalFields = useMemo(() => {
    const known = new Set(groups.map((group) => group.id));
    const result = groups.map((group) => ({
      id: group.id,
      name: group.name,
      fields: globalFields.filter((field) => field.group_id === group.id),
    }));
    const ungrouped = globalFields.filter((field) => !known.has(field.group_id));
    if (ungrouped.length) {
      result.push({ id: '', name: '未分组', fields: ungrouped });
    }
    return result.filter((group) => group.fields.length > 0);
  }, [groups, globalFields]);

  const togglePickField = (id: string) => {
    setPickedFieldIds((prev) =>
      prev.includes(id) ? prev.filter((item) => item !== id) : [...prev, id],
    );
  };

  const openPickFields = () => {
    setPickedFieldIds([]);
    setPickFieldsOpen(true);
  };

  const confirmPickFields = async () => {
    if (!currentProjectId || pickedFieldIds.length === 0) {
      setPickFieldsOpen(false);
      return;
    }
    setBusy(true);
    try {
      const result = await copyProjectFields(currentProjectId, pickedFieldIds);
      show(`已复制 ${result.copied} 个字段到本项目`, 'success');
      setPickFieldsOpen(false);
      await load(currentProjectId);
    } catch (err) {
      show(err instanceof Error ? err.message : '复制字段失败', 'error');
    } finally {
      setBusy(false);
    }
  };

  /** 卡片内联修改：只提交变化的字段，保存到本项目。 */
  const patchField = async (field: ProjectField, patch: Partial<ProjectFieldUpdate>) => {
    const payload: ProjectFieldUpdate = {
      name: field.name,
      label: field.label,
      dtype: normalizeDtype(field.dtype),
      precision: field.precision,
      group_id: field.group_id,
      description: field.description,
      ...patch,
    };
    try {
      const saved = await updateProjectField(field.id, payload);
      setFields((prev) => prev.map((item) => (item.id === saved.id ? saved : item)));
    } catch (err) {
      show(err instanceof Error ? err.message : '保存字段失败', 'error');
    }
  };

  const commitName = async (field: ProjectField) => {
    const raw = nameDrafts[field.id];
    if (raw === undefined) return;
    const name = raw.trim();
    if (!name) {
      show('字段名不能为空', 'error');
      setNameDrafts((prev) => ({ ...prev, [field.id]: field.name }));
      return;
    }
    setNameDrafts((prev) => {
      const next = { ...prev };
      delete next[field.id];
      return next;
    });
    if (name !== field.name) {
      await patchField(field, { name, label: name });
      setAliases((prev) =>
        prev.map((item) => (item.standard === field.name ? { ...item, standard: name } : item)),
      );
    }
  };

  /** 给项目字段添加别名（与全局别名页一致：回车或点「添加」）。 */
  const addAlias = async (field: ProjectField) => {
    const raw = (aliasDrafts[field.name] ?? '').trim();
    if (!currentProjectId || !raw) return;
    setBusy(true);
    try {
      const saved = await saveProjectAlias({
        project_id: currentProjectId,
        alias: raw,
        standard: field.name,
      });
      setAliases((prev) => {
        const rest = prev.filter((item) => item.id !== saved.id && item.alias !== saved.alias);
        return [...rest, saved];
      });
      setAliasDrafts((prev) => ({ ...prev, [field.name]: '' }));
    } catch (err) {
      show(err instanceof Error ? err.message : '添加别名失败', 'error');
    } finally {
      setBusy(false);
    }
  };

  const removeAlias = async (alias: ProjectAlias) => {
    setBusy(true);
    try {
      await deleteProjectAlias(alias.id);
      setAliases((prev) => prev.filter((item) => item.id !== alias.id));
    } catch (err) {
      show(err instanceof Error ? err.message : '删除别名失败', 'error');
    } finally {
      setBusy(false);
    }
  };

  const removeField = async (field: ProjectField) => {
    setBusy(true);
    try {
      await deleteProjectField(field.id);
      setFields((prev) => prev.filter((item) => item.id !== field.id));
      setAliases((prev) => prev.filter((item) => item.standard !== field.name));
      show('已从本项目移除', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '删除字段失败', 'error');
    } finally {
      setBusy(false);
    }
  };

  const confirmPickTemplates = async () => {
    if (!currentProjectId || pickedTemplateIds.length === 0) {
      setPickTemplatesOpen(false);
      return;
    }
    setBusy(true);
    try {
      for (const id of pickedTemplateIds) {
        await duplicatePdfTemplate(id, { project_id: currentProjectId });
      }
      show(`已复制 ${pickedTemplateIds.length} 个模板到本项目`, 'success');
      setPickTemplatesOpen(false);
      await load(currentProjectId);
    } catch (err) {
      show(err instanceof Error ? err.message : '复制模板失败', 'error');
    } finally {
      setBusy(false);
    }
  };

  const removeTemplate = async (template: PdfTemplate) => {
    setBusy(true);
    try {
      await deletePdfTemplate(template.id);
      setTemplates((prev) => prev.filter((item) => item.id !== template.id));
      show('已从本项目移除模板', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '删除模板失败', 'error');
    } finally {
      setBusy(false);
    }
  };

  if (!currentProjectId) {
    return (
      <div className="page">
        <div className="page-header">
          <div>
            <h2 className="page-title">项目模板</h2>
            <p className="page-subtitle">从全局库复制字段与 PDF 模板，成为本项目独占的副本。</p>
          </div>
        </div>
        <div className="page-fill">
          <EmptyState text="请先在顶部选择一个考试项目" />
        </div>
      </div>
    );
  }

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h2 className="page-title">项目模板</h2>
          <p className="page-subtitle">
            {currentProject?.name ?? '当前项目'} 的本地模板。修改只保存到本项目，不影响全局库。
          </p>
        </div>
        <div className="row-actions">
          <button className="btn btn-sm" disabled={loading} onClick={() => load(currentProjectId)}>
            刷新
          </button>
        </div>
      </div>

      <div className="project-templates page-fill">
        <div className="template-tabs">
          <button
            className={`tab-btn ${tab === 'fields' ? 'active' : ''}`}
            onClick={() => setTab('fields')}
          >
            字段模板（{fields.length}）
          </button>
          <button
            className={`tab-btn ${tab === 'pdf' ? 'active' : ''}`}
            onClick={() => setTab('pdf')}
          >
            PDF 模板（{templates.length}）
          </button>
        </div>

        {loading ? (
          <div className="table-state">
            <Spinner />
            <span className="table-state-text">加载中…</span>
          </div>
        ) : tab === 'fields' ? (
          <section className="card card-fill">
            <div className="card-head-row">
              <div>
                <h3 className="card-title" style={{ margin: 0 }}>
                  字段模板
                </h3>
                <div className="detail-meta">从全局字段别名管理中挑选，复制后可在本项目内改名、改类型。</div>
              </div>
              <div className="row-actions">
                <button className="btn btn-sm" disabled={busy} onClick={openPickFields}>
                  从全局字段添加
                </button>
              </div>
            </div>
            <div className="card-scroll">
              {fields.length === 0 ? (
                <EmptyState text="本项目还没有字段模板。点击「从全局字段添加」选择需要的字段。" />
              ) : (
                <div className="alias-grid">
                  {fields.map((field) => {
                    const dtype = normalizeDtype(field.dtype);
                    return (
                      <div className="alias-card" key={field.id}>
                        <div className="alias-card-top">
                          <input
                            className="alias-name-input"
                            value={nameDrafts[field.id] ?? field.name}
                            onChange={(event) =>
                              setNameDrafts((prev) => ({ ...prev, [field.id]: event.target.value }))
                            }
                            onBlur={() => void commitName(field)}
                            onKeyDown={(event) => {
                              if (event.key === 'Enter') event.currentTarget.blur();
                              if (event.key === 'Escape') {
                                setNameDrafts((prev) => {
                                  const next = { ...prev };
                                  delete next[field.id];
                                  return next;
                                });
                              }
                            }}
                          />
                          <button
                            className="alias-chip-remove"
                            title="从本项目移除"
                            disabled={busy}
                            onClick={() => void removeField(field)}
                          >
                            ✕
                          </button>
                        </div>

                        <div className="alias-card-meta">
                          <div className="alias-type-combo">
                            <InlineSelect
                              value={dtype}
                              options={DTYPE_OPTIONS}
                              onChange={(value) => {
                                const next = value as ColumnDtype;
                                patchField(field, {
                                  dtype: next,
                                  precision: dtypeNeedsPrecision(next) ? field.precision : 0,
                                });
                              }}
                            />
                            {dtypeNeedsPrecision(dtype) ? (
                              <PrecisionInput
                                value={field.precision ?? 2}
                                onChange={(value) =>
                                  setFields((prev) =>
                                    prev.map((item) =>
                                      item.id === field.id ? { ...item, precision: value } : item,
                                    ),
                                  )
                                }
                                onCommit={(value) => patchField(field, { precision: value })}
                              />
                            ) : null}
                          </div>
                          <span className="detail-meta">
                            {field.source_id ? '来自全局字段' : '本项目新建'}
                          </span>
                        </div>

                        <div className="alias-chips">
                          {fieldAliases(field.name).length === 0 ? (
                            <span className="alias-empty">暂无别名</span>
                          ) : (
                            <>
                              <span className="alias-chip-prefix">别名：</span>
                              {fieldAliases(field.name).map((alias) => (
                                <span key={alias.id} className="alias-chip">
                                  {alias.alias}
                                  <button
                                    className="alias-chip-remove"
                                    title="删除该别名"
                                    onClick={() => void removeAlias(alias)}
                                  >
                                    ✕
                                  </button>
                                </span>
                              ))}
                            </>
                          )}
                        </div>

                        <div className="alias-add">
                          <input
                            type="text"
                            placeholder="添加别名，回车确认"
                            value={aliasDrafts[field.name] ?? ''}
                            onChange={(event) =>
                              setAliasDrafts((prev) => ({
                                ...prev,
                                [field.name]: event.target.value,
                              }))
                            }
                            onKeyDown={(event) => {
                              if (event.key === 'Enter') void addAlias(field);
                            }}
                          />
                          <button
                            className="btn btn-sm"
                            disabled={busy || !(aliasDrafts[field.name] ?? '').trim()}
                            onClick={() => void addAlias(field)}
                          >
                            添加
                          </button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          </section>
        ) : (
          <section className="card card-fill">
            <div className="card-head-row">
              <div>
                <h3 className="card-title" style={{ margin: 0 }}>
                  PDF 模板
                </h3>
                <div className="detail-meta">复制全局模板后，可在本项目中自由修改并保存，不影响全局模板库。</div>
              </div>
              <div className="row-actions">
                <button
                  className="btn btn-sm"
                  disabled={busy}
                  onClick={() => {
                    setPickedTemplateIds([]);
                    setPickTemplatesOpen(true);
                  }}
                >
                  从全局模板复制
                </button>
                <button className="btn btn-sm btn-primary" disabled={busy} onClick={onNewTemplate}>
                  新建空白模板
                </button>
              </div>
            </div>
            <div className="card-scroll">
              {templates.length === 0 ? (
                <EmptyState text="本项目还没有 PDF 模板。可从全局模板库复制，或新建空白模板。" />
              ) : (
                <div className="project-template-grid">
                  {templates.map((template) => (
                    <div className="project-template-card" key={template.id}>
                      <div className="project-template-card__head">
                        <span className="project-template-card__name">{template.name}</span>
                        <span className="badge badge-standard">
                          {template.kind === 'matrix' ? '矩阵' : '单人'}
                        </span>
                      </div>
                      <div className="project-template-card__meta">
                        {template.source_id ? '来自全局模板' : '本项目新建'} · 更新于 {template.updated_at}
                      </div>
                      <div className="row-actions">
                        <button className="btn btn-sm" disabled={busy} onClick={() => onEditTemplate(template.id)}>
                          编辑
                        </button>
                        <button className="btn btn-sm" disabled={busy} onClick={() => removeTemplate(template)}>
                          移除
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </section>
        )}
      </div>

      {/* 从全局字段库挑选 */}
      <Modal
        open={pickFieldsOpen}
        title="从全局字段库添加"
        okText="复制到本项目"
        cancelText="取消"
        confirmLoading={busy}
        okButtonProps={{ disabled: pickedFieldIds.length === 0 }}
        onOk={confirmPickFields}
        onCancel={() => setPickFieldsOpen(false)}
      >
        <p className="panel-hint">
          已有同名字段会自动跳过；复制后可在本项目内独立修改。
        </p>
        <div className="pick-groups">
          {groupedGlobalFields.length === 0 ? (
            <EmptyState text="全局字段库还没有字段" />
          ) : (
            groupedGlobalFields.map((group) => (
              <div className="pick-group" key={group.id || 'ungrouped'}>
                <div className="pick-group-title">
                  {group.name}
                  <span className="pick-group-count">{group.fields.length}</span>
                </div>
                <div className="pick-tiles">
                  {group.fields.map((field) => {
                    const exists = existingNames.has(field.name);
                    const selected = pickedFieldIds.includes(field.id);
                    return (
                      <button
                        type="button"
                        key={field.id}
                        className={`pick-tile ${selected ? 'selected' : ''} ${
                          exists ? 'disabled' : ''
                        }`}
                        disabled={exists}
                        title={exists ? '已在本项目' : `复制「${field.name}」到本项目`}
                        onClick={() => {
                          if (!exists) togglePickField(field.id);
                        }}
                      >
                        <span className="pick-tile-name">{field.name}</span>
                        <span className="pick-tile-meta">
                          {exists
                            ? '已在本项目'
                            : (DTYPE_LABELS[normalizeDtype(field.dtype)] ?? field.dtype)}
                        </span>
                      </button>
                    );
                  })}
                </div>
              </div>
            ))
          )}
        </div>
      </Modal>

      {/* 从全局模板库复制 */}
      <Modal
        open={pickTemplatesOpen}
        title="从全局模板库复制"
        okText="复制到本项目"
        cancelText="取消"
        confirmLoading={busy}
        okButtonProps={{ disabled: pickedTemplateIds.length === 0 }}
        onOk={confirmPickTemplates}
        onCancel={() => setPickTemplatesOpen(false)}
      >
        <div className="pick-list">
          {globalTemplates.length === 0 ? (
            <EmptyState text="全局模板库还没有模板" />
          ) : (
            globalTemplates.map((template) => (
              <label key={template.id} className="pick-row">
                <input
                  type="checkbox"
                  checked={pickedTemplateIds.includes(template.id)}
                  onChange={(event) =>
                    setPickedTemplateIds((prev) =>
                      event.target.checked
                        ? [...prev, template.id]
                        : prev.filter((id) => id !== template.id),
                    )
                  }
                />
                <span>
                  {template.name}
                  <span className="detail-meta"> · {template.kind === 'matrix' ? '矩阵' : '单人'}</span>
                </span>
              </label>
            ))
          )}
        </div>
      </Modal>
    </div>
  );
}
