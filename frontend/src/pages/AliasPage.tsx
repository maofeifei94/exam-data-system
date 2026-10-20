import { useEffect, useMemo, useState } from 'react';
import {
  createStandardField,
  deleteAlias,
  deleteStandardField,
  listAliases,
  listStandardFields,
  saveAliases,
  updateStandardField,
} from '../api/client';
import type { AliasMapping, ColumnDtype, StandardField } from '../api/types';
import { EmptyState } from '../components/EmptyState';
import { Field } from '../components/Field';
import { Modal } from '../components/Modal';
import { Spinner } from '../components/Spinner';
import { useToast } from '../components/Toast';

const DTYPES: ColumnDtype[] = ['text', 'number', 'date', 'image'];
const DTYPE_LABELS: Record<string, string> = {
  text: '文本',
  number: '数值',
  date: '日期',
  image: '图片',
};

interface FieldForm {
  id: string | null;
  name: string;
  label: string;
  dtype: ColumnDtype;
  description: string;
}

const EMPTY_FORM: FieldForm = { id: null, name: '', label: '', dtype: 'text', description: '' };

interface AliasCard {
  standard: string;
  aliases: AliasMapping[];
  field: StandardField | null;
}

export default function AliasPage() {
  const { show } = useToast();

  const [fields, setFields] = useState<StandardField[]>([]);
  const [aliases, setAliases] = useState<AliasMapping[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState('');
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);

  const [modalOpen, setModalOpen] = useState(false);
  const [form, setForm] = useState<FieldForm>(EMPTY_FORM);
  const [savingField, setSavingField] = useState(false);

  const loadAll = async () => {
    setLoading(true);
    try {
      const [fieldList, aliasList] = await Promise.all([listStandardFields(), listAliases()]);
      setFields(fieldList);
      setAliases(aliasList);
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

  // 以“标准名称”为主键，把别名按标准名归组；标准字段即使暂无别名也生成一张卡片
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
    // 存在别名但标准字段未登记的情况，照样展示，方便清理
    for (const [standard, list] of byStandard) {
      if (!consumed.has(standard)) {
        result.push({ standard, aliases: list, field: null });
      }
    }
    return result;
  }, [fields, aliases]);

  const filteredCards = useMemo(() => {
    const query = search.trim().toLowerCase();
    if (!query) return cards;
    return cards.filter(
      (card) =>
        card.standard.toLowerCase().includes(query) ||
        card.field?.label.toLowerCase().includes(query) ||
        card.aliases.some((alias) => alias.alias.toLowerCase().includes(query)),
    );
  }, [cards, search]);

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
    setForm(EMPTY_FORM);
    setModalOpen(true);
  };

  const openEditField = (field: StandardField) => {
    setForm({
      id: field.id,
      name: field.name,
      label: field.label,
      dtype: field.dtype,
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

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h2 className="page-title">字段别名</h2>
          <p className="page-subtitle">
            每张卡片代表一个标准字段（卡片标题即标准名称），其下方以圆角标签列出所有别名。
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
          <button className="btn btn-primary" onClick={openCreateField}>
            ＋ 新增标准字段
          </button>
        </div>
      </div>

      {loading ? (
        <div className="table-state">
          <Spinner />
          <span className="table-state-text">加载中…</span>
        </div>
      ) : filteredCards.length === 0 ? (
        <EmptyState text="暂无标准字段" />
      ) : (
        <div className="alias-grid">
          {filteredCards.map((card) => (
            <div key={card.standard} className="alias-card">
              <div className="alias-card-head">
                <div className="alias-card-info">
                  <div className="alias-card-name">
                    {card.standard}
                    {card.field ? (
                      <span className="alias-dtype">{DTYPE_LABELS[card.field.dtype] ?? card.field.dtype}</span>
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
                    <button className="icon-btn" title="编辑标准字段" onClick={() => openEditField(card.field!)}>
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

      <Modal
        open={modalOpen}
        title={form.id ? '编辑标准字段' : '新增标准字段'}
        onClose={() => setModalOpen(false)}
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
        <Field label="描述（可选）" htmlFor="sf-desc">
          <input
            id="sf-desc"
            type="text"
            value={form.description}
            onChange={(event) => setForm((prev) => ({ ...prev, description: event.target.value }))}
          />
        </Field>
      </Modal>
    </div>
  );
}
