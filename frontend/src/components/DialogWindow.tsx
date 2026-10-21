import { useEffect, useRef, useState } from 'react';
import type { PointerEvent as ReactPointerEvent, ReactNode } from 'react';

export interface DialogWindowProps {
  open: boolean;
  title: string;
  children: ReactNode;
  onClose: () => void;
  footer?: ReactNode;
  width?: number | string;
  /** 内容自身已有布局（如内嵌整页编辑器）时，正文不再加内边距和滚动 */
  flush?: boolean;
}

/**
 * 无边框「窗口式」对话框：带独立标题栏，像 IDE 的新建项目/设置窗口，
 * 而不是网页里常见的居中卡片弹层。标题栏可拖动，关闭后位置复位。
 */
export function DialogWindow({
  open,
  title,
  children,
  onClose,
  footer,
  width = 560,
  flush = false,
}: DialogWindowProps) {
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const windowRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) setOffset({ x: 0, y: 0 });
  }, [open]);

  if (!open) return null;

  const startDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    if ((event.target as HTMLElement).closest('.dialog-close')) return;
    const el = windowRef.current;
    if (!el) return;
    event.preventDefault();
    const startX = event.clientX;
    const startY = event.clientY;
    const origin = offset;
    const rect = el.getBoundingClientRect();
    const edge = 8;
    const onMove = (ev: PointerEvent) => {
      const dx = ev.clientX - startX;
      const dy = ev.clientY - startY;
      // 限制窗口始终留在视口内，避免拖出屏幕后找不到窗口
      const maxX = Math.max(edge, window.innerWidth - rect.width - edge);
      const maxY = Math.max(edge, window.innerHeight - rect.height - edge);
      const nx = Math.min(Math.max(edge, rect.left + dx), maxX);
      const ny = Math.min(Math.max(edge, rect.top + dy), maxY);
      setOffset({ x: origin.x + (nx - rect.left), y: origin.y + (ny - rect.top) });
    };
    const onUp = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  };

  return (
    <div className="dialog-overlay" onMouseDown={onClose}>
      <div
        ref={windowRef}
        className={`dialog-window ${flush ? 'dialog-flush' : ''}`}
        style={{ width, transform: `translate(${offset.x}px, ${offset.y}px)` }}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="dialog-titlebar" onPointerDown={startDrag}>
          <span className="dialog-title">{title}</span>
          <button className="dialog-close" onClick={onClose} aria-label="关闭">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
              <path d="M5 5l14 14M19 5 5 19" />
            </svg>
          </button>
        </div>
        <div className="dialog-body">{children}</div>
        {footer ? <div className="dialog-footer">{footer}</div> : null}
      </div>
    </div>
  );
}

export default DialogWindow;
