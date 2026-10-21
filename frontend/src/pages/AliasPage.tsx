import { useEffect, useMemo, useState } from 'react';
import {
  createFieldGroup,
  createStandardField,
  deleteAlias,
  deleteFieldGroup,
  deleteStandardField,
  listAliases,
  listFieldGroups,
  listStandardFields,
  saveAliases,
  updateFieldGroup,
  updateStandardField,
} from '../api/client';
import type { AliasMapping, ColumnDtype, FieldGroup, StandardField } from '../api/types';
import { PRECISION_DTYPES, normalizeDtype } from '../api/types';
import { EmptyState } from '../components/EmptyState';
import { Field } from '../components/Field';
import { Modal } from '../components/Modal';
import { Spinner } from '../components/Spinner';
import { useToast } from '../components/Toast';

const DTYPES: ColumnDtype[] = [
  'text',
  'integer',
  'decimal',
  'percent',
  'date',
  'datetime',
  'boolean',
  'image',
];

const DTYPE_LABELS: Record<string, string> = {
  text: '文本',
  integer: '整数',
  decimal: '小数',
  percent: '百分比',
  date: '日期',
  datetime: '日期时间',
  boolean: '布尔',
  image: '图片',
};

function dtypeText(field: StandardField): string {
  const dtype = normalizeDtype(field.dtype);
  const label = DTYPE_LABELS[dtype] ?? dtype;
  if (PRECISION_DTYPES.includes(dtype)) {
    return `${label}（${field.precision} 位小数）`;
  }
  return label;
}

interface FieldForm {
  id: string | null;
  name: string;
  label: string;
  dtype: ColumnDtype;
  precision: number;
  groupId: string;
  description: string;
}

const EMPTY_FORM: FieldForm = {
  id: null,
  name: '',
  label: '',
  dtype: 'text',
  precision: 2,
  groupId: '',
  description: '',
};

interface AliasCard {
  standard: string;
  aliases: AliasMapping[];
  field: StandardField | null;
}

type GroupKey = 'all' | 'ungrouped' | string;

export default function AliasPage() {
  const { show } = useToast();

  const [fields, setFields] = useState<StandardField[]>([]);
  const [aliases, setAliases] = useState<AliasMapping[]>([]);
  const [groups, setGroups] = useState<FieldGroup[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState('');
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [activeGroup, setActiveGroup] = useState<GroupKey>('all');

  const [modalOpen, setModalOpen] = useState(false);
  const [form, setForm] = useState<FieldForm>(EMPTY_FORM);
  const [savingField, setSavingField] = useState(false);

  const [groupModalOpen, setGroupModalOpen] = useState(false);
  const [groupForm, setGroupForm] = useState<{ id: string | null; name: string }>({
    id: null,
    name: '',
  });
  const [savingGroup, setSavingGroup] = useState(false);

  const loadAll = async () => {
    setLoading(true);
    try {
      const [fieldList, aliasList, groupList] = await Promise.all([
        listStandardFields(),
        listAliases(),
        listFieldGroups(),
      ]);
      setFields(fieldList);
      setAliases(aliasList);
      setGroups(groupList);
      setActiveGroup((prev) => {
        if (prev === 'all' || prev === 'ungrouped') return prev;
        return groupList.some((group) => group.id === prev) ? prev : 'all';
      });
    } catch (err) {
      show(err instanceof Error ? err.message : '加载字段别名失败', 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadAll();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const cards = useMemo<AliasCard[]>(() => {
    const byStandard = new Map<string, AliasMapping[]>();
    for (const alias of aliases) {
      const list = byStandard.get(alias.standard) ?? [];
      list.push(alias);
      byStandard.set(alias.standard, list);
    }
    const result: AliasCard[] = [];
    const consumed = new Set<string>();
    for (const field of fields) {
      result.push({ standard: field.name, aliases: byStandard.get(field.name) ?? [], field });
      consumed.add(field.name);
    }
    for (const [standard, list] of byStandard) {
      if (!consumed.has(standard)) {
        result.push({ standard, aliases: list, field: null });
      }
    }
    return result;
  }, [fields, aliases]);

  const searchedCards = useMemo(() => {
    const query = search.trim().toLowerCase();
    if (!query) return cards;
    return cards.filter(
      (card) =>
        card.standard.toLowerCase().includes(query) ||
        card.field?.label.toLowerCase().includes(query) ||
        card.aliases.some((alias) => alias.alias.toLowerCase().includes(query)),
    );
  }, [cards, search]);

  const groupKeyOf = (card: AliasCard): string => card.field?.group_id || '';

  /** 有字段的分组 + 未分组；空分组也显示，方便往里添加字段 */
  const sections = useMemo(() => {
    const buckets = new Map<string, AliasCard[]>();
    groups.forEach((group) => buckets.set(group.id, []));
    buckets.set('', []);
    for (const card of searchedCards) {
      const key = groupKeyOf(card);
      const list = buckets.get(key) ?? [];
      list.push(card);
      buckets.set(key, list);
    }
    return [...buckets.entries()].filter(([key]) => {
      if (activeGroup === 'all') return true;
      if (activeGroup === 'ungrouped') return key === '';
      return key === activeGroup;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchedCards, groups, activeGroup]);

  const countOf = (key: GroupKey): number => {
    if (key === 'all') return cards.length;
    if (key === 'ungrouped') return cards.filter((card) => !groupKeyOf(card)).length;
    return cards.filter((card) => groupKeyOf(card) === key).length;
  };

  const addAlias = async (standard: string) => {
    const value = (drafts[standard] ?? '').trim();
    if (!value) return;
    setBusy(true);
    try {
      await saveAliases([{ alias: value, standard }]);
      setDrafts((prev) => ({ ...prev, [standard]: '' }));
      setAliases(await listAliases());
      show(`已为「${standard}」添加别名「${value}」`, 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '添加别名失败', 'error');
    } finally {
      setBusy(false);
    }
  };

  const removeAlias = async (alias: AliasMapping) => {
    try {
      await deleteAlias(alias.id);
      setAliases((prev) => prev.filter((item) => item.id !== alias.id));
    } catch (err) {
      show(err instanceof Error ? err.message : '删除别名失败', 'error');
    }
  };

  const openCreateField = () => {
    setForm({
      ...EMPTY_FORM,
      groupId: activeGroup === 'all' || activeGroup === 'ungrouped' || search ? '' : activeGroup,
    });
    setModalOpen(true);
  };

  const openEditField = (field: StandardField) => {
    setForm({
      id: field.id,
      name: field.name,
      label: field.label,
      dtype: normalizeDtype(field.dtype),
      precision: field.precision ?? 2,
      groupId: field.group_id ?? '',
      description: field.description,
    });
    setModalOpen(true);
  };

  const submitField = async () => {
    if (!form.name.trim()) {
      show('请填写标准名称', 'error');
      return;
    }
    setSavingField(true);
    try {
      const payload = {
        name: form.name.trim(),
        label: form.label.trim() || form.name.trim(),
        dtype: form.dtype,
        precision: PRECISION_DTYPES.includes(form.dtype) ? Math.max(0, Math.min(10, form.precision)) : 0,
        group_id: form.groupId,
        description: form.description.trim(),
      };
      if (form.id) {
        await updateStandardField(form.id, payload);
        show('标准字段已更新', 'success');
      } else {
        await createStandardField(payload);
        show('标准字段已创建', 'success');
      }
      setModalOpen(false);
      await loadAll();
    } catch (err) {
      show(err instanceof Error ? err.message : '保存失败', 'error');
    } finally {
      setSavingField(false);
    }
  };

  const removeField = async (card: AliasCard) => {
    if (!card.field) return;
    if (!window.confirm(`确认删除标准字段「${card.field.label}」？（其别名卡片会一并移除）`)) {
      return;
    }
    try {
      await deleteStandardField(card.field.id);
      for (const alias of card.aliases) {
        await deleteAlias(alias.id);
      }
      show('已删除标准字段', 'success');
      await loadAll();
    } catch (err) {
      show(err instanceof Error ? err.message : '删除失败', 'error');
    }
  };

  // ------------------------- 分组管理 -------------------------

  const openCreateGroup = () => {
    setGroupForm({ id: null, name: '' });
    setGroupModalOpen(true);
  };

  const openRenameGroup = (group: FieldGroup) => {
    setGroupForm({ id: group.id, name: group.name });
    setGroupModalOpen(true);
  };

  const submitGroup = async () => {
    if (!groupForm.name.trim()) {
      show('请填写分组名称', 'error');
      return;
    }
    setSavingGroup(true);
    try {
      if (groupForm.id) {
        await updateFieldGroup(groupForm.id, { name: groupForm.name.trim() });
        show('分组已重命名', 'success');
      } else {
        await createFieldGroup({ name: groupForm.name.trim() });
        show('分组已创建', 'success');
      }
      setGroupModalOpen(false);
      await loadAll();
    } catch (err) {
      show(err instanceof Error ? err.message : '保存分组失败', 'error');
    } finally {
      setSavingGroup(false);
    }
  };

  const removeGroup = async (group: FieldGroup) => {
    if (!window.confirm(`确认删除分组「${group.name}」？组内字段会移到「未分组」。`)) return;
    try {
      await deleteFieldGroup(group.id);
      show('分组已删除', 'success');
      await loadAll();
    } catch (err) {
      show(err instanceof Error ? err.message : '删除分组失败', 'error');
    }
  };

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h2 className="page-title">字段别名</h2>
          <p className="page-subtitle">
            全局配置：标准字段按分组归档，别名用于把各来源表格的原始列名自动映射到标准字段。
          </p>
        </div>
        <div className="row-actions">
          <input
            className="search-box"
            type="search"
            placeholder="搜索标准名称或别名"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
          <button className="btn" onClick={openCreateGroup}>
            ＋ 新建分组
          </button>
          <button className="btn btn-primary" onClick={openCreateField}>
            ＋ 新增标准字段
          </button>
        </div>
      </div>

      <div className="group-bar">
        <button
          className={`group-chip ${activeGroup === 'all' ? 'active' : ''}`}
          onClick={() => setActiveGroup('all')}
        >
          全部 <span className="group-count">{countOf('all')}</span>
        </button>
        {groups.map((group) => (
          <button
            key={group.id}
            className={`group-chip ${activeGroup === group.id ? 'active' : ''}`}
            onClick={() => setActiveGroup(group.id)}
          >
            {group.name} <span className="group-count">{countOf(group.id)}</span>
          </button>
        ))}
        <button
          className={`group-chip ${activeGroup === 'ungrouped' ? 'active' : ''}`}
          onClick={() => setActiveGroup('ungrouped')}
        >
          未分组 <span className="group-count">{countOf('ungrouped')}</span>
        </button>
      </div>

      {loading ? (
        <div className="table-state">
          <Spinner />
          <span className="table-state-text">加载中…</span>
        </div>
      ) : searchedCards.length === 0 ? (
        <div className="page-fill">
          <EmptyState text={search ? '没有匹配的标准字段或别名' : '暂无标准字段'} />
        </div>
      ) : (
        <div className="page-body">
          {sections.map(([key, list]) => {
            const group = groups.find((item) => item.id === key);
            return (
              <section key={key || 'ungrouped'} className="alias-group-section">
                <div className="alias-group-head">
                  <span className="alias-group-name">{group ? group.name : '未分组'}</span>
                  <span className="alias-group-size">{list.length} 个字段</span>
                  {group ? (
                    <div className="row-actions">
                      <button className="icon-btn" onClick={() => openRenameGroup(group)}>
                        重命名
                      </button>
                      <button className="icon-btn danger" onClick={() => removeGroup(group)}>
                        删除分组
                      </button>
                    </div>
                  ) : null}
                </div>

                {list.length === 0 ? (
                  <p className="panel-hint">该分组暂无字段，可在「新增标准字段」时选择此分组。</p>
                ) : (
                  <div className="alias-grid">
                    {list.map((card) => (
                      <div key={card.standard} className="alias-card">
                        <div className="alias-card-head">
                          <div className="alias-card-info">
                            <div className="alias-card-name">
                              {card.standard}
                              {card.field ? (
                                <span
                                  className={`alias-dtype ${
                                    normalizeDtype(card.field.dtype) === 'integer' ? 'int' : ''
                                  }`}
                                >
                                  {dtypeText(card.field)}
                                </span>
                              ) : (
                                <span className="alias-dtype warn">未登记</span>
                              )}
                            </div>
                            {card.field && card.field.label && card.field.label !== card.standard ? (
                              <div className="alias-card-sub">{card.field.label}</div>
                            ) : null}
                          </div>
                          {card.field ? (
                            <div className="alias-card-actions">
                              <button
                                className="icon-btn"
                                title="编辑标准字段"
                                onClick={() => openEditField(card.field!)}
                              >
                                编辑
                              </button>
                              <button
                                className="icon-btn danger"
                                title="删除标准字段"
                                onClick={() => removeField(card)}
                              >
                                删除
                              </button>
                            </div>
                          ) : null}
                        </div>

                        <div className="alias-chips">
                          {card.aliases.length === 0 ? (
                            <span className="alias-empty">暂无别名</span>
                          ) : (
                            card.aliases.map((alias) => (
                              <span key={alias.id} className="alias-chip">
                                {alias.alias}
                                <button
                                  className="alias-chip-remove"
                                  title="删除该别名"
                                  onClick={() => removeAlias(alias)}
                                >
                                  ✕
                                </button>
                              </span>
                            ))
                          )}
                        </div>

                        <div className="alias-add">
                          <input
                            type="text"
                            placeholder="添加别名，回车确认"
                            value={drafts[card.standard] ?? ''}
                            onChange={(event) =>
                              setDrafts((prev) => ({ ...prev, [card.standard]: event.target.value }))
                            }
                            onKeyDown={(event) => {
                              if (event.key === 'Enter') addAlias(card.standard);
                            }}
                          />
                          <button
                            className="btn btn-sm"
                            disabled={busy || !(drafts[card.standard] ?? '').trim()}
                            onClick={() => addAlias(card.standard)}
                          >
                            添加
                          </button>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </section>
            );
          })}
        </div>
      )}

      <Modal
        open={modalOpen}
        title={form.id ? '编辑标准字段' : '新增标准字段'}
        onClose={() => setModalOpen(false)}
        width={560}
        footer={
          <>
            <button className="btn" onClick={() => setModalOpen(false)}>
              取消
            </button>
            <button className="btn btn-primary" onClick={submitField} disabled={savingField}>
              {savingField ? '保存中…' : '保存'}
            </button>
          </>
        }
      >
        <Field label="标准名称" htmlFor="sf-name">
          <input
            id="sf-name"
            type="text"
            placeholder="例如：姓名"
            value={form.name}
            onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))}
          />
        </Field>
        <Field label="显示标签" htmlFor="sf-label">
          <input
            id="sf-label"
            type="text"
            placeholder="留空则与标准名称相同"
            value={form.label}
            onChange={(event) => setForm((prev) => ({ ...prev, label: event.target.value }))}
          />
        </Field>

        <div className="form-grid">
          <Field label="数据类型" htmlFor="sf-dtype">
            <select
              id="sf-dtype"
              value={form.dtype}
              onChange={(event) =>
                setForm((prev) => ({ ...prev, dtype: event.target.value as ColumnDtype }))
              }
            >
              {DTYPES.map((dtype) => (
                <option key={dtype} value={dtype}>
                  {DTYPE_LABELS[dtype]}
                </option>
              ))}
            </select>
          </Field>
          <Field label="精度（小数位）" htmlFor="sf-precision">
            <input
              id="sf-precision"
              type="number"
              min={0}
              max={10}
              disabled={!PRECISION_DTYPES.includes(form.dtype)}
              value={form.precision}
              onChange={(event) =>
                setForm((prev) => ({ ...prev, precision: Number(event.target.value) || 0 }))
              }
            />
          </Field>
        </div>
        <p className="field-hint" style={{ marginTop: -6 }}>
          参考 Excel 数据类型：整数不保留小数；小数 / 百分比可单独设置精度。
        </p>

        <Field label="所属分组" htmlFor="sf-group">
          <select
            id="sf-group"
            value={form.groupId}
            onChange={(event) => setForm((prev) => ({ ...prev, groupId: event.target.value }))}
          >
            <option value="">未分组</option>
            {groups.map((group) => (
              <option key={group.id} value={group.id}>
                {group.name}
              </option>
            ))}
          </select>
        </Field>
        <Field label="描述（可选）" htmlFor="sf-desc">
          <input
            id="sf-desc"
            type="text"
            value={form.description}
            onChange={(event) => setForm((prev) => ({ ...prev, description: event.target.value }))}
          />
        </Field>
      </Modal>

      <Modal
        open={groupModalOpen}
        title={groupForm.id ? '重命名分组' : '新建分组'}
        onClose={() => setGroupModalOpen(false)}
        width={420}
        footer={
          <>
            <button className="btn" onClick={() => setGroupModalOpen(false)}>
              取消
            </button>
            <button className="btn btn-primary" onClick={submitGroup} disabled={savingGroup}>
              {savingGroup ? '保存中…' : '保存'}
            </button>
          </>
        }
      >
        <Field label="分组名称" htmlFor="sf-group-name">
          <input
            id="sf-group-name"
            type="text"
            autoFocus
            placeholder="例如：考生信息 / 成绩 / 考务"
            value={groupForm.name}
            onChange={(event) => setGroupForm((prev) => ({ ...prev, name: event.target.value }))}
            onKeyDown={(event) => {
              if (event.key === 'Enter') submitGroup();
            }}
          />
        </Field>
      </Modal>
    </div>
  );
}
