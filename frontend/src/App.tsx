import { useEffect, useRef, useState } from 'react';
import { getHealth } from './api/client';
import { ProjectProvider, useProject } from './context/ProjectContext';
import ProjectsPage from './pages/ProjectsPage';
import DatasetsPage from './pages/DatasetsPage';
import ImportPage from './pages/ImportPage';
import AliasPage from './pages/AliasPage';
import FlowPage from './pages/FlowPage';
import TemplatePage from './pages/TemplatePage';

export type ViewKey = 'projects' | 'datasets' | 'import' | 'flows' | 'templates' | 'alias';

interface ViewMeta {
  key: ViewKey;
  label: string;
  icon: string;
  hint: string;
}

// 活动栏图标：使用内联 SVG 路径（VS Code 风格单色图标）
const VIEWS: ViewMeta[] = [
  { key: 'projects', label: '考试项目', icon: 'folder', hint: '新建 / 切换考试项目' },
  { key: 'datasets', label: '数据集', icon: 'table', hint: '本项目的数据集' },
  { key: 'import', label: '数据导入', icon: 'import', hint: '导入并做字段映射' },
  { key: 'flows', label: '处理流程', icon: 'flow', hint: '流程图式自定义处理流程' },
  { key: 'templates', label: 'PDF 模板', icon: 'layout', hint: 'PPT 式参数化模板' },
  { key: 'alias', label: '字段别名', icon: 'tag', hint: '标准字段与别名卡片' },
];

const ICONS: Record<string, string> = {
  folder: 'M3 5.5A1.5 1.5 0 0 1 4.5 4h4l1.5 2H19a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 19 17H4.5A1.5 1.5 0 0 1 3 15.5z',
  table: 'M3 5h18v3H3zM3 10h9v9H3zM14 10h7v4h-7zM14 16h7v3h-7z',
  import: 'M12 3v10m0 0-4-4m4 4 4-4M4 17h16v3H4z',
  flow: 'M6 4h5v5H6zM13 15h5v5h-5zM8.5 9v3.5a2 2 0 0 0 2 2H13M13 20.5H8.5',
  layout: 'M3 4h18v16H3zM3 9h18M9 9v11',
  tag: 'M3 12 12 3h7a2 2 0 0 1 2 2v7l-9 9zM16 7h.01',
  close: 'M5 5l14 14M19 5 5 19',
  dot: 'M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z',
};

function Icon({ name, size = 20 }: { name: string; size?: number }) {
  return (
    <svg
      className="vs-icon"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={ICONS[name] ?? ICONS.dot} />
    </svg>
  );
}

function useBackendStatus(): boolean {
  const [connected, setConnected] = useState(false);

  useEffect(() => {
    let active = true;
    const check = async () => {
      try {
        await getHealth();
        if (active) setConnected(true);
      } catch {
        if (active) setConnected(false);
      }
    };
    check();
    const timer = window.setInterval(check, 5000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, []);

  return connected;
}

// ---------------------------------------------------------------------------
// 顶栏下拉菜单
// ---------------------------------------------------------------------------

interface MenuItem {
  label: string;
  onClick?: () => void;
  separator?: boolean;
  disabled?: boolean;
}

function MenuBar({
  onOpenView,
  onNewProject,
}: {
  onOpenView: (view: ViewKey) => void;
  onNewProject: () => void;
}) {
  const { projects, currentProject, selectProject } = useProject();
  const [open, setOpen] = useState<string | null>(null);
  const barRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDocClick = (event: MouseEvent) => {
      if (barRef.current && !barRef.current.contains(event.target as Node)) {
        setOpen(null);
      }
    };
    document.addEventListener('mousedown', onDocClick);
    return () => document.removeEventListener('mousedown', onDocClick);
  }, [open]);

  const menus: Record<string, MenuItem[]> = {
    文件: [
      { label: '新建考试项目…', onClick: onNewProject },
      { label: '导入数据', onClick: () => onOpenView('import') },
      { separator: true, label: '' },
      { label: '数据集', onClick: () => onOpenView('datasets') },
    ],
    项目: projects.length
      ? projects.map((p) => ({
          label: `${p.id === currentProject?.id ? '● ' : '　'}${p.name}`,
          onClick: () => selectProject(p.id),
        }))
      : [{ label: '暂无项目，先新建一个', disabled: true }],
    配置: [
      { label: '字段别名管理', onClick: () => onOpenView('alias') },
      { label: '处理流程库', onClick: () => onOpenView('flows') },
      { label: 'PDF 模板库', onClick: () => onOpenView('templates') },
      { separator: true, label: '' },
      { label: '考试项目设置', onClick: () => onOpenView('projects') },
    ],
    帮助: [{ label: '关于本系统', disabled: true }],
  };

  return (
    <div className="menubar" ref={barRef}>
      <div className="menubar-brand">
        <span className="menubar-logo">
          <Icon name="layout" size={16} />
        </span>
        <span className="menubar-title">考试数据处理</span>
      </div>
      {Object.entries(menus).map(([name, items]) => (
        <div key={name} className="menu">
          <button
            className={`menu-trigger ${open === name ? 'open' : ''}`}
            onClick={() => setOpen(open === name ? null : name)}
          >
            {name}
          </button>
          {open === name ? (
            <div className="menu-dropdown">
              {items.map((item, index) =>
                item.separator ? (
                  <div key={index} className="menu-sep" />
                ) : (
                  <button
                    key={index}
                    className="menu-item"
                    disabled={item.disabled}
                    onClick={() => {
                      setOpen(null);
                      item.onClick?.();
                    }}
                  >
                    {item.label}
                  </button>
                ),
              )}
            </div>
          ) : null}
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// 主外壳
// ---------------------------------------------------------------------------

function Workbench({
  initialNewProject,
  clearInitialNewProject,
}: {
  initialNewProject: boolean;
  clearInitialNewProject: () => void;
}) {
  const { projects, currentProject, currentProjectId, selectProject, refresh, loading } = useProject();
  const connected = useBackendStatus();

  const [tabs, setTabs] = useState<ViewKey[]>(['projects']);
  const [active, setActive] = useState<ViewKey>('projects');
  const [focusNewProject, setFocusNewProject] = useState(initialNewProject);

  useEffect(() => {
    if (initialNewProject) {
      setFocusNewProject(true);
      clearInitialNewProject();
    }
  }, [initialNewProject, clearInitialNewProject]);

  const openView = (view: ViewKey) => {
    setTabs((prev) => (prev.includes(view) ? prev : [...prev, view]));
    setActive(view);
  };

  const closeTab = (view: ViewKey) => {
    setTabs((prev) => {
      const next = prev.filter((t) => t !== view);
      if (view === active) {
        setActive(next[next.length - 1] ?? 'projects');
      }
      return next.length ? next : ['projects'];
    });
  };

  const openNewProject = () => {
    setFocusNewProject(true);
    openView('projects');
  };

  const renderView = (view: ViewKey) => {
    switch (view) {
      case 'projects':
        return (
          <ProjectsPage
            autoOpenCreate={focusNewProject}
            onCloseCreate={() => setFocusNewProject(false)}
            onNavigate={(next) => openView(next)}
          />
        );
      case 'datasets':
        return <DatasetsPage />;
      case 'import':
        return <ImportPage onNavigate={(next) => openView(next)} />;
      case 'flows':
        return <FlowPage />;
      case 'templates':
        return <TemplatePage />;
      case 'alias':
        return <AliasPage />;
      default:
        return null;
    }
  };

  const activeMeta = VIEWS.find((v) => v.key === active);

  return (
    <div className="vscode">
      <MenuBar onOpenView={openView} onNewProject={openNewProject} />

      {/* 项目选择器行 */}
      <div className="toolbar">
        <label className="toolbar-select">
          <span className="toolbar-label">考试项目</span>
          <select
            value={currentProjectId}
            onChange={(event) => selectProject(event.target.value)}
            disabled={loading}
          >
            {projects.length === 0 ? <option value="">（暂无项目）</option> : null}
            <option value="">全部数据（不限定项目）</option>
            {projects.map((project) => (
              <option key={project.id} value={project.id}>
                {project.name}
              </option>
            ))}
          </select>
        </label>
        {currentProject?.folder ? (
          <span className="toolbar-path" title={currentProject.config_file}>
            {currentProject.folder}
          </span>
        ) : (
          <span className="toolbar-path muted">未指定项目文件夹</span>
        )}
        <button className="btn btn-sm" onClick={openNewProject}>
          ＋ 新建考试项目
        </button>
      </div>

      <div className="workbench">
        {/* 活动栏 */}
        <aside className="activitybar">
          {VIEWS.map((view) => (
            <button
              key={view.key}
              className={`activity-item ${active === view.key ? 'active' : ''}`}
              onClick={() => openView(view.key)}
              title={view.hint}
            >
              <Icon name={view.icon} size={22} />
              <span className="activity-label">{view.label}</span>
            </button>
          ))}
          <div className="activity-spacer" />
          <button className="activity-item" onClick={() => refresh()} title="刷新项目列表">
            <Icon name="dot" size={18} />
            <span className="activity-label">刷新</span>
          </button>
        </aside>

        {/* 编辑器区 */}
        <section className="editor">
          <div className="tabbar">
            {tabs.map((view) => {
              const meta = VIEWS.find((v) => v.key === view);
              return (
                <div
                  key={view}
                  className={`tab ${active === view ? 'active' : ''}`}
                  onClick={() => setActive(view)}
                >
                  <Icon name={meta?.icon ?? 'dot'} size={14} />
                  <span className="tab-label">{meta?.label ?? view}</span>
                  <button
                    className="tab-close"
                    title="关闭"
                    onClick={(event) => {
                      event.stopPropagation();
                      closeTab(view);
                    }}
                  >
                    <Icon name="close" size={12} />
                  </button>
                </div>
              );
            })}
            <div className="tabbar-fill" />
          </div>
          <div className="editor-content">
            <div className="breadcrumb">
              <span>{currentProject ? currentProject.name : '全部数据'}</span>
              <span className="breadcrumb-sep">›</span>
              <span className="breadcrumb-current">{activeMeta?.label}</span>
            </div>
            <div className="editor-scroll">{renderView(active)}</div>
          </div>
        </section>
      </div>

      {/* 状态栏 */}
      <footer className="statusbar">
        <span className="status-item">
          <span className={`status-dot ${connected ? 'ok' : 'down'}`} />
          {connected ? '后端已连接' : '后端未连接'}
        </span>
        <span className="status-item">
          项目：{currentProject?.name ?? '全部数据'}
        </span>
        <span className="status-spacer" />
        <span className="status-item">考试数据处理系统</span>
      </footer>
    </div>
  );
}

export default function App() {
  const [initialNewProject, setInitialNewProject] = useState(false);

  return (
    <ProjectProvider>
      <Workbench
        initialNewProject={initialNewProject}
        clearInitialNewProject={() => setInitialNewProject(false)}
      />
    </ProjectProvider>
  );
}
