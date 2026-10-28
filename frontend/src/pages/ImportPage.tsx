import { useEffect, useState } from 'react';
import { Button, Input, InputNumber, Select, Steps, Switch } from 'antd';
import {
  applyMapping,
  getDatasetRows,
  importDataset,
  listFlows,
  listProjectFields,
  listStandardFields,
} from '../api/client';
import type { ColumnSuggestion, Dataset, ImportResult, RowPage, StandardField } from '../api/types';
import { DataTable } from '../components/DataTable';
import { EmptyState } from '../components/EmptyState';
import { useToast } from '../components/Toast';
import { useProject } from '../context/ProjectContext';
import { mergeFieldLists } from '../lib/projectFields';
import type { ViewKey } from '../App';

const KEEP_ORIGINAL = '__keep_original__';

const STEP_TITLES = ['选择文件', '数据预览', '字段映射', '完成'];

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
  const [requiredFields, setRequiredFields] = useState<string[]>([]);
  const [importing, setImporting] = useState(false);
  const [result, setResult] = useState<ImportResult | null>(null);
  const [mappings, setMappings] = useState<Record<string, string>>({});
  const [persistAliases, setPersistAliases] = useState(true);

  const [applying, setApplying] = useState(false);
  const [appliedDataset, setAppliedDataset] = useState<Dataset | null>(null);
  const [appliedRows, setAppliedRows] = useState<RowPage | null>(null);

  /** 当前步骤；只有访问过的步骤才能通过顶部步骤条回跳 */
  const [step, setStep] = useState(0);
  const [maxStep, setMaxStep] = useState(0);

  // 数据导入只保留流程「必要字段校验」声明的字段，其余原始列在应用映射时删除。
  useEffect(() => {
    let active = true;
    (async () => {
      if (!currentProjectId) {
        if (active) setRequiredFields([]);
        return;
      }
      try {
        const flows = await listFlows({ projectId: currentProjectId });
        const names: string[] = [];
        for (const flow of flows) {
          for (const step of flow.steps) {
            if (!step.enabled || step.operator !== 'require_fields') continue;
            const fields = Array.isArray(step.params?.fields) ? step.params.fields : [];
            for (const field of fields) {
              const name = String(field).trim();
              if (name && !names.includes(name)) names.push(name);
            }
          }
        }
        if (active) setRequiredFields(names);
      } catch {
        if (active) setRequiredFields([]);
      }
    })();
    return () => {
      active = false;
    };
  }, [currentProjectId]);

  const goTo = (next: number) => {
    setMaxStep((prev) => Math.max(prev, next));
    setStep(next);
  };

  const reset = () => {
    setFile(null);
    setName('');
    setResult(null);
    setMappings({});
    setAppliedDataset(null);
    setAppliedRows(null);
    setStep(0);
    setMaxStep(0);
  };

  const handleImport = async () => {
    if (!file) {
      show('请先选择文件', 'error');
      return;
    }
    setImporting(true);
    setAppliedDataset(null);
    setAppliedRows(null);
    try {
      const [fields, projectFields, imported] = await Promise.all([
        listStandardFields(),
        listProjectFields(currentProjectId || ''),
        importDataset(file, name || undefined, headerRow, currentProjectId || undefined),
      ]);
      setStandardFields(mergeFieldLists(fields, projectFields));
      setResult(imported);
      const initial: Record<string, string> = {};
      for (const suggestion of imported.mapping) {
        initial[suggestion.source] = suggestion.standard ?? KEEP_ORIGINAL;
      }
      setMappings(initial);
      goTo(1);
      show('导入成功，请预览数据后确认字段映射', 'success');
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
      const dataset = await applyMapping(
        result.dataset.id,
        payload,
        persistAliases,
        requiredFields,
      );
      setAppliedDataset(dataset);
      setAppliedRows(await getDatasetRows(dataset.id, 0, 100));
      goTo(3);
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
        <div>
          <h2 className="page-title">数据导入</h2>
          <p className="page-subtitle">
            {currentProject ? `导入到考试项目「${currentProject.name}」` : '未归属项目，可先在顶部选择考试项目'}
          </p>
        </div>
      </div>

      <div className="import-wizard page-fill">
        <Steps
          className="import-steps"
          size="small"
          current={step}
          onChange={(next) => {
            if (next <= maxStep) setStep(next);
          }}
          items={STEP_TITLES.map((title) => ({ title }))}
        />

        <div className="import-body">
          {step === 0 ? (
            <section className="card">
              <h3 className="card-title">选择要导入的文件</h3>
              <label className="field">
                <span className="field-label">Excel / CSV 文件</span>
                <input
                  type="file"
                  accept=".xlsx,.xls,.csv"
                  onChange={(event) => setFile(event.target.files?.[0] ?? null)}
                />
              </label>
              <label className="field">
                <span className="field-label">数据集名称（可选）</span>
                <Input
                  placeholder="留空则使用文件名"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                />
              </label>
              <label className="field">
                <span className="field-label">表头行号</span>
                <InputNumber
                  min={1}
                  style={{ width: 140 }}
                  value={headerRow}
                  onChange={(value) => setHeaderRow(Number(value) || 1)}
                />
              </label>
              <p className="panel-hint">
                {requiredFields.length > 0
                  ? `应用映射后只保留流程声明的 ${requiredFields.length} 个必要字段，其余原始列与数据会被删除。`
                  : '支持 xlsx / xls / csv。导入后可先预览，再逐列确认字段映射。'}
              </p>
            </section>
          ) : null}

          {step === 1 ? (
            <section className="card card-fill">
              <h3 className="card-title">导入预览（映射前）</h3>
              <div className="card-scroll">
                {result ? (
                  <DataTable
                    columns={result.preview.columns.map((column) => column.name)}
                    rows={result.preview.rows}
                    emptyText="暂无预览数据"
                    maxHeight="100%"
                    stickyHeader
                  />
                ) : (
                  <EmptyState text="还没有导入数据" />
                )}
              </div>
            </section>
          ) : null}

          {step === 2 ? (
            <section className="card card-fill">
              <h3 className="card-title">字段映射</h3>
              <p className="panel-hint">
                为每个源字段选择对应的标准字段。默认「保持原名」，流程只能引用映射后的字段名。
              </p>
              <div className="card-scroll">
                {result ? (
                  <table className="data-table align-center bordered">
                    <thead className="data-table__head sticky">
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
                              <Select
                                style={{ width: '100%', minWidth: 220 }}
                                value={mappings[suggestion.source] ?? KEEP_ORIGINAL}
                                onChange={(value) =>
                                  setMappings((prev) => ({
                                    ...prev,
                                    [suggestion.source]: value,
                                  }))
                                }
                                options={[
                                  {
                                    value: KEEP_ORIGINAL,
                                    label: `保持原名（${suggestion.source}）`,
                                  },
                                  ...standardFields.map((field) => ({
                                    value: field.name,
                                    label: `${field.label}（${field.name}）${
                                      requiredFields.includes(field.name) ? ' · 必要' : ''
                                    }`,
                                  })),
                                ]}
                              />
                            </td>
                            <td>
                              <span className={badge.className}>{badge.text}</span>
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                ) : (
                  <EmptyState text="还没有导入数据" />
                )}
              </div>

              <label className="checkbox-row">
                <Switch
                  size="small"
                  checked={persistAliases}
                  onChange={setPersistAliases}
                  style={{ marginRight: 8 }}
                />
                保存为新别名
              </label>

              {requiredFields.length > 0 ? (
                <div className="required-fields-note">
                  <span className="field-label">仅保留必要字段：</span>
                  <div className="chip-row">
                    {requiredFields.map((field) => (
                      <span key={field} className="token-chip">
                        {field}
                      </span>
                    ))}
                  </div>
                </div>
              ) : null}
            </section>
          ) : null}

          {step === 3 ? (
            <section className="card card-fill">
              <h3 className="card-title">
                {appliedDataset ? `完成 · ${appliedDataset.name}` : '完成'}
              </h3>
              {appliedDataset ? (
                <>
                  <div className="detail-meta">共 {appliedDataset.row_count} 行</div>
                  <div className="card-scroll">
                    <DataTable
                      columns={appliedDataset.columns.map((column) => column.name)}
                      rows={appliedRows?.rows ?? []}
                      emptyText="暂无数据"
                      maxHeight="100%"
                      stickyHeader
                    />
                  </div>
                </>
              ) : (
                <EmptyState text="映射尚未应用" />
              )}
            </section>
          ) : null}
        </div>

        <div className="import-footer">
          {importing ? <span className="footer-hint">正在解析文件并写入数据集…</span> : null}
          {step === 0 ? (
            <>
              <span className="footer-hint">支持 xlsx / xls / csv</span>
              <Button type="primary" loading={importing} disabled={!file} onClick={handleImport}>
                开始导入
              </Button>
            </>
          ) : null}
          {step === 1 ? (
            <>
              <Button onClick={() => setStep(0)}>上一步</Button>
              <Button type="primary" onClick={() => setStep(2)}>
                下一步：字段映射
              </Button>
            </>
          ) : null}
          {step === 2 ? (
            <>
              <Button onClick={() => setStep(1)}>上一步</Button>
              <Button type="primary" loading={applying} onClick={handleApply}>
                应用映射
              </Button>
            </>
          ) : null}
          {step === 3 ? (
            <>
              {onNavigate ? (
                <button className="link" onClick={() => onNavigate('flow')}>
                  前往项目流程 →
                </button>
              ) : null}
              <Button onClick={() => onNavigate?.('files')}>查看项目文件</Button>
              <Button type="primary" onClick={reset}>
                再导入一个文件
              </Button>
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}
