import { useEffect, useState } from 'react';
import {
  deleteDataset,
  deleteProject,
  downloadFile,
  getDatasetRows,
  listDatasets,
  updateProject,
} from '../api/client';
import type { Dataset, RowPage } from '../api/types';
import { DataTable } from '../components/DataTable';
import { EmptyState } from '../components/EmptyState';
import { Field } from '../components/Field';
import { Spinner } from '../components/Spinner';
import { useToast } from '../components/Toast';
import { useProject } from '../context/ProjectContext';
import { isDesktop, pickFolder } from '../lib/native';

const PAGE_SIZE = 50;

interface Props {
  onNewProject: () => void;
  onOpenImport: () => void;
  onOpenFlow: () => void;
}

export default function ProjectFilesPage({ onNewProject, onOpenImport, onOpenFlow }: Props) {
  const { show } = useToast();
  const { currentProject, currentProjectId, patchCurrentProject, refresh } = useProject();

  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [folder, setFolder] = useState('');
  const [saving, setSaving] = useState(false);

  const [datasets, setDatasets] = useState<Dataset[]>([]);
  const [loadingDatasets, setLoadingDatasets] = useState(false);
  const [selected, setSelected] = useState<Dataset | null>(null);
  const [rows, setRows] = useState<RowPage | null>(null);
  const [rowsLoading, setRowsLoading] = useState(false);
  const [offset, setOffset] = useState(0);

  // 当前项目变化时同步表单
  useEffect(() => {
    setName(currentProject?.name ?? '');
    setDescription(currentProject?.description ?? '');
    setFolder(currentProject?.folder ?? '');
  }, [currentProject]);

  const loadDatasets = async () => {
    if (!currentProjectId) {
      setDatasets([]);
      setSelected(null);
      setRows(null);
      return;
    }
    setLoadingDatasets(true);
    try {
      setDatasets(await listDatasets(currentProjectId));
    } catch (err) {
      show(err instanceof Error ? err.message : '加载数据集失败', 'error');
    } finally {
      setLoadingDatasets(false);
    }
  };

  useEffect(() => {
    setSelected(null);
    setRows(null);
    loadDatasets();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentProjectId]);

  const loadRows = async (dataset: Dataset, nextOffset: number) => {
    setRowsLoading(true);
    try {
      setRows(await getDatasetRows(dataset.id, nextOffset, PAGE_SIZE));
      setOffset(nextOffset);
    } catch (err) {
      show(err instanceof Error ? err.message : '加载数据失败', 'error');
    } finally {
      setRowsLoading(false);
    }
  };

  const openDataset = (dataset: Dataset) => {
    setSelected(dataset);
    setRows(null);
    setOffset(0);
    loadRows(dataset, 0);
  };

  const browseFolder = async () => {
    if (!isDesktop()) {
      show('浏览器预览环境无法调起系统文件夹窗口，请手动输入路径', 'error');
      return;
    }
    const picked = await pickFolder(folder || undefined);
    if (picked) setFolder(picked);
  };

  const saveProject = async () => {
    if (!currentProject) return;
    if (!name.trim()) {
      show('项目名称不能为空', 'error');
      return;
    }
    setSaving(true);
    try {
      const updated = await updateProject(currentProject.id, {
        name: name.trim(),
        description: description.trim(),
        folder: folder.trim(),
        flow_id: currentProject.flow_id,
        pdf_template_id: currentProject.pdf_template_id,
        dataset_id: currentProject.dataset_id,
      });
      patchCurrentProject(updated);
      await refresh();
      show('项目信息已保存', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '保存失败', 'error');
    } finally {
      setSaving(false);
    }
  };

  const removeDataset = async (dataset: Dataset) => {
    if (!window.confirm(`确认删除数据集「${dataset.name}」？`)) return;
    try {
      await deleteDataset(dataset.id);
      if (selected?.id === dataset.id) {
        setSelected(null);
        setRows(null);
      }
      await loadDatasets();
      show('数据集已删除', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '删除失败', 'error');
    }
  };

  const exportDataset = async (dataset: Dataset) => {
    try {
      await downloadFile(`/api/datasets/${dataset.id}/export?format=csv`, null, `${dataset.name}.csv`, 'GET');
    } catch (err) {
      show(err instanceof Error ? err.message : '导出失败', 'error');
    }
  };

  const removeProject = async () => {
    if (!currentProject) return;
    if (!window.confirm(`确认删除项目「${currentProject.name}」？其数据集将变为未归属。`)) return;
    try {
      await deleteProject(currentProject.id);
      await refresh();
      show('项目已删除', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '删除失败', 'error');
    }
  };

  if (!currentProject) {
    return (
      <div className="page">
        <div className="page-header">
          <div>
            <h2 className="page-title">项目文件</h2>
            <p className="page-subtitle">项目的信息、数据集与处理配置都集中在这里。</p>
          </div>
        </div>
        <div className="page-fill">
          <EmptyState text="还没有打开任何考试项目" />
          <div className="row-actions" style={{ justifyContent: 'center' }}>
            <button className="btn btn-primary" onClick={onNewProject}>
              ＋ 新建考试项目
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h2 className="page-title">{currentProject.name}</h2>
          <p className="page-subtitle">
            {currentProject.description || '项目信息、数据集与处理流程配置'}
          </p>
        </div>
        <div className="row-actions">
          <button className="btn" onClick={onOpenFlow}>
            打开项目流程
          </button>
          <button className="btn btn-primary" onClick={onOpenImport}>
            ＋ 导入数据
          </button>
        </div>
      </div>

      <div className="split-2 page-fill">
        {/* ---------------- 左：项目信息 ---------------- */}
        <div className="column">
          <section className="card card-fill">
            <div className="card-head-row">
              <h3 className="card-title">项目信息</h3>
              <button className="btn btn-sm btn-primary" onClick={saveProject} disabled={saving}>
                {saving ? '保存中…' : '保存'}
              </button>
            </div>
            <div className="card-scroll">
              <Field label="项目名称" htmlFor="pf-name">
                <input
                  id="pf-name"
                  type="text"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                />
              </Field>
              <Field label="说明" htmlFor="pf-desc">
                <input
                  id="pf-desc"
                  type="text"
                  value={description}
                  onChange={(event) => setDescription(event.target.value)}
                />
              </Field>
              <Field label="项目文件夹" htmlFor="pf-folder">
                <div className="browse-row">
                  <input
                    id="pf-folder"
                    type="text"
                    value={folder}
                    placeholder="未指定"
                    onChange={(event) => setFolder(event.target.value)}
                  />
                  <button className="btn" onClick={browseFolder}>
                    浏览…
                  </button>
                </div>
              </Field>
              <div className="info-grid">
                <span className="info-label">配置文件</span>
                <span className="info-value mono" title={currentProject.config_file}>
                  {currentProject.config_file || '尚未写入'}
                </span>
              </div>
            </div>
          </section>

          <section className="card">
            <h3 className="card-title">处理配置</h3>
            <div className="info-grid">
              <span className="info-label">处理流程</span>
              <span className="info-value">
                {currentProject.flow_id ? '已绑定' : '未绑定（在项目流程中保存后自动绑定）'}
              </span>
              <span className="info-label">PDF 模板</span>
              <span className="info-value">
                {currentProject.pdf_template_id ? '已绑定' : '未绑定（在 PDF 输出节点中选择）'}
              </span>
              <span className="info-label">数据导入</span>
              <span className="info-value">
                {currentProject.dataset_id
                  ? datasets.find((d) => d.id === currentProject.dataset_id)?.name ?? '已绑定数据集'
                  : '未绑定（在数据导入节点中选择）'}
              </span>
            </div>
            <div className="link-row">
              <button className="link" onClick={onOpenFlow}>
                前往项目流程 →
              </button>
              <button className="link" onClick={removeProject}>
                删除项目
              </button>
            </div>
          </section>
        </div>

        {/* ---------------- 右：数据集 ---------------- */}
        <div className="column">
          <section className="card card-fill" style={{ flex: '0 0 44%' }}>
            <div className="card-head-row">
              <h3 className="card-title">
                数据集 <span className="group-count">{datasets.length}</span>
              </h3>
              <div className="row-actions">
                <button className="btn btn-sm" onClick={loadDatasets}>
                  刷新
                </button>
                <button className="btn btn-sm btn-primary" onClick={onOpenImport}>
                  导入
                </button>
              </div>
            </div>
            <div className="card-scroll">
              {loadingDatasets ? (
                <div className="table-state">
                  <Spinner />
                  <span className="table-state-text">加载中…</span>
                </div>
              ) : datasets.length === 0 ? (
                <EmptyState text="暂无数据集，点击右上角导入" />
              ) : (
                <ul className="file-list">
                  {datasets.map((dataset) => (
                    <li
                      key={dataset.id}
                      className={`file-item ${selected?.id === dataset.id ? 'active' : ''}`}
                      onClick={() => openDataset(dataset)}
                    >
                      <div className="file-item-main">
                        <div className="file-item-name">{dataset.name}</div>
                        <div className="file-item-meta">
                          {dataset.row_count} 行 · {dataset.columns.length} 列 · {dataset.source_file}
                        </div>
                      </div>
                      <div className="row-actions" onClick={(event) => event.stopPropagation()}>
                        <button className="icon-btn" title="导出 CSV" onClick={() => exportDataset(dataset)}>
                          导出
                        </button>
                        <button
                          className="icon-btn danger"
                          title="删除数据集"
                          onClick={() => removeDataset(dataset)}
                        >
                          删除
                        </button>
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </section>

          <section className="card card-fill">
            {selected ? (
              <>
                <div className="card-head-row">
                  <div>
                    <h3 className="card-title" style={{ margin: 0 }}>
                      {selected.name}
                    </h3>
                    <div className="detail-meta">
                      共 {rows?.total ?? selected.row_count} 行 · {selected.columns.length} 列
                    </div>
                  </div>
                  <div className="row-actions">
                    <button
                      className="btn btn-sm"
                      disabled={offset <= 0 || rowsLoading}
                      onClick={() => loadRows(selected, Math.max(offset - PAGE_SIZE, 0))}
                    >
                      上一页
                    </button>
                    <span className="detail-meta">
                      {Math.floor(offset / PAGE_SIZE) + 1} /{' '}
                      {Math.max(1, Math.ceil((rows?.total ?? 0) / PAGE_SIZE))}
                    </span>
                    <button
                      className="btn btn-sm"
                      disabled={rowsLoading || (rows ? offset + PAGE_SIZE >= rows.total : true)}
                      onClick={() => loadRows(selected, offset + PAGE_SIZE)}
                    >
                      下一页
                    </button>
                  </div>
                </div>
                <div className="card-scroll">
                  {rows ? (
                    <DataTable
                      columns={rows.columns.map((column) => column.name)}
                      rows={rows.rows}
                      loading={rowsLoading}
                      emptyText="暂无数据"
                      maxHeight="100%"
                      stickyHeader
                    />
                  ) : (
                    <div className="table-state">
                      <Spinner />
                      <span className="table-state-text">加载中…</span>
                    </div>
                  )}
                </div>
              </>
            ) : (
              <EmptyState text="选择上方数据集查看数据预览" />
            )}
          </section>
        </div>
      </div>
    </div>
  );
}
