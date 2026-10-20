import { useEffect, useMemo, useState } from 'react';
import {
  createProject,
  deleteProject,
  duplicateFlow,
  duplicatePdfTemplate,
  listFlows,
  listPdfTemplates,
  updateProject,
} from '../api/client';
import type { ExamProject, Flow, PdfTemplate } from '../api/types';
import { EmptyState } from '../components/EmptyState';
import { Field } from '../components/Field';
import { useToast } from '../components/Toast';
import { useProject } from '../context/ProjectContext';
import type { ViewKey } from '../App';

const GLOBAL_PREFIX = 'global:';

interface ProjectForm {
  id: string | null;
  name: string;
  description: string;
  folder: string;
  flowId: string;
  pdfTemplateId: string;
}

const EMPTY_FORM: ProjectForm = {
  id: null,
  name: '',
  description: '',
  folder: '',
  flowId: '',
  pdfTemplateId: '',
};

interface Props {
  autoOpenCreate?: boolean;
  onCloseCreate?: () => void;
  onNavigate?: (view: ViewKey) => void;
}

export default function ProjectsPage({ autoOpenCreate, onCloseCreate, onNavigate }: Props) {
  const { show } = useToast();
  const { projects, currentProject, currentProjectId, selectProject, refresh, patchCurrentProject } =
    useProject();

  const [form, setForm] = useState<ProjectForm>(EMPTY_FORM);
  const [creating, setCreating] = useState(false);
  const [saving, setSaving] = useState(false);

  // 全局库 + 本项目库，用于“引用后立即复制一份”的绑定
  const [globalFlows, setGlobalFlows] = useState<Flow[]>([]);
  const [globalTemplates, setGlobalTemplates] = useState<PdfTemplate[]>([]);
  const [projectFlows, setProjectFlows] = useState<Flow[]>([]);
  const [projectTemplates, setProjectTemplates] = useState<PdfTemplate[]>([]);
  const [loadingLibs, setLoadingLibs] = useState(true);

  useEffect(() => {
    if (autoOpenCreate) {
      setCreating(true);
      setForm(EMPTY_FORM);
    }
  }, [autoOpenCreate]);

  const loadLibs = async () => {
    setLoadingLibs(true);
    try {
      const [gf, gt] = await Promise.all([
        listFlows({ scope: 'global' }),
        listPdfTemplates({ scope: 'global' }),
      ]);
      setGlobalFlows(gf);
      setGlobalTemplates(gt);
      if (currentProjectId) {
        const [pf, pt] = await Promise.all([
          listFlows({ projectId: currentProjectId }),
          listPdfTemplates({ projectId: currentProjectId }),
        ]);
        setProjectFlows(pf);
        setProjectTemplates(pt);
      } else {
        setProjectFlows([]);
        setProjectTemplates([]);
      }
    } catch (err) {
      show(err instanceof Error ? err.message : '加载模板库失败', 'error');
    } finally {
      setLoadingLibs(false);
    }
  };

  useEffect(() => {
    loadLibs();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentProjectId]);

  // 选中项目变化时，把详情表单同步到该项目
  useEffect(() => {
    if (creating) return;
    if (currentProject) {
      setForm({
        id: currentProject.id,
        name: currentProject.name,
        description: currentProject.description,
        folder: currentProject.folder,
        flowId: currentProject.flow_id,
        pdfTemplateId: currentProject.pdf_template_id,
      });
    } else {
      setForm(EMPTY_FORM);
    }
  }, [currentProject, creating]);

  const startCreate = () => {
    setCreating(true);
    setForm(EMPTY_FORM);
  };

  const cancelCreate = () => {
    setCreating(false);
    onCloseCreate?.();
    if (currentProject) {
      setForm({
        id: currentProject.id,
        name: currentProject.name,
        description: currentProject.description,
        folder: currentProject.folder,
        flowId: currentProject.flow_id,
        pdfTemplateId: currentProject.pdf_template_id,
      });
    } else {
      setForm(EMPTY_FORM);
    }
  };

  /**
   * 处理绑定下拉的取值：
   *  - `global:<id>` → 调 duplicate 复制成项目本地副本，记录返回的副本 id
   *  - 其它（项目本地流程/模板 id 或空）→ 直接使用
   */
  const resolveBinding = async (
    raw: string,
    targetProjectId: string,
    kind: 'flow' | 'template',
  ): Promise<string> => {
    if (!raw.startsWith(GLOBAL_PREFIX)) {
      return raw;
    }
    const sourceId = raw.slice(GLOBAL_PREFIX.length);
    if (!targetProjectId) {
      show('请先保存项目，再引用全局模板', 'error');
      return '';
    }
    if (kind === 'flow') {
      const copy = await duplicateFlow(sourceId, { project_id: targetProjectId });
      setProjectFlows((prev) => [copy, ...prev]);
      show(`已引用全局流程「${copy.name}」并复制为项目副本`, 'success');
      return copy.id;
    }
    const copy = await duplicatePdfTemplate(sourceId, { project_id: targetProjectId });
    setProjectTemplates((prev) => [copy, ...prev]);
    show(`已引用全局模板「${copy.name}」并复制为项目副本`, 'success');
    return copy.id;
  };

  const handleSave = async () => {
    if (!form.name.trim()) {
      show('请填写考试项目名称', 'error');
      return;
    }
    setSaving(true);
    try {
      // 先落库拿到 id，再解析全局引用（复制需要目标项目 id）
      const base = {
        name: form.name.trim(),
        description: form.description.trim(),
        folder: form.folder.trim(),
        flow_id: form.flowId.startsWith(GLOBAL_PREFIX) ? '' : form.flowId,
        pdf_template_id: form.pdfTemplateId.startsWith(GLOBAL_PREFIX) ? '' : form.pdfTemplateId,
      };
      let project: ExamProject;
      if (form.id) {
        project = await updateProject(form.id, base);
      } else {
        project = await createProject(base);
      }

      const flowId = form.flowId
        ? await resolveBinding(form.flowId, project.id, 'flow')
        : '';
      const templateId = form.pdfTemplateId
        ? await resolveBinding(form.pdfTemplateId, project.id, 'template')
        : '';

      if (flowId !== project.flow_id || templateId !== project.pdf_template_id) {
        project = await updateProject(project.id, {
          name: project.name,
          description: project.description,
          folder: project.folder,
          flow_id: flowId,
          pdf_template_id: templateId,
        });
      }

      const list = await refresh();
      const fresh = list.find((p) => p.id === project.id) ?? project;
      patchCurrentProject(fresh);
      selectProject(fresh.id);
      setCreating(false);
      onCloseCreate?.();
      show(form.id ? '项目已更新' : '考试项目已创建并写入配置文件', 'success');
      await loadLibs();
    } catch (err) {
      show(err instanceof Error ? err.message : '保存项目失败', 'error');
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (project: ExamProject) => {
    if (!window.confirm(`确认删除考试项目「${project.name}」？其数据集将变为未归属。`)) {
      return;
    }
    try {
      await deleteProject(project.id);
      await refresh();
      show('项目已删除', 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '删除失败', 'error');
    }
  };

  const flowOptions = useMemo(() => {
    const options: { value: string; label: string }[] = [];
    projectFlows.forEach((flow) => options.push({ value: flow.id, label: `【本项目】${flow.name}` }));
    globalFlows.forEach((flow) =>
      options.push({ value: `${GLOBAL_PREFIX}${flow.id}`, label: `〔全局〕${flow.name}` }),
    );
    return options;
  }, [projectFlows, globalFlows]);

  const templateOptions = useMemo(() => {
    const options: { value: string; label: string }[] = [];
    projectTemplates.forEach((tpl) =>
      options.push({ value: tpl.id, label: `【本项目】${tpl.name}` }),
    );
    globalTemplates.forEach((tpl) =>
      options.push({ value: `${GLOBAL_PREFIX}${tpl.id}`, label: `〔全局〕${tpl.name}` }),
    );
    return options;
  }, [projectTemplates, globalTemplates]);

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h2 className="page-title">考试项目</h2>
          <p className="page-subtitle">
            每个考试项目绑定一个文件夹、一套处理流程与一套 PDF 模板；引用全局模板时会立即复制为本项目副本。
          </p>
        </div>
        <div className="row-actions">
          <button className="btn" onClick={loadLibs}>
            刷新
          </button>
          <button className="btn btn-primary" onClick={startCreate}>
            ＋ 新建考试项目
          </button>
        </div>
      </div>

      <div className="projects-grid">
        <aside className="card project-list">
          <h3 className="card-title">项目列表</h3>
          {projects.length === 0 ? (
            <EmptyState text="暂无考试项目，点击右上角新建" />
          ) : (
            <ul className="project-items">
              {projects.map((project) => (
                <li
                  key={project.id}
                  className={`project-item ${project.id === currentProjectId ? 'active' : ''}`}
                  onClick={() => {
                    selectProject(project.id);
                    setCreating(false);
                  }}
                >
                  <div className="project-item-name">{project.name}</div>
                  <div className="project-item-meta">
                    {project.folder ? project.folder : '未指定文件夹'}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </aside>

        <section className="card">
          <h3 className="card-title">{creating ? '新建考试项目' : '项目设置'}</h3>

          {!creating && !currentProject ? (
            <EmptyState text="请选择一个项目，或新建考试项目" />
          ) : (
            <>
              <Field label="项目名称" htmlFor="pj-name">
                <input
                  id="pj-name"
                  type="text"
                  placeholder="例如：2026 年春季教师资格考试"
                  value={form.name}
                  onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))}
                />
              </Field>
              <Field label="说明（可选）" htmlFor="pj-desc">
                <input
                  id="pj-desc"
                  type="text"
                  value={form.description}
                  onChange={(event) =>
                    setForm((prev) => ({ ...prev, description: event.target.value }))
                  }
                />
              </Field>
              <Field label="项目文件夹" htmlFor="pj-folder">
                <input
                  id="pj-folder"
                  type="text"
                  placeholder="例如：/workspace/exams/2026-spring"
                  value={form.folder}
                  onChange={(event) => setForm((prev) => ({ ...prev, folder: event.target.value }))}
                />
              </Field>
              <p className="panel-hint">
                保存时会在该文件夹下写入 <code>exam-project.json</code>，记录本项目绑定的流程与模板。
              </p>

              <hr className="divider" />

              <Field label="绑定的处理流程" htmlFor="pj-flow">
                <select
                  id="pj-flow"
                  value={form.flowId}
                  disabled={loadingLibs}
                  onChange={(event) => setForm((prev) => ({ ...prev, flowId: event.target.value }))}
                >
                  <option value="">未绑定</option>
                  {flowOptions.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              </Field>
              <Field label="绑定的 PDF 模板" htmlFor="pj-tpl">
                <select
                  id="pj-tpl"
                  value={form.pdfTemplateId}
                  disabled={loadingLibs}
                  onChange={(event) =>
                    setForm((prev) => ({ ...prev, pdfTemplateId: event.target.value }))
                  }
                >
                  <option value="">未绑定</option>
                  {templateOptions.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              </Field>
              <p className="panel-hint">
                选择「〔全局〕」条目时会先复制一份到本项目，之后全局库的改动不会影响本项目。
              </p>

              {!creating && currentProject?.config_file ? (
                <>
                  <hr className="divider" />
                  <h4 className="panel-subtitle">项目配置文件</h4>
                  <p className="config-path mono">{currentProject.config_file}</p>
                </>
              ) : null}

              <div className="row-actions">
                <button className="btn btn-primary" onClick={handleSave} disabled={saving}>
                  {saving ? '保存中…' : creating ? '创建项目' : '保存设置'}
                </button>
                {creating ? (
                  <button className="btn" onClick={cancelCreate}>
                    取消
                  </button>
                ) : (
                  <button
                    className="btn btn-danger"
                    onClick={() => currentProject && handleDelete(currentProject)}
                  >
                    删除项目
                  </button>
                )}
              </div>

              {!creating && onNavigate ? (
                <div className="link-row">
                  <button className="link" onClick={() => onNavigate('flows')}>
                    前往处理流程库 →
                  </button>
                  <button className="link" onClick={() => onNavigate('templates')}>
                    前往 PDF 模板库 →
                  </button>
                </div>
              ) : null}
            </>
          )}
        </section>
      </div>
    </div>
  );
}
