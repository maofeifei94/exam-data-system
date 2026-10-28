import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';

/** 界面选项：表格展示相关配置，持久化在 localStorage。 */
export interface UiSettings {
  /** 表格内容是否居中（默认开启） */
  tableCenter: boolean;
  /** 是否显示表格边框（默认开启） */
  tableBorder: boolean;
}

export const DEFAULT_UI_SETTINGS: UiSettings = {
  tableCenter: true,
  tableBorder: true,
};

const STORAGE_KEY = 'exam.ui.settings.v1';

interface UiSettingsContextValue extends UiSettings {
  setTableCenter: (value: boolean) => void;
  setTableBorder: (value: boolean) => void;
  reset: () => void;
}

const UiSettingsContext = createContext<UiSettingsContextValue | null>(null);

function load(): UiSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_UI_SETTINGS;
    const parsed = JSON.parse(raw) as Partial<UiSettings>;
    return {
      tableCenter:
        typeof parsed.tableCenter === 'boolean' ? parsed.tableCenter : DEFAULT_UI_SETTINGS.tableCenter,
      tableBorder:
        typeof parsed.tableBorder === 'boolean' ? parsed.tableBorder : DEFAULT_UI_SETTINGS.tableBorder,
    };
  } catch {
    // 本地配置损坏时回退默认值，不影响功能
    return DEFAULT_UI_SETTINGS;
  }
}

export function UiSettingsProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<UiSettings>(load);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
    } catch {
      // 存储被禁用时忽略
    }
  }, [settings]);

  const setTableCenter = useCallback(
    (value: boolean) => setSettings((prev) => ({ ...prev, tableCenter: value })),
    [],
  );
  const setTableBorder = useCallback(
    (value: boolean) => setSettings((prev) => ({ ...prev, tableBorder: value })),
    [],
  );
  const reset = useCallback(() => setSettings(DEFAULT_UI_SETTINGS), []);

  const value = useMemo(
    () => ({ ...settings, setTableCenter, setTableBorder, reset }),
    [settings, setTableCenter, setTableBorder, reset],
  );

  return <UiSettingsContext.Provider value={value}>{children}</UiSettingsContext.Provider>;
}

export function useUiSettings(): UiSettingsContextValue {
  const ctx = useContext(UiSettingsContext);
  if (!ctx) {
    throw new Error('useUiSettings 必须在 UiSettingsProvider 内使用');
  }
  return ctx;
}
