import type { ReactNode } from 'react';
import { DialogWindow } from './DialogWindow';

export interface ModalProps {
  open: boolean;
  title?: string;
  children: ReactNode;
  onClose: () => void;
  width?: number | string;
  footer?: ReactNode;
}

/** 兼容旧调用：统一改用窗口式对话框，保持桌面软件的窗口外观。 */
export function Modal({ open, title, children, onClose, width = 520, footer }: ModalProps) {
  return (
    <DialogWindow open={open} title={title ?? ''} onClose={onClose} width={width} footer={footer}>
      {children}
    </DialogWindow>
  );
}

export default Modal;
