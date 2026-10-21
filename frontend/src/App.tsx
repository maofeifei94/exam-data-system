import { useEffect, useRef, useState } from 'react';
import { Button, Select } from 'antd';
import { getHealth } from './api/client';
import { ProjectProvider, useProject } from './context/ProjectContext';
import { desktop } from './lib/native';
import { ThemeProvider, useThemeMode } from './theme';
import ProjectFilesPage from './pages/ProjectFilesPage';
import ProjectFlowPage from './pages/ProjectFlowPage';
import ImportPage from './pages/ImportPage';
import AliasPage from './pages/AliasPage';
import TemplatePage from './pages/TemplatePage';
import { NewProjectDialog } from './components/NewProjectDialog';

export type ViewKey = 'files' | 'flow' | 'import' | 'alias' | 'templates';

interface ViewMeta {
  key: ViewKey;
  label: string;
  icon: string;
}

const VIEWS: Record<ViewKey, ViewMeta> = {
  files: { key: 'files', label: '项目文件', icon: 'folder' },
  flow: { key: 'flow', label: '项目流程', icon: 'flow' },
  import: { key: 'import', label: '数据导入', icon: 'import' },
  alias: { key: 'alias', label: '字段别名', icon: 'tag' },
  templates: { key: 'templates', label: 'PDF 模板', icon: 'layout' },
};

/** 左侧边栏只保留两项：项目文件、项目流程。字段别名 / PDF 模板属于全局配置，从顶栏菜单进入。 */
const SIDEBAR_VIEWS: ViewKey[] = ['files', 'flow'];

const ICONS: Record<string, string> = {
  folder: 'M3 5.5A1.5 1.5 0 0 1 4.5 4h4l1.5 2H19a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 19 17H4.5A1.5 1.5 0 0 1 3 15.5z',
  table: 'M3 5h18v3H3zM3 10h9v9H3zM14 10h7v4h-7zM14 16h7v3h-7z',
  import: 'M12 3v10m0 0-4-4m4 4 4-4M4 17h16v3H4z',
  flow: 'M6 4h5v5H6zM13 15h5v5h-5zM8.5 9v3.5a2 2 0 0 0 2 2H13M13 20.5H8.5',
  layout: 'M3 4h18v16H3zM3 9h18M9 9v11',
  tag: 'M3 12 12 3h7a2 2 0 0 1 2 2v7l-9 9zM16 7h.01',
  close: 'M5 5l14 14M19 5 5 19',
  dot: 'M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z',
  folderOpen: 'M3 7h6l2 2h8v9H3zM3 7V5h6l2 2',
  refresh: 'M4 11a8 8 0 0 1 13.5-5.7L20 8M20 4v4h-4M20 13a8 8 0 0 1-13.5 5.7L4 16M4 20v-4h4',
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
// 自定义标题栏：窗口控制按钮（无边框窗口）
// ---------------------------------------------------------------------------

function WindowControls() {
  const api = desktop();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!api) return;
    api.isMaximized().then(setMaximized).catch(() => undefined);
    return api.onMaximizedChange(setMaximized);
  }, [api]);

  if (!api) return null;

  return (
    <div className="window-controls">
      <button className="wc-btn" title="最小化" onClick={() => api.minimize()}>
        <svg width="12" height="12" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="1.8">
          <path d="M5 12h14" />
        </svg>
      </button>
      <button
        className="wc-btn"
        title={maximized ? '还原' : '最大化'}
        onClick={() => api.toggleMaximize()}
      >
        {maximized ? (
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6">
            <rect x="5" y="9" width="10" height="10" />
            <path d="M9 5h10v10" />
          </svg>
        ) : (
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6">
            <rect x="5" y="5" width="14" height="14" />
          </svg>
        )}
      </button>
      <button className="wc-btn wc-close" title="关闭" onClick={() => api.close()}>
        <svg width="12" height="12" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="1.8">
          <path d="M5 5l14 14M19 5 5 19" />
        </svg>
      </button>
    </div>
  );
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
  const api = desktop();
  const { projects, currentProject, selectProject } = useProject();
  const { mode, toggle: toggleTheme } = useThemeMode();
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
      { label: '数据导入…', onClick: () => onOpenView('import') },
      { separator: true, label: '' },
      {
        label: '在文件管理器中打开项目文件夹',
        disabled: !currentProject?.folder,
        onClick: () => currentProject && api?.openPath(currentProject.folder),
      },
      { separator: true, label: '' },
      { label: '退出', disabled: !api, onClick: () => api?.close() },
    ],
    项目: projects.length
      ? projects.map((project) => ({
          label: `${project.id === currentProject?.id ? '● ' : '　'}${project.name}`,
          onClick: () => selectProject(project.id),
        }))
      : [{ label: '暂无项目，先新建一个', disabled: true }],
    全局配置: [
      { label: '字段别名管理', onClick: () => onOpenView('alias') },
      { label: 'PDF 模板库', onClick: () => onOpenView('templates') },
      { separator: true, label: '' },
      { label: mode === 'dark' ? '切换到浅色模式' : '切换到深色模式', onClick: toggleTheme },
    ],
    帮助: [{ label: '考试数据处理系统 v0.1.0', disabled: true }],
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
      <div className="menubar-fill" />
      <WindowControls />
    </div>
  );
}

// ---------------------------------------------------------------------------
// 主外壳
// ---------------------------------------------------------------------------

function Workbench() {
  const api = desktop();
  const { projects, currentProject, currentProjectId, selectProject, loading } = useProject();
  const connected = useBackendStatus();
  const { mode, toggle: toggleTheme } = useThemeMode();

  const [active, setActive] = useState<ViewKey>('files');
  const [newProjectOpen, setNewProjectOpen] = useState(false);

  // 首次启动且没有任何项目时，直接弹出「新建项目」窗口（IDE 常见行为）
  useEffect(() => {
    if (!loading && projects.length === 0) {
      setNewProjectOpen(true);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loading]);

  const openView = (view: ViewKey) => setActive(view);

  const renderView = (view: ViewKey) => {
    switch (view) {
      case 'files':
        return (
          <ProjectFilesPage
            onNewProject={() => setNewProjectOpen(true)}
            onOpenImport={() => openView('import')}
            onOpenFlow={() => openView('flow')}
          />
        );
      case 'flow':
        return <ProjectFlowPage />;
      case 'import':
        return <ImportPage onNavigate={(next) => openView(next)} />;
      case 'alias':
        return <AliasPage />;
      case 'templates':
        return <TemplatePage />;
      default:
        return null;
    }
  };

  return (
    <div className="vscode">
      <MenuBar onOpenView={openView} onNewProject={() => setNewProjectOpen(true)} />

      {/* 项目切换工具条 */}
      <div className="toolbar">
        <div className="toolbar-select">
          <span className="toolbar-label">当前项目</span>
          <Select
            size="small"
            style={{ width: 220 }}
            value={currentProjectId}
            onChange={(value) => selectProject(value)}
            disabled={loading}
            options={[
              { value: '', label: projects.length === 0 ? '（暂无项目，显示全部数据）' : '全部数据（不限定项目）' },
              ...projects.map((project) => ({ value: project.id, label: project.name })),
            ]}
          />
        </div>
        {currentProject?.folder ? (
          <button
            className="toolbar-path toolbar-path-link"
            title={`在文件管理器中打开：${currentProject.folder}`}
            onClick={() => api?.openPath(currentProject.folder)}
          >
            <Icon name="folderOpen" size={14} />
            <span>{currentProject.folder}</span>
          </button>
        ) : (
          <span className="toolbar-path muted">未指定项目文件夹</span>
        )}
        <div className="toolbar-fill" />
        <Button size="small" onClick={toggleTheme}>
          {mode === 'dark' ? '浅色模式' : '深色模式'}
        </Button>
        <Button size="small" type="primary" onClick={() => setNewProjectOpen(true)}>
          ＋ 新建项目
        </Button>
      </div>

      <div className="workbench">
        {/* 左侧边栏：只保留项目文件 / 项目流程 */}
        <aside className="activitybar">
          {SIDEBAR_VIEWS.map((view) => {
            const meta = VIEWS[view];
            return (
              <button
                key={view}
                className={`activity-item ${active === view ? 'active' : ''}`}
                onClick={() => openView(view)}
              >
                <Icon name={meta.icon} size={22} />
                <span className="activity-label">{meta.label}</span>
              </button>
            );
          })}
        </aside>

        <section className="editor">
          <div className="editor-content">
            <div className="editor-scroll">{renderView(active)}</div>
          </div>
        </section>
      </div>

      <footer className="statusbar">
        <span className="status-item">
          <span className={`status-dot ${connected ? 'ok' : 'down'}`} />
          {connected ? '后端已连接' : '后端未连接'}
        </span>
        <span className="status-item">项目：{currentProject?.name ?? '全部数据'}</span>
        <span className="status-spacer" />
        <span className="status-item">{VIEWS[active].label}</span>
      </footer>

      <NewProjectDialog open={newProjectOpen} onClose={() => setNewProjectOpen(false)} />
    </div>
  );
}

export default function App() {
  return (
    <ThemeProvider>
      <ProjectProvider>
        <Workbench />
      </ProjectProvider>
    </ThemeProvider>
  );
}
