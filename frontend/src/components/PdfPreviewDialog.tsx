import { DialogWindow } from './DialogWindow';
import { Spinner } from './Spinner';

export interface PdfPreviewDialogProps {
  open: boolean;
  title?: string;
  /** 后台生成的 PDF Blob URL；为空表示尚未生成 */
  url: string | null;
  filename: string;
  loading?: boolean;
  /** 底部提示，例如「预览最多 60 行」 */
  hint?: string;
  onClose: () => void;
}

/**
 * 前台 PDF 预览窗口：模板由前台编辑，后台 Rust 渲染完成后在这里内嵌预览，
 * 滚动 / 缩放 / 翻页交给浏览器 PDF 阅读器，底部提供统一下载入口。
 */
export function PdfPreviewDialog({
  open,
  title = 'PDF 预览',
  url,
  filename,
  loading = false,
  hint,
  onClose,
}: PdfPreviewDialogProps) {
  const download = () => {
    if (!url) return;
    const link = document.createElement('a');
    link.href = url;
    link.download = filename;
    document.body.appendChild(link);
    link.click();
    link.remove();
  };

  return (
    <DialogWindow
      open={open}
      title={title}
      onClose={onClose}
      width="min(1180px, 94vw)"
      flush
      footer={
        <div className="pdf-preview-actions">
          <span className="pdf-preview-hint">{hint ?? ''}</span>
          <div className="toolbar-fill" />
          <button type="button" className="btn btn-sm" onClick={onClose}>
            关闭
          </button>
          <button
            type="button"
            className="btn btn-sm btn-primary"
            onClick={download}
            disabled={!url || loading}
          >
            下载 PDF
          </button>
        </div>
      }
    >
      <div className="pdf-preview-body">
        {loading ? (
          <div className="pdf-preview-state">
            <Spinner />
            <span>正在由后台生成 PDF…</span>
          </div>
        ) : url ? (
          <iframe className="pdf-preview-frame" src={url} title="PDF 预览" />
        ) : (
          <div className="pdf-preview-state">尚未生成 PDF</div>
        )}
      </div>
    </DialogWindow>
  );
}

export default PdfPreviewDialog;
