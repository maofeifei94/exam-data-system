import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';

/** 算子领域：基础通用 / 考试专业 / 财务专业 */
export type OperatorDomain = 'general' | 'exam' | 'finance';

export const OPERATOR_DOMAIN_OPTIONS: { value: OperatorDomain; label: string }[] = [
  { value: 'general', label: '基础通用' },
  { value: 'exam', label: '考试专业' },
  { value: 'finance', label: '财务专业' },
];

export const ALL_OPERATOR_DOMAINS: OperatorDomain[] = ['general', 'exam', 'finance'];

/** 界面选项：表格展示相关配置与算子领域，持久化在 localStorage。 */
export interface UiSettings {
  /** 表格内容是否居中（默认开启） */
  tableCenter: boolean;
  /** 是否显示表格边框（默认开启） */
  tableBorder: boolean;
  /** 处理流程面板中展示的算子领域（至少保留一个） */
  operatorDomains: OperatorDomain[];
}

export const DEFAULT_UI_SETTINGS: UiSettings = {
  tableCenter: true,
  tableBorder: true,
  operatorDomains: ALL_OPERATOR_DOMAINS,
};

const STORAGE_KEY = 'exam.ui.settings.v1';

interface UiSettingsContextValue extends UiSettings {
  setTableCenter: (value: boolean) => void;
  setTableBorder: (value: boolean) => void;
  /** 勾选/取消某个算子领域；至少保留一个领域 */
  toggleOperatorDomain: (domain: OperatorDomain) => void;
  reset: () => void;
}

const UiSettingsContext = createContext<UiSettingsContextValue | null>(null);

function load(): UiSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_UI_SETTINGS;
    const parsed = JSON.parse(raw) as Partial<UiSettings>;
    const domains = Array.isArray(parsed.operatorDomains)
      ? ALL_OPERATOR_DOMAINS.filter((d) => parsed.operatorDomains?.includes(d))
      : [];
    return {
      tableCenter:
        typeof parsed.tableCenter === 'boolean' ? parsed.tableCenter : DEFAULT_UI_SETTINGS.tableCenter,
      tableBorder:
        typeof parsed.tableBorder === 'boolean' ? parsed.tableBorder : DEFAULT_UI_SETTINGS.tableBorder,
      operatorDomains: domains.length > 0 ? domains : ALL_OPERATOR_DOMAINS,
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
  const toggleOperatorDomain = useCallback((domain: OperatorDomain) => {
    setSettings((prev) => {
      const enabled = prev.operatorDomains.includes(domain);
      // 最后一个领域不允许取消，否则算子面板会变成空白。
      if (enabled && prev.operatorDomains.length <= 1) return prev;
      return {
        ...prev,
        operatorDomains: enabled
          ? prev.operatorDomains.filter((d) => d !== domain)
          : ALL_OPERATOR_DOMAINS.filter(
              (d) => d === domain || prev.operatorDomains.includes(d),
            ),
      };
    });
  }, []);
  const reset = useCallback(() => setSettings(DEFAULT_UI_SETTINGS), []);

  const value = useMemo(
    () => ({
      ...settings,
      setTableCenter,
      setTableBorder,
      toggleOperatorDomain,
      reset,
    }),
    [settings, setTableCenter, setTableBorder, toggleOperatorDomain, reset],
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
