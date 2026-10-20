import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';
import { listProjects } from '../api/client';
import type { ExamProject } from '../api/types';

const STORAGE_KEY = 'exam.currentProjectId';

interface ProjectContextValue {
  projects: ExamProject[];
  loading: boolean;
  /** 当前选中的考试项目；为 null 表示尚未选择（此时数据集等按“全部”展示） */
  currentProject: ExamProject | null;
  currentProjectId: string;
  /** 重新拉取项目列表 */
  refresh: () => Promise<ExamProject[]>;
  /** 切换当前考试项目 */
  selectProject: (id: string) => void;
  /** 用最新的项目对象替换当前项目（例如更新了绑定流程后） */
  patchCurrentProject: (project: ExamProject) => void;
}

const ProjectContext = createContext<ProjectContextValue | null>(null);

export function ProjectProvider({ children }: { children: ReactNode }) {
  const [projects, setProjects] = useState<ExamProject[]>([]);
  const [loading, setLoading] = useState(true);
  const [currentProjectId, setCurrentProjectId] = useState<string>(
    () => window.localStorage.getItem(STORAGE_KEY) ?? '',
  );

  const refresh = useCallback(async () => {
    try {
      const list = await listProjects();
      setProjects(list);
      // 若当前选中的项目已被删除，自动回落到列表中的第一个
      setCurrentProjectId((prev) => {
        if (prev && list.some((p) => p.id === prev)) return prev;
        return list[0]?.id ?? '';
      });
      return list;
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh().catch(() => setLoading(false));
  }, [refresh]);

  const selectProject = useCallback((id: string) => {
    setCurrentProjectId(id);
    window.localStorage.setItem(STORAGE_KEY, id);
  }, []);

  const currentProject = useMemo(
    () => projects.find((p) => p.id === currentProjectId) ?? null,
    [projects, currentProjectId],
  );

  const patchCurrentProject = useCallback((project: ExamProject) => {
    setProjects((prev) => prev.map((p) => (p.id === project.id ? project : p)));
  }, []);

  const value: ProjectContextValue = {
    projects,
    loading,
    currentProject,
    currentProjectId,
    refresh,
    selectProject,
    patchCurrentProject,
  };

  return <ProjectContext.Provider value={value}>{children}</ProjectContext.Provider>;
}

export function useProject(): ProjectContextValue {
  const ctx = useContext(ProjectContext);
  if (!ctx) {
    throw new Error('useProject 必须在 ProjectProvider 内使用');
  }
  return ctx;
}
