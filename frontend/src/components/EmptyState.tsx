import type { ReactNode } from 'react';

export interface EmptyStateProps {
  text?: string;
  children?: ReactNode;
}

export function EmptyState({ text = '暂无数据', children }: EmptyStateProps) {
  return (
    <div className="empty-state">
      <div className="empty-state-icon">📭</div>
      <div className="empty-state-text">{text}</div>
      {children}
    </div>
  );
}

export default EmptyState;
