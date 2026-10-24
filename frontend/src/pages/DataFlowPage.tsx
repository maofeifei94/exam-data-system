import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import {
  getDatasetRows,
  getFlowStepRows,
  listDatasets,
  listFlows,
  listPdfTemplates,
  renderPdfTemplate,
  runFlow,
} from '../api/client';
import type { Dataset, Flow, FlowStep, PdfTemplate, RowPage } from '../api/types';
import { DataTable } from '../components/DataTable';
import { EmptyState } from '../components/EmptyState';
import { Spinner } from '../components/Spinner';
import { useToast } from '../components/Toast';
import { useProject } from '../context/ProjectContext';

const PAGE_SIZE = 50;

/** 当前选中的节点：起点（数据导入）、某个流程步骤、或终点（PDF 输出）。 */
type NodeSelection =
  | { kind: 'input' }
  | { kind: 'step'; stepId: string; index: number }
  | { kind: 'output' };

interface TreeNode {
  step: FlowStep;
  index: number;
  children: TreeNode[];
}

/**
 * 把扁平的步骤数组还原成多级树：
 * `branch_of` 指向父步骤的分支节点挂到父节点下，其余步骤作为主链根节点。
 */
function buildStepTree(steps: FlowStep[]): TreeNode[] {
  const ids = new Set(steps.map((step) => step.id));
  const indexOf = new Map(steps.map((step, index) => [step.id, index]));
  const visited = new Set<string>();

  const build = (parentId: string | null): TreeNode[] =>
    steps
      .filter((step) => {
        const parent = step.branch_of ?? '';
        return parentId === null ? !parent || !ids.has(parent) : parent === parentId;
      })
      .filter((step) => !visited.has(step.id))
      .map((step) => {
        visited.add(step.id);
        return {
          step,
          index: indexOf.get(step.id) ?? 0,
          children: build(step.id),
        };
      });

  return build(null);
}

function stepLabel(step: FlowStep): string {
  return step.label.trim() || step.operator;
}

export default function DataFlowPage() {
  const { show } = useToast();
  const { currentProject, currentProjectId } = useProject();

  const [loading, setLoading] = useState(true);
  const [flow, setFlow] = useState<Flow | null>(null);
  const [dataset, setDataset] = useState<Dataset | null>(null);
  const [template, setTemplate] = useState<PdfTemplate | null>(null);

  const [selected, setSelected] = useState<NodeSelection | null>({ kind: 'input' });
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [rows, setRows] = useState<RowPage | null>(null);
  const [rowsLoading, setRowsLoading] = useState(false);
  const [offset, setOffset] = useState(0);

  const [pdfUrl, setPdfUrl] = useState<string | null>(null);
  const [pdfLoading, setPdfLoading] = useState(false);
  const [runMessage, setRunMessage] = useState('');

  // 项目或绑定关系变化时重新加载数据流。
  useEffect(() => {
    if (!currentProjectId) {
      setFlow(null);
      setDataset(null);
      setTemplate(null);
      setLoading(false);
      return;
    }
    let active = true;
    (async () => {
      setLoading(true);
      try {
        const [datasets, flows, templates] = await Promise.all([
          listDatasets(currentProjectId),
          listFlows({ projectId: currentProjectId }),
          listPdfTemplates({ projectId: currentProjectId }),
        ]);
        if (!active) return;
        setDataset(
          datasets.find((item) => item.id === currentProject?.dataset_id) ?? datasets[0] ?? null,
        );
        setFlow(flows.find((item) => item.id === currentProject?.flow_id) ?? flows[0] ?? null);
        setTemplate(
          templates.find((item) => item.id === currentProject?.pdf_template_id) ?? null,
        );
        setSelected({ kind: 'input' });
        setExpanded({});
        setRunMessage('');
        setPdfUrl((prev) => {
          if (prev) URL.revokeObjectURL(prev);
          return null;
        });
      } catch (err) {
        show(err instanceof Error ? err.message : '加载数据流失败', 'error');
      } finally {
        if (active) setLoading(false);
      }
    })();
    return () => {
      active = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    currentProjectId,
    currentProject?.dataset_id,
    currentProject?.flow_id,
    currentProject?.pdf_template_id,
  ]);

  // 卸载时释放 PDF 的 Blob URL。
  useEffect(
    () => () => {
      if (pdfUrl) URL.revokeObjectURL(pdfUrl);
    },
    [pdfUrl],
  );

  const loadRows = useCallback(
    async (node: NodeSelection, nextOffset: number) => {
      if (!flow || !dataset) {
        setRows(null);
        return;
      }
      setRowsLoading(true);
      try {
        const page =
          node.kind === 'input'
            ? await getDatasetRows(dataset.id, nextOffset, PAGE_SIZE)
            : await getFlowStepRows(
                flow.id,
                node.kind === 'output' ? flow.steps.length : node.index,
                dataset.id,
                nextOffset,
                PAGE_SIZE,
              );
        setRows(page);
        setOffset(nextOffset);
      } catch (err) {
        setRows(null);
        show(err instanceof Error ? err.message : '加载节点数据失败', 'error');
      } finally {
        setRowsLoading(false);
      }
    },
    [flow, dataset, show],
  );

  useEffect(() => {
    if (!selected) {
      setRows(null);
      return;
    }
    setRows(null);
    setOffset(0);
    void loadRows(selected, 0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected, loadRows]);

  const tree = useMemo(() => (flow ? buildStepTree(flow.steps) : []), [flow]);

  const toggle = (id: string) => {
    setExpanded((prev) => ({ ...prev, [id]: !(prev[id] ?? true) }));
  };

  const total = rows?.total ?? 0;
  const page = Math.floor(offset / PAGE_SIZE) + 1;
  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE));

  const selectedStep = selected?.kind === 'step' ? flow?.steps[selected.index] ?? null : null;
  const title =
    selected?.kind === 'input'
      ? '数据导入'
      : selected?.kind === 'output'
        ? '最终输出'
        : selectedStep
          ? stepLabel(selectedStep)
          : '节点数据';
  const subtitle =
    selected?.kind === 'input'
      ? `${dataset?.name ?? '未绑定数据集'} · 原始数据`
      : selected?.kind === 'output'
        ? '整条流程执行后的结果（含排序与截断）'
        : selectedStep
          ? `${selectedStep.operator}${selectedStep.enabled ? '' : ' · 已禁用'}`
          : '从左侧节点树中选择一个节点';

  const generatePdf = async () => {
    if (!flow || !dataset || !template) return;
    setPdfLoading(true);
    try {
      const result = await runFlow(flow.id, dataset.id);
      if (result.status !== 'success' || !result.output_dataset_id) {
        show(result.message ?? '流程执行失败，无法生成 PDF', 'error');
        return;
      }
      setRunMessage(result.message ?? '');
      const blob = await renderPdfTemplate(template.id, result.output_dataset_id);
      setPdfUrl(URL.createObjectURL(blob));
      // 输出表格同步刷新为本次运行的实际结果。
      void loadRows({ kind: 'output' }, 0);
      show('PDF 已生成', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '生成 PDF 失败', 'error');
    } finally {
      setPdfLoading(false);
    }
  };

  const downloadPdf = () => {
    if (!pdfUrl) return;
    const link = document.createElement('a');
    link.href = pdfUrl;
    link.download = `${template?.name ?? '输出'}.pdf`;
    document.body.appendChild(link);
    link.click();
    link.remove();
  };

  const renderTree = (nodes: TreeNode[], depth: number): ReactNode =>
    nodes.map((node) => {
      const open = expanded[node.step.id] ?? true;
      const active = selected?.kind === 'step' && selected.stepId === node.step.id;
      const hasChildren = node.children.length > 0;
      return (
        <div key={node.step.id}>
          <div
            className={`dataflow-node ${active ? 'active' : ''} ${
              node.step.enabled ? '' : 'disabled'
            }`}
            style={{ paddingLeft: 8 + depth * 16 }}
            onClick={() =>
              setSelected({ kind: 'step', stepId: node.step.id, index: node.index })
            }
          >
            <button
              type="button"
              className={`dataflow-caret ${hasChildren ? '' : 'empty'}`}
              onClick={(event) => {
                event.stopPropagation();
                if (hasChildren) toggle(node.step.id);
              }}
              aria-label={open ? '折叠' : '展开'}
            >
              {hasChildren ? (open ? '▾' : '▸') : '·'}
            </button>
            <span className="dataflow-node-main">
              <span className="dataflow-node-label">{stepLabel(node.step)}</span>
              <span className="dataflow-node-meta">
                {node.step.operator}
                {node.step.enabled ? '' : ' · 已禁用'}
              </span>
            </span>
            <span className="dataflow-node-index">{node.index + 1}</span>
          </div>
          {hasChildren && open ? renderTree(node.children, depth + 1) : null}
        </div>
      );
    });

  if (!currentProjectId) {
    return (
      <div className="page">
        <div className="page-header">
          <div>
            <h2 className="page-title">数据流</h2>
            <p className="page-subtitle">按节点查阅流程每一步的数据，以及最终 PDF 输出。</p>
          </div>
        </div>
        <div className="page-fill">
          <EmptyState text="请先在顶部选择一个考试项目" />
        </div>
      </div>
    );
  }

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
        <div>
          <h2 className="page-title">数据流</h2>
          <p className="page-subtitle">
            左侧是流程节点树；点击任一节点即可查看它执行后的数据，终点节点还能生成 PDF。
          </p>
        </div>
        <div className="row-actions">
          <button
            className="btn btn-sm"
            disabled={!selected || rowsLoading}
            onClick={() => selected && loadRows(selected, offset)}
          >
            刷新
          </button>
        </div>
      </div>

      <div className="dataflow-layout page-fill">
        {/* ---------------- 左：节点树 ---------------- */}
        <aside className="card dataflow-tree">
          <h3 className="card-title">流程节点</h3>
          <div className="dataflow-tree-list">
            <div
              className={`dataflow-node anchor ${selected?.kind === 'input' ? 'active' : ''}`}
              onClick={() => setSelected({ kind: 'input' })}
            >
              <span className="dataflow-caret empty">·</span>
              <span className="dataflow-node-main">
                <span className="dataflow-node-label">数据导入</span>
                <span className="dataflow-node-meta">
                  {dataset ? `${dataset.name} · ${dataset.row_count} 行` : '未绑定数据集'}
                </span>
              </span>
              <span className="dataflow-node-index">起</span>
            </div>

            {flow ? (
              tree.length ? (
                renderTree(tree, 0)
              ) : (
                <p className="panel-hint">流程还没有处理步骤</p>
              )
            ) : (
              <p className="panel-hint">尚未创建项目流程</p>
            )}

            <div
              className={`dataflow-node anchor ${selected?.kind === 'output' ? 'active' : ''}`}
              onClick={() => setSelected({ kind: 'output' })}
            >
              <span className="dataflow-caret empty">·</span>
              <span className="dataflow-node-main">
                <span className="dataflow-node-label">PDF 输出</span>
                <span className="dataflow-node-meta">
                  {template ? template.name : '未绑定 PDF 模板'}
                </span>
              </span>
              <span className="dataflow-node-index">终</span>
            </div>
          </div>
        </aside>

        {/* ---------------- 右：节点数据 / PDF 结果 ---------------- */}
        <div className="column">
          <section className="card card-fill">
            <div className="card-head-row">
              <div>
                <h3 className="card-title" style={{ margin: 0 }}>
                  {title}
                </h3>
                <div className="detail-meta">{subtitle}</div>
              </div>
              <div className="row-actions">
                <button
                  className="btn btn-sm"
                  disabled={offset <= 0 || rowsLoading}
                  onClick={() => selected && loadRows(selected, Math.max(offset - PAGE_SIZE, 0))}
                >
                  上一页
                </button>
                <span className="detail-meta">
                  {page} / {pages}
                </span>
                <button
                  className="btn btn-sm"
                  disabled={rowsLoading || offset + PAGE_SIZE >= total}
                  onClick={() => selected && loadRows(selected, offset + PAGE_SIZE)}
                >
                  下一页
                </button>
              </div>
            </div>
            <div className="card-scroll">
              {!selected ? (
                <EmptyState text="从左侧节点树中选择一个节点" />
              ) : rows ? (
                <DataTable
                  columns={rows.columns.map((column) => column.name)}
                  rows={rows.rows}
                  loading={rowsLoading}
                  emptyText="该节点没有数据"
                  maxHeight="100%"
                  stickyHeader
                />
              ) : rowsLoading ? (
                <div className="table-state">
                  <Spinner />
                  <span className="table-state-text">加载中…</span>
                </div>
              ) : (
                <EmptyState text={dataset && flow ? '该节点没有数据' : '请先绑定数据集与流程'} />
              )}
            </div>
          </section>

          {selected?.kind === 'output' ? (
            <section className="card dataflow-pdf-card">
              <div className="card-head-row">
                <div>
                  <h3 className="card-title" style={{ margin: 0 }}>
                    PDF 模板生成结果
                  </h3>
                  <div className="detail-meta">
                    {template ? `模板：${template.name}` : '未绑定 PDF 模板'}
                    {runMessage ? ` · ${runMessage}` : ''}
                  </div>
                </div>
                <div className="row-actions">
                  <button className="btn btn-sm" disabled={!pdfUrl} onClick={downloadPdf}>
                    下载 PDF
                  </button>
                  <button
                    className="btn btn-sm btn-primary"
                    disabled={pdfLoading || !flow || !dataset || !template}
                    onClick={generatePdf}
                  >
                    {pdfLoading ? '生成中…' : pdfUrl ? '重新生成' : '生成 PDF 预览'}
                  </button>
                </div>
              </div>
              {pdfUrl ? (
                <iframe className="dataflow-pdf-frame" title="PDF 预览" src={pdfUrl} />
              ) : (
                <p className="panel-hint">
                  点击「生成 PDF 预览」会先运行流程得到最终数据，再用项目绑定的模板渲染 PDF。
                </p>
              )}
            </section>
          ) : null}
        </div>
      </div>
    </div>
  );
}
