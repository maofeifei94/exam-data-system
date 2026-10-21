import { createContext, useContext, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { ConfigProvider, theme as antdTheme } from 'antd';
import zhCN from 'antd/locale/zh_CN';

export type ThemeMode = 'light' | 'dark';

interface ThemeContextValue {
  mode: ThemeMode;
  toggle: () => void;
  setMode: (mode: ThemeMode) => void;
}

const ThemeContext = createContext<ThemeContextValue>({
  mode: 'light',
  toggle: () => undefined,
  setMode: () => undefined,
});

const STORAGE_KEY = 'exam-theme-mode';

export const FONT_SANS =
  "'Noto Sans SC Local', 'Noto Sans SC', 'MiSans', 'HarmonyOS Sans SC', 'PingFang SC', 'Microsoft YaHei UI', 'Microsoft YaHei', sans-serif";

function initialMode(): ThemeMode {
  const saved = localStorage.getItem(STORAGE_KEY);
  if (saved === 'dark' || saved === 'light') return saved;
  return window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [mode, setMode] = useState<ThemeMode>(initialMode);

  useEffect(() => {
    document.documentElement.dataset.theme = mode;
    localStorage.setItem(STORAGE_KEY, mode);
  }, [mode]);

  const value = useMemo<ThemeContextValue>(
    () => ({ mode, setMode, toggle: () => setMode((prev) => (prev === 'dark' ? 'light' : 'dark')) }),
    [mode],
  );

  return (
    <ThemeContext.Provider value={value}>
      <ConfigProvider
        locale={zhCN}
        theme={{
          algorithm: mode === 'dark' ? antdTheme.darkAlgorithm : antdTheme.defaultAlgorithm,
          token: {
            colorPrimary: mode === 'dark' ? '#2dd4bf' : '#0f766e',
            colorInfo: mode === 'dark' ? '#2dd4bf' : '#0f766e',
            colorSuccess: mode === 'dark' ? '#4ade80' : '#2f8a52',
            colorWarning: mode === 'dark' ? '#fbbf24' : '#b06a16',
            colorError: mode === 'dark' ? '#f87171' : '#c0362c',
            colorBgBase: mode === 'dark' ? '#0d1210' : '#ffffff',
            colorTextBase: mode === 'dark' ? '#e8eee9' : '#17211c',
            borderRadius: 10,
            borderRadiusLG: 14,
            fontSize: 13,
            fontFamily: FONT_SANS,
            controlHeight: 34,
            boxShadowSecondary: mode === 'dark'
              ? '0 28px 70px -20px rgba(0, 0, 0, 0.8)'
              : '0 24px 60px -24px rgba(12, 26, 20, 0.4)',
          },
        }}
      >
        {children}
      </ConfigProvider>
    </ThemeContext.Provider>
  );
}

export function useThemeMode(): ThemeContextValue {
  return useContext(ThemeContext);
}
