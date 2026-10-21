import type { ReactNode } from 'react';

export interface EmptyStateProps {
  text?: string;
  children?: ReactNode;
}

export function EmptyState({ text = '暂无数据', children }: EmptyStateProps) {
  return (
    <div className="empty-state">
      <div className="empty-state-icon">
        <svg
          width="22"
          height="22"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth={1.6}
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <path d="M4 13h4l1.5 3h5L16 13h4" />
          <path d="M5.5 5.5h13l1.5 7.5v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-4z" />
        </svg>
      </div>
      <div className="empty-state-text">{text}</div>
      {children}
    </div>
  );
}

export default EmptyState;
