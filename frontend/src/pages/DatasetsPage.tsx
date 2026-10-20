import { useEffect, useState } from 'react';
import {
  deleteDataset,
  downloadFile,
  getDatasetRows,
  listDatasets,
} from '../api/client';
import type { Dataset, RowPage } from '../api/types';
import { DataTable } from '../components/DataTable';
import { EmptyState } from '../components/EmptyState';
import { Spinner } from '../components/Spinner';
import { useToast } from '../components/Toast';
import { useProject } from '../context/ProjectContext';

const PAGE_SIZE = 100;

export default function DatasetsPage() {
  const { show } = useToast();
  const { currentProjectId } = useProject();
  const [datasets, setDatasets] = useState<Dataset[]>([]);
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<Dataset | null>(null);
  const [rows, setRows] = useState<RowPage | null>(null);
  const [rowsLoading, setRowsLoading] = useState(false);
  const [offset, setOffset] = useState(0);

  const loadDatasets = async () => {
    setLoading(true);
    try {
      setDatasets(await listDatasets(currentProjectId || undefined));
    } catch (err) {
      show(err instanceof Error ? err.message : '加载数据集失败', 'error');
    } finally {
      setLoading(false);
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

  const openDetail = (dataset: Dataset) => {
    setSelected(dataset);
    setRows(null);
    setOffset(0);
    loadRows(dataset, 0);
  };

  const handleDelete = async (dataset: Dataset) => {
    if (!window.confirm(`确认删除数据集「${dataset.name}」？`)) {
      return;
    }
    try {
      await deleteDataset(dataset.id);
      if (selected?.id === dataset.id) {
        setSelected(null);
        setRows(null);
      }
      show('删除成功', 'success');
      await loadDatasets();
    } catch (err) {
      show(err instanceof Error ? err.message : '删除失败', 'error');
    }
  };

  const handleExport = async (dataset: Dataset) => {
    try {
      await downloadFile(`/api/datasets/${dataset.id}/export?format=csv`, null, `${dataset.name}.csv`, 'GET');
    } catch (err) {
      show(err instanceof Error ? err.message : '导出失败', 'error');
    }
  };

  if (loading) {
    return (
      <div className="page">
        <div className="table-state">
          <Spinner />
          <span className="table-state-text">加载中…</span>
        </div>
      </div>
    );
  }

  return (
    <div className="page">
      <div className="page-header">
        <h2 className="page-title">数据集</h2>
        <button className="btn" onClick={loadDatasets}>
          刷新
        </button>
      </div>

      {datasets.length === 0 ? (
        <EmptyState text="暂无数据集，请前往「数据导入」创建" />
      ) : (
        <table className="data-table">
          <thead className="data-table__head sticky">
            <tr>
              <th>名称</th>
              <th className="num">行数</th>
              <th className="num">列数</th>
              <th>来源文件</th>
              <th>创建时间</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {datasets.map((dataset, index) => (
              <tr
                key={dataset.id}
                className={index % 2 === 1 ? 'even' : undefined}
                onClick={() => openDetail(dataset)}
              >
                <td>{dataset.name}</td>
                <td className="num mono">{dataset.row_count}</td>
                <td className="num mono">{dataset.columns.length}</td>
                <td>{dataset.source_file}</td>
                <td>{dataset.created_at}</td>
                <td onClick={(event) => event.stopPropagation()}>
                  <div className="row-actions">
                    <button className="btn btn-sm" onClick={() => handleExport(dataset)}>
                      导出CSV
                    </button>
                    <button
                      className="btn btn-sm btn-danger"
                      onClick={() => handleDelete(dataset)}
                    >
                      删除
                    </button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {selected ? (
        <div className="detail-panel">
          <div className="detail-header">
            <div>
              <h3 className="detail-title">{selected.name}</h3>
              <div className="detail-meta">
                共 {rows?.total ?? selected.row_count} 行 · {selected.columns.length} 列
              </div>
            </div>
            <button className="icon-btn" onClick={() => setSelected(null)} aria-label="关闭">
              ✕
            </button>
          </div>

          <div className="detail-toolbar">
            <button
              className="btn btn-sm"
              disabled={offset <= 0 || rowsLoading}
              onClick={() => loadRows(selected, Math.max(offset - PAGE_SIZE, 0))}
            >
              上一页
            </button>
            <span className="detail-meta">
              第 {Math.floor(offset / PAGE_SIZE) + 1} 页 · 显示 {offset + 1}-
              {Math.min(offset + PAGE_SIZE, rows?.total ?? 0)}
            </span>
            <button
              className="btn btn-sm"
              disabled={rowsLoading || (rows ? offset + PAGE_SIZE >= rows.total : true)}
              onClick={() => loadRows(selected, offset + PAGE_SIZE)}
            >
              下一页
            </button>
          </div>

          {rows ? (
            <DataTable
              columns={rows.columns.map((column) => column.name)}
              rows={rows.rows}
              loading={rowsLoading}
              emptyText="暂无数据"
              maxHeight="440px"
              stickyHeader
            />
          ) : (
            <div className="table-state">
              <Spinner />
              <span className="table-state-text">加载中…</span>
            </div>
          )}
        </div>
      ) : null}
    </div>
  );
}
