import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { DragEvent as ReactDragEvent } from 'react';
import { createPortal } from 'react-dom';
import { Button, Input, Modal } from 'antd';
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

interface AliasCard {
  standard: string;
  aliases: AliasMapping[];
  field: StandardField | null;
}

interface NewFieldDraft {
  name: string;
  dtype: ColumnDtype;
  precision: number;
  groupId: string;
}

const EMPTY_DRAFT: NewFieldDraft = {
  name: '',
  dtype: 'text',
  precision: 2,
  groupId: '',
};

export default function AliasPage() {
  const { show } = useToast();

  const [fields, setFields] = useState<StandardField[]>([]);
  const [aliases, setAliases] = useState<AliasMapping[]>([]);
  const [groups, setGroups] = useState<FieldGroup[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState('');
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [nameDrafts, setNameDrafts] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);

  const [newField, setNewField] = useState<NewFieldDraft | null>(null);

  const [groupModalOpen, setGroupModalOpen] = useState(false);
  const [groupForm, setGroupForm] = useState<{ id: string | null; name: string }>({
    id: null,
    name: '',
  });
  const [savingGroup, setSavingGroup] = useState(false);

  // 拖拽分组
  const [dragStandard, setDragStandard] = useState<string | null>(null);
  const [dropGroup, setDropGroup] = useState<string | null>(null);

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
        card.aliases.some((alias) => alias.alias.toLowerCase().includes(query)),
    );
  }, [cards, search]);

  const groupKeyOf = (card: AliasCard): string => card.field?.group_id || '';

  /** 始终展示所有分组（含未分组、空分组），方便把卡片直接拖进目标分组。 */
  const sections = useMemo(() => {
    const buckets = new Map<string, AliasCard[]>();
    groups.forEach((group) => buckets.set(group.id, []));
    buckets.set('', []);
    for (const card of searchedCards) {
      const key = buckets.has(groupKeyOf(card)) ? groupKeyOf(card) : '';
      buckets.get(key)!.push(card);
    }
    return [...buckets.entries()];
  }, [searchedCards, groups]);

  // ------------------------- 字段：保存与拖拽 -------------------------

  const saveField = async (field: StandardField, patch: Partial<StandardField>) => {
    const next = { ...field, ...patch };
    setFields((prev) => prev.map((item) => (item.id === field.id ? next : item)));
    try {
      const saved = await updateStandardField(field.id, {
        name: next.name,
        label: next.name,
        dtype: next.dtype,
        precision: PRECISION_DTYPES.includes(normalizeDtype(next.dtype)) ? next.precision : 0,
        group_id: next.group_id,
        description: next.description,
      });
      setFields((prev) => prev.map((item) => (item.id === saved.id ? saved : item)));
    } catch (err) {
      setFields((prev) => prev.map((item) => (item.id === field.id ? field : item)));
      show(err instanceof Error ? err.message : '保存字段失败', 'error');
    }
  };

  const commitName = (field: StandardField) => {
    const raw = nameDrafts[field.id];
    if (raw === undefined) return;
    const value = raw.trim();
    setNameDrafts((prev) => {
      const next = { ...prev };
      delete next[field.id];
      return next;
    });
    if (!value || value === field.name) return;
    saveField(field, { name: value });
  };

  const moveFieldToGroup = async (standard: string, groupId: string) => {
    const card = cards.find((item) => item.standard === standard);
    if (!card?.field || groupKeyOf(card) === groupId) return;
    await saveField(card.field, { group_id: groupId });
    show(`已移动到「${groupName(groupId)}」`, 'success');
  };

  const groupName = (id: string): string =>
    id ? groups.find((group) => group.id === id)?.name ?? '未分组' : '未分组';

  const handleDragStart = (event: ReactDragEvent, card: AliasCard) => {
    if (!card.field) return;
    event.dataTransfer.effectAllowed = 'move';
    event.dataTransfer.setData('text/plain', card.standard);
    setDragStandard(card.standard);
  };

  const handleDrop = (event: ReactDragEvent, key: string) => {
    event.preventDefault();
    const standard = event.dataTransfer.getData('text/plain') || dragStandard;
    setDropGroup(null);
    setDragStandard(null);
    if (standard) moveFieldToGroup(standard, key);
  };

  // ------------------------- 别名 -------------------------

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

  // ------------------------- 新增字段（卡片内联） -------------------------

  const startNewField = () => {
    setNewField({ ...EMPTY_DRAFT });
  };

  const submitNewField = async () => {
    if (!newField) return;
    const name = newField.name.trim();
    if (!name) {
      show('请填写标准字段名称', 'error');
      return;
    }
    setBusy(true);
    try {
      const created = await createStandardField({
        name,
        label: name,
        dtype: newField.dtype,
        precision: PRECISION_DTYPES.includes(newField.dtype) ? newField.precision : 0,
        group_id: newField.groupId,
        description: '',
      });
      setFields((prev) => [...prev, created]);
      setNewField(null);
      show(`已创建标准字段「${name}」`, 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '创建失败', 'error');
    } finally {
      setBusy(false);
    }
  };

  const removeField = async (card: AliasCard) => {
    if (!card.field) return;
    if (!window.confirm(`确认删除标准字段「${card.standard}」？（其别名卡片会一并移除）`)) {
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

  const groupOptions = useMemo(
    () => [
      { value: '', label: '未分组' },
      ...groups.map((group) => ({ value: group.id, label: group.name })),
    ],
    [groups],
  );

  const dtypeOptions = DTYPES.map((dtype) => ({ value: dtype, label: DTYPE_LABELS[dtype] }));

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h2 className="page-title">字段别名</h2>
          <p className="page-subtitle">
            全局配置：卡片可直接拖动到其它分组；点击卡片上的数据类型或分组即可修改。
          </p>
        </div>
        <div className="row-actions">
          <Input.Search
            allowClear
            placeholder="搜索标准名称或别名"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            style={{ width: 240 }}
          />
          <Button onClick={openCreateGroup}>＋ 新建分组</Button>
          <Button type="primary" onClick={startNewField}>
            ＋ 新增标准字段
          </Button>
        </div>
      </div>

      <div className="group-bar">
        <span className="group-bar-label">分组</span>
        {sections.map(([key, list]) => (
          <span key={key || 'ungrouped'} className="group-chip static">
            {key ? groupName(key) : '未分组'} <span className="group-count">{list.length}</span>
          </span>
        ))}
        <span className="group-bar-hint">拖动卡片到目标分组即可归档</span>
      </div>

      {loading ? (
        <div className="table-state">
          <Spinner />
          <span className="table-state-text">加载中…</span>
        </div>
      ) : searchedCards.length === 0 && !newField ? (
        <div className="page-fill">
          <EmptyState text={search ? '没有匹配的标准字段或别名' : '暂无标准字段'} />
        </div>
      ) : (
        <div className="page-body">
          {sections.map(([key, list]) => {
            const group = groups.find((item) => item.id === key);
            return (
              <section
                key={key || 'ungrouped'}
                className={`alias-group-section ${dropGroup === key ? 'drop-active' : ''}`}
                onDragOver={(event) => {
                  event.preventDefault();
                  event.dataTransfer.dropEffect = 'move';
                  if (dropGroup !== key) setDropGroup(key);
                }}
                onDragLeave={(event) => {
                  if (!event.currentTarget.contains(event.relatedTarget as Node)) {
                    setDropGroup((prev) => (prev === key ? null : prev));
                  }
                }}
                onDrop={(event) => handleDrop(event, key)}
              >
                <div className="alias-group-head">
                  <span className="alias-group-name">{group ? group.name : '未分组'}</span>
                  <span className="alias-group-size">{list.length} 个字段</span>
                  <div className="row-actions">
                    {group ? (
                      <>
                        <button className="icon-btn" onClick={() => openRenameGroup(group)}>
                          重命名
                        </button>
                        <button className="icon-btn danger" onClick={() => removeGroup(group)}>
                          删除分组
                        </button>
                      </>
                    ) : null}
                    <button className="icon-btn" onClick={() => setNewField({ ...EMPTY_DRAFT, groupId: key })}>
                      ＋ 添加字段
                    </button>
                  </div>
                </div>

                <div className="alias-grid">
                  {newField && newField.groupId === key ? (
                    <div className="alias-card alias-card-draft">
                      <div className="alias-card-top">
                        <Input
                          autoFocus
                          placeholder="标准字段名称"
                          value={newField.name}
                          onChange={(event) =>
                            setNewField((prev) => (prev ? { ...prev, name: event.target.value } : prev))
                          }
                          onPressEnter={submitNewField}
                        />
                      </div>
                      <div className="alias-card-meta">
                        <InlineSelect
                          value={newField.dtype}
                          options={dtypeOptions}
                          onChange={(value) =>
                            setNewField((prev) => (prev ? { ...prev, dtype: value as ColumnDtype } : prev))
                          }
                        />
                        {PRECISION_DTYPES.includes(newField.dtype) ? (
                          <label className="alias-precision">
                            小数位
                            <input
                              type="number"
                              min={0}
                              max={10}
                              value={newField.precision}
                              onChange={(event) =>
                                setNewField((prev) =>
                                  prev ? { ...prev, precision: Number(event.target.value) || 0 } : prev,
                                )
                              }
                            />
                          </label>
                        ) : null}
                        <InlineSelect
                          value={newField.groupId}
                          options={groupOptions}
                          onChange={(value) =>
                            setNewField((prev) => (prev ? { ...prev, groupId: value } : prev))
                          }
                        />
                      </div>
                      <div className="row-actions">
                        <button className="btn btn-sm btn-primary" onClick={submitNewField} disabled={busy}>
                          创建
                        </button>
                        <button className="btn btn-sm" onClick={() => setNewField(null)}>
                          取消
                        </button>
                      </div>
                    </div>
                  ) : null}

                  {list.map((card) => (
                    <div
                      key={card.standard}
                      className={`alias-card ${dragStandard === card.standard ? 'dragging' : ''}`}
                      draggable={Boolean(card.field)}
                      onDragStart={(event) => handleDragStart(event, card)}
                      onDragEnd={() => {
                        setDragStandard(null);
                        setDropGroup(null);
                      }}
                    >
                      <div className="alias-card-top">
                        <span className="alias-drag" title="拖动到其它分组">
                          ⠿
                        </span>
                        {card.field ? (
                          <input
                            className="alias-name-input"
                            value={nameDrafts[card.field.id] ?? card.field.name}
                            onChange={(event) =>
                              setNameDrafts((prev) => ({ ...prev, [card.field!.id]: event.target.value }))
                            }
                            onBlur={() => commitName(card.field!)}
                            onKeyDown={(event) => {
                              if (event.key === 'Enter') event.currentTarget.blur();
                              if (event.key === 'Escape') {
                                setNameDrafts((prev) => {
                                  const next = { ...prev };
                                  delete next[card.field!.id];
                                  return next;
                                });
                              }
                            }}
                          />
                        ) : (
                          <span className="alias-card-name">{card.standard}</span>
                        )}
                        {card.field ? (
                          <button
                            className="icon-btn danger"
                            title="删除标准字段"
                            onClick={() => removeField(card)}
                          >
                            删除
                          </button>
                        ) : null}
                      </div>

                      {card.field ? (
                        <div className="alias-card-meta">
                          <InlineSelect
                            value={normalizeDtype(card.field.dtype)}
                            options={dtypeOptions}
                            onChange={(value) => saveField(card.field!, { dtype: value as ColumnDtype })}
                          />
                          {PRECISION_DTYPES.includes(normalizeDtype(card.field.dtype)) ? (
                            <label className="alias-precision">
                              小数位
                              <input
                                type="number"
                                min={0}
                                max={10}
                                value={card.field.precision ?? 2}
                                onChange={(event) =>
                                  setFields((prev) =>
                                    prev.map((item) =>
                                      item.id === card.field!.id
                                        ? { ...item, precision: Number(event.target.value) || 0 }
                                        : item,
                                    ),
                                  )
                                }
                                onBlur={() => saveField(card.field!, { precision: card.field!.precision })}
                              />
                            </label>
                          ) : null}
                          <InlineSelect
                            value={card.field.group_id ?? ''}
                            options={groupOptions}
                            onChange={(value) => saveField(card.field!, { group_id: value })}
                          />
                        </div>
                      ) : (
                        <div className="alias-card-meta">
                          <span className="alias-dtype warn">未登记到标准字段</span>
                        </div>
                      )}

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

                  {list.length === 0 && !(newField && newField.groupId === key) ? (
                    <p className="alias-drop-hint">拖动字段卡片到此处，或点击「添加字段」</p>
                  ) : null}
                </div>
              </section>
            );
          })}
        </div>
      )}

      <Modal
        open={groupModalOpen}
        title={groupForm.id ? '重命名分组' : '新建分组'}
        onCancel={() => setGroupModalOpen(false)}
        onOk={submitGroup}
        okText="保存"
        cancelText="取消"
        confirmLoading={savingGroup}
        width={420}
        destroyOnHidden
      >
        <div className="dialog-form">
          <label className="form-row">
            <span className="field-label">分组名称</span>
            <Input
              autoFocus
              placeholder="例如：考生信息 / 成绩 / 考务"
              value={groupForm.name}
              onChange={(event) =>
                setGroupForm((prev) => ({ ...prev, name: event.target.value }))
              }
              onPressEnter={submitGroup}
            />
          </label>
        </div>
      </Modal>
    </div>
  );
}

// --------------------------------------------------------------------
// 内联下拉：直接渲染在卡片内，避免弹层被容器裁剪或层级覆盖
// --------------------------------------------------------------------

interface InlineSelectProps {
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
  title?: string;
}

function InlineSelect({ value, options, onChange, title }: InlineSelectProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const popRef = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState({ top: 0, left: 0, width: 148 });

  // 用 fixed 定位渲染到 body，彻底避开滚动容器 / 卡片容器的裁剪
  const place = () => {
    const trigger = triggerRef.current;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const width = Math.max(rect.width, 148);
    const height = Math.min(246, options.length * 29 + 10);
    const roomBelow = window.innerHeight - rect.bottom - 8;
    const up = roomBelow < height && rect.top - 8 > height;
    const left = Math.min(Math.max(rect.left, 8), window.innerWidth - width - 8);
    setPos({
      top: up ? rect.top - height - 5 : rect.bottom + 5,
      left,
      width,
    });
  };

  useLayoutEffect(() => {
    if (!open) return;
    place();
    // 打开时若落点被视口底部截断，翻转向上
  }, [open, options.length]);

  useEffect(() => {
    if (!open) return;
    const onDocClick = (event: MouseEvent) => {
      const target = event.target as Node;
      if (popRef.current?.contains(target) || triggerRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onViewportChange = () => place();
    document.addEventListener('mousedown', onDocClick);
    window.addEventListener('resize', onViewportChange);
    window.addEventListener('scroll', onViewportChange, true);
    return () => {
      document.removeEventListener('mousedown', onDocClick);
      window.removeEventListener('resize', onViewportChange);
      window.removeEventListener('scroll', onViewportChange, true);
    };
  }, [open, options.length]);

  const current = options.find((option) => option.value === value);

  return (
    <div className="alias-select">
      <button
        type="button"
        ref={triggerRef}
        className="alias-select-trigger"
        title={title}
        onClick={() => setOpen((prev) => !prev)}
      >
        <span>{current?.label ?? '未选择'}</span>
        <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4">
          <path d="m6 9 6 6 6-6" />
        </svg>
      </button>
      {open
        ? createPortal(
            <div
              ref={popRef}
              className="alias-select-pop portal"
              role="listbox"
              style={{ top: pos.top, left: pos.left, minWidth: pos.width }}
            >
              {options.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  className={`alias-select-item ${option.value === value ? 'active' : ''}`}
                  onClick={() => {
                    onChange(option.value);
                    setOpen(false);
                  }}
                >
                  {option.label}
                </button>
              ))}
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}
