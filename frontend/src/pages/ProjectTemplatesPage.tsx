import { useEffect, useMemo, useState } from 'react';
import { Input, InputNumber, Modal, Select } from 'antd';
import {
  copyProjectFields,
  deletePdfTemplate,
  deleteProjectField,
  duplicatePdfTemplate,
  listPdfTemplates,
  listProjectFields,
  listStandardFields,
  updateProjectField,
} from '../api/client';
import type {
  ColumnDtype,
  PdfTemplate,
  ProjectField,
  ProjectFieldUpdate,
  StandardField,
} from '../api/types';
import { PRECISION_DTYPES, normalizeDtype } from '../api/types';
import { EmptyState } from '../components/EmptyState';
import { Field } from '../components/Field';
import { Spinner } from '../components/Spinner';
import { useToast } from '../components/Toast';
import { useProject } from '../context/ProjectContext';

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

interface FieldDraft {
  name: string;
  dtype: ColumnDtype;
  precision: number;
  description: string;
}

function dtypeNeedsPrecision(dtype: ColumnDtype): boolean {
  return PRECISION_DTYPES.includes(normalizeDtype(dtype));
}

export default function ProjectTemplatesPage({ onEditTemplate, onNewTemplate }: Props) {
  const { show } = useToast();
  const { currentProject, currentProjectId } = useProject();

  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [tab, setTab] = useState<'fields' | 'pdf'>('fields');

  const [fields, setFields] = useState<ProjectField[]>([]);
  const [templates, setTemplates] = useState<PdfTemplate[]>([]);
  const [globalFields, setGlobalFields] = useState<StandardField[]>([]);
  const [globalTemplates, setGlobalTemplates] = useState<PdfTemplate[]>([]);

  // 从全局字段库挑选
  const [pickFieldsOpen, setPickFieldsOpen] = useState(false);
  const [pickedFieldIds, setPickedFieldIds] = useState<string[]>([]);

  // 从全局模板库复制
  const [pickTemplatesOpen, setPickTemplatesOpen] = useState(false);
  const [pickedTemplateIds, setPickedTemplateIds] = useState<string[]>([]);

  // 编辑项目字段
  const [editingField, setEditingField] = useState<ProjectField | null>(null);
  const [fieldDraft, setFieldDraft] = useState<FieldDraft | null>(null);

  const load = async (projectId: string) => {
    setLoading(true);
    try {
      const [projectFieldList, projectTemplates, allFields, allTemplates] = await Promise.all([
        listProjectFields(projectId),
        listPdfTemplates({ projectId }),
        listStandardFields(),
        listPdfTemplates({ scope: 'global' }),
      ]);
      setFields(projectFieldList);
      setTemplates(projectTemplates);
      setGlobalFields(allFields);
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
      setTemplates([]);
      setGlobalFields([]);
      setGlobalTemplates([]);
      return;
    }
    void load(currentProjectId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentProjectId]);

  const existingNames = useMemo(() => new Set(fields.map((field) => field.name)), [fields]);

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

  const openEditField = (field: ProjectField) => {
    setEditingField(field);
    setFieldDraft({
      name: field.name,
      dtype: normalizeDtype(field.dtype),
      precision: field.precision,
      description: field.description,
    });
  };

  const saveField = async () => {
    if (!editingField || !fieldDraft) return;
    const name = fieldDraft.name.trim();
    if (!name) {
      show('字段名不能为空', 'error');
      return;
    }
    const payload: ProjectFieldUpdate = {
      name,
      label: name,
      dtype: fieldDraft.dtype,
      precision: dtypeNeedsPrecision(fieldDraft.dtype) ? fieldDraft.precision : 0,
      description: fieldDraft.description,
    };
    setBusy(true);
    try {
      const saved = await updateProjectField(editingField.id, payload);
      setFields((prev) => prev.map((item) => (item.id === saved.id ? saved : item)));
      setEditingField(null);
      setFieldDraft(null);
      show('已保存到本项目', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '保存字段失败', 'error');
    } finally {
      setBusy(false);
    }
  };

  const removeField = async (field: ProjectField) => {
    setBusy(true);
    try {
      await deleteProjectField(field.id);
      setFields((prev) => prev.filter((item) => item.id !== field.id));
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
                <table className="data-table align-center bordered">
                  <thead className="data-table__head">
                    <tr>
                      <th>字段名</th>
                      <th>类型</th>
                      <th>精度</th>
                      <th>说明</th>
                      <th>来源</th>
                      <th>操作</th>
                    </tr>
                  </thead>
                  <tbody>
                    {fields.map((field) => (
                      <tr key={field.id}>
                        <td>{field.name}</td>
                        <td>{DTYPE_LABELS[normalizeDtype(field.dtype)] ?? field.dtype}</td>
                        <td>{dtypeNeedsPrecision(normalizeDtype(field.dtype)) ? field.precision : '—'}</td>
                        <td>{field.description || '—'}</td>
                        <td>{field.source_id ? '全局复制' : '本项目新建'}</td>
                        <td>
                          <div className="row-actions">
                            <button className="btn btn-sm" disabled={busy} onClick={() => openEditField(field)}>
                              编辑
                            </button>
                            <button className="btn btn-sm" disabled={busy} onClick={() => removeField(field)}>
                              移除
                            </button>
                          </div>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
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
        <div className="pick-list">
          {globalFields.map((field) => {
            const exists = existingNames.has(field.name);
            const checked = pickedFieldIds.includes(field.id);
            return (
              <label key={field.id} className={`pick-row ${exists ? 'disabled' : ''}`}>
                <input
                  type="checkbox"
                  disabled={exists}
                  checked={checked}
                  onChange={(event) =>
                    setPickedFieldIds((prev) =>
                      event.target.checked
                        ? [...prev, field.id]
                        : prev.filter((id) => id !== field.id),
                    )
                  }
                />
                <span>
                  {field.name}
                  <span className="detail-meta">
                    {' '}
                    · {DTYPE_LABELS[normalizeDtype(field.dtype)] ?? field.dtype}
                    {exists ? ' · 已在本项目' : ''}
                  </span>
                </span>
              </label>
            );
          })}
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

      {/* 编辑项目字段 */}
      <Modal
        open={editingField !== null}
        title="编辑项目字段"
        okText="保存到本项目"
        cancelText="取消"
        confirmLoading={busy}
        onOk={saveField}
        onCancel={() => {
          setEditingField(null);
          setFieldDraft(null);
        }}
      >
        {fieldDraft ? (
          <div className="column">
            <Field label="字段名">
              <Input
                value={fieldDraft.name}
                onChange={(event) => setFieldDraft({ ...fieldDraft, name: event.target.value })}
              />
            </Field>
            <Field label="数据类型">
              <Select
                style={{ width: 200 }}
                value={fieldDraft.dtype}
                onChange={(value: ColumnDtype) => setFieldDraft({ ...fieldDraft, dtype: value })}
                options={DTYPES.map((dtype) => ({ value: dtype, label: DTYPE_LABELS[dtype] }))}
              />
            </Field>
            {dtypeNeedsPrecision(fieldDraft.dtype) ? (
              <Field label="保留小数位">
                <InputNumber
                  min={0}
                  max={6}
                  value={fieldDraft.precision}
                  onChange={(value) =>
                    setFieldDraft({ ...fieldDraft, precision: Number(value) || 0 })
                  }
                />
              </Field>
            ) : null}
            <Field label="说明">
              <Input
                value={fieldDraft.description}
                onChange={(event) =>
                  setFieldDraft({ ...fieldDraft, description: event.target.value })
                }
              />
            </Field>
            <p className="panel-hint">修改只保存到「{currentProject?.name ?? '本项目'}」，不会影响全局字段库。</p>
          </div>
        ) : null}
      </Modal>
    </div>
  );
}
