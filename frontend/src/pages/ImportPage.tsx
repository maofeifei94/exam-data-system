import { useState } from 'react';
import {
  applyMapping,
  getDatasetRows,
  importDataset,
  listStandardFields,
} from '../api/client';
import type { ColumnSuggestion, Dataset, ImportResult, RowPage, StandardField } from '../api/types';
import { DataTable } from '../components/DataTable';
import { Field } from '../components/Field';
import { Spinner } from '../components/Spinner';
import { useToast } from '../components/Toast';
import { useProject } from '../context/ProjectContext';
import type { ViewKey } from '../App';

const KEEP_ORIGINAL = '__keep_original__';

function matchedBadge(suggestion: ColumnSuggestion): { className: string; text: string } {
  switch (suggestion.matched_by) {
    case 'alias':
      return { className: 'badge badge-alias', text: '别名匹配' };
    case 'exact_standard':
      return { className: 'badge badge-standard', text: '标准名' };
    default:
      return { className: 'badge badge-none', text: '未匹配' };
  }
}

interface Props {
  onNavigate?: (view: ViewKey) => void;
  /** 映射应用完成后的回调（在项目流程的「数据导入」节点内嵌使用时，用于自动绑定数据集） */
  onImported?: (dataset: Dataset) => void;
}

export default function ImportPage({ onNavigate, onImported }: Props) {
  const { show } = useToast();
  const { currentProject, currentProjectId } = useProject();
  const [file, setFile] = useState<File | null>(null);
  const [name, setName] = useState('');
  const [headerRow, setHeaderRow] = useState(1);

  const [standardFields, setStandardFields] = useState<StandardField[]>([]);
  const [importing, setImporting] = useState(false);
  const [result, setResult] = useState<ImportResult | null>(null);
  const [mappings, setMappings] = useState<Record<string, string>>({});
  const [persistAliases, setPersistAliases] = useState(true);

  const [applying, setApplying] = useState(false);
  const [appliedDataset, setAppliedDataset] = useState<Dataset | null>(null);
  const [appliedRows, setAppliedRows] = useState<RowPage | null>(null);

  const handleImport = async () => {
    if (!file) {
      show('请先选择文件', 'error');
      return;
    }
    setImporting(true);
    setAppliedDataset(null);
    setAppliedRows(null);
    try {
      const [fields, imported] = await Promise.all([
        listStandardFields(),
        importDataset(file, name || undefined, headerRow, currentProjectId || undefined),
      ]);
      setStandardFields(fields);
      setResult(imported);
      const initial: Record<string, string> = {};
      for (const suggestion of imported.mapping) {
        initial[suggestion.source] = suggestion.standard ?? KEEP_ORIGINAL;
      }
      setMappings(initial);
      show('导入成功，请确认字段映射', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '导入失败', 'error');
    } finally {
      setImporting(false);
    }
  };

  const handleApply = async () => {
    if (!result) {
      return;
    }
    setApplying(true);
    try {
      const payload = Object.entries(mappings)
        .filter(([, to]) => to !== KEEP_ORIGINAL)
        .map(([from, to]) => ({ from, to }));
      const dataset = await applyMapping(result.dataset.id, payload, persistAliases);
      setAppliedDataset(dataset);
      setAppliedRows(await getDatasetRows(dataset.id, 0, 100));
      show('映射已应用', 'success');
      onImported?.(dataset);
    } catch (err) {
      show(err instanceof Error ? err.message : '应用映射失败', 'error');
    } finally {
      setApplying(false);
    }
  };

  return (
    <div className="page">
      <div className="page-header">
        <h2 className="page-title">数据导入</h2>
      </div>

      <section className="card">
        <h3 className="card-title">第一步 · 选择文件</h3>
        <Field label="Excel / CSV 文件" htmlFor="import-file">
          <input
            id="import-file"
            type="file"
            accept=".xlsx,.xls,.csv"
            onChange={(event) => setFile(event.target.files?.[0] ?? null)}
          />
        </Field>
        <Field label="数据集名称（可选）" htmlFor="import-name">
          <input
            id="import-name"
            type="text"
            placeholder="留空则使用文件名"
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        </Field>
        <Field label="表头行号" htmlFor="import-header-row">
          <input
            id="import-header-row"
            type="number"
            min={1}
            value={headerRow}
            onChange={(event) => setHeaderRow(Number(event.target.value) || 1)}
          />
        </Field>
        <p className="panel-hint">
          导入的数据集将归属：
          {currentProject ? `考试项目「${currentProject.name}」` : '未归属（可在顶部先选择考试项目）'}
        </p>
        <button className="btn btn-primary" onClick={handleImport} disabled={importing}>
          {importing ? '导入中…' : '开始导入'}
        </button>
      </section>

      {importing ? (
        <div className="table-state">
          <Spinner />
          <span className="table-state-text">导入中…</span>
        </div>
      ) : null}

      {result ? (
        <>
          <section className="card">
            <h3 className="card-title">导入预览（映射前）</h3>
            <DataTable
              columns={result.preview.columns.map((column) => column.name)}
              rows={result.preview.rows}
              emptyText="暂无预览数据"
              maxHeight="320px"
              stickyHeader
            />
          </section>

          <section className="card">
            <h3 className="card-title">第三步 · 字段映射</h3>
            <table className="data-table">
              <thead className="data-table__head">
                <tr>
                  <th>源字段</th>
                  <th>标准字段</th>
                  <th>匹配方式</th>
                </tr>
              </thead>
              <tbody>
                {result.mapping.map((suggestion, index) => {
                  const badge = matchedBadge(suggestion);
                  return (
                    <tr key={suggestion.source} className={index % 2 === 1 ? 'even' : undefined}>
                      <td>{suggestion.source}</td>
                      <td>
                        <select
                          value={mappings[suggestion.source] ?? KEEP_ORIGINAL}
                          onChange={(event) =>
                            setMappings((prev) => ({
                              ...prev,
                              [suggestion.source]: event.target.value,
                            }))
                          }
                        >
                          <option value={KEEP_ORIGINAL}>
                            保持原名（{suggestion.source}）
                          </option>
                          {standardFields.map((field) => (
                            <option key={field.id} value={field.name}>
                              {field.label}（{field.name}）
                            </option>
                          ))}
                        </select>
                      </td>
                      <td>
                        <span className={badge.className}>{badge.text}</span>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>

            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={persistAliases}
                onChange={(event) => setPersistAliases(event.target.checked)}
              />
              保存为新别名
            </label>

            <button className="btn btn-primary" onClick={handleApply} disabled={applying}>
              {applying ? '应用中…' : '应用映射'}
            </button>
          </section>
        </>
      ) : null}

      {appliedDataset ? (
        <section className="card">
          <h3 className="card-title">映射结果 · {appliedDataset.name}</h3>
          <div className="detail-meta">共 {appliedDataset.row_count} 行</div>
          <DataTable
            columns={appliedDataset.columns.map((column) => column.name)}
            rows={appliedRows?.rows ?? []}
            emptyText="暂无数据"
            maxHeight="320px"
            stickyHeader
          />
          <div className="link-row">
            {onNavigate ? (
              <button className="link" onClick={() => onNavigate('flow')}>
                前往项目流程 →
              </button>
            ) : null}
          </div>
        </section>
      ) : null}
    </div>
  );
}
