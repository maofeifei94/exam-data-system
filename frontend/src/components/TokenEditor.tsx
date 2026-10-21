import { useEffect, useRef, useState } from 'react';
import type { ChangeEvent, KeyboardEvent } from 'react';
import { fieldColor, hexToRgba, splitExprTokens, splitTemplateTokens } from '../lib/tokens';

type TokenMode = 'expr' | 'template';

interface TokenEditorProps {
  value: string;
  onChange: (value: string) => void;
  /** expr：`[字段]` 表达式；template：`{{字段}}` PDF 模板占位符 */
  mode: TokenMode;
  /** 已知字段；提供时未匹配到的字段会以红色警示 */
  fields?: string[];
  placeholder?: string;
  multiline?: boolean;
  /** 由外部触发进入编辑态（例如画布中双击元素） */
  autoEdit?: boolean;
  className?: string;
}

interface ViewToken {
  kind: 'text' | 'field' | 'builtin';
  text: string;
  field?: string;
}

/**
 * 字段胶囊编辑器。
 *
 * 展示态把字段渲染成有颜色的圆角矩阵（胶囊），不显示 `[]` / `{{}}`；
 * 双击、按 Backspace / Delete / Enter 或按 F2 时才进入编辑态，显示原始文本。
 */
export function TokenEditor({
  value,
  onChange,
  mode,
  fields,
  placeholder,
  multiline = false,
  autoEdit = false,
  className,
}: TokenEditorProps) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);
  const inputRef = useRef<HTMLTextAreaElement | HTMLInputElement | null>(null);

  useEffect(() => {
    if (!editing) setDraft(value);
  }, [editing, value]);

  useEffect(() => {
    if (autoEdit) setEditing(true);
  }, [autoEdit]);

  useEffect(() => {
    if (!editing) return;
    const el = inputRef.current;
    if (!el) return;
    el.focus();
    const end = el.value.length;
    try {
      el.setSelectionRange(end, end);
    } catch {
      // 某些输入类型不支持 setSelectionRange，忽略即可
    }
  }, [editing]);

  const beginEdit = () => {
    setDraft(value);
    setEditing(true);
  };

  const handleViewKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Backspace' || event.key === 'Delete' || event.key === 'Enter' || event.key === 'F2') {
      event.preventDefault();
      beginEdit();
    }
  };

  if (editing) {
    const handleChange = (event: ChangeEvent<HTMLTextAreaElement | HTMLInputElement>) => {
      setDraft(event.target.value);
      onChange(event.target.value);
    };
    const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement | HTMLInputElement>) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        setDraft(value);
        setEditing(false);
        return;
      }
      if (!multiline && event.key === 'Enter') {
        event.preventDefault();
        setEditing(false);
      }
    };
    if (multiline) {
      return (
        <textarea
          value={draft}
          placeholder={placeholder}
          className={`token-input ${className ?? ''}`.trim()}
          rows={2}
          onChange={handleChange}
          onBlur={() => setEditing(false)}
          onKeyDown={handleKeyDown}
          ref={(node) => {
            inputRef.current = node;
          }}
        />
      );
    }
    return (
      <input
        type="text"
        value={draft}
        placeholder={placeholder}
        className={`token-input ${className ?? ''}`.trim()}
        onChange={handleChange}
        onBlur={() => setEditing(false)}
        onKeyDown={handleKeyDown}
        ref={(node) => {
          inputRef.current = node;
        }}
      />
    );
  }

  const rawTokens: ViewToken[] =
    mode === 'expr'
      ? splitExprTokens(value).map((token) => ({ kind: token.kind, text: token.text, field: token.field }))
      : splitTemplateTokens(value).map((token) => ({ kind: token.kind, text: token.text, field: token.field }));

  const known = fields ? new Set(fields) : null;

  return (
    <div
      className={`token-view ${className ?? ''}`.trim()}
      role="textbox"
      tabIndex={0}
      onDoubleClick={beginEdit}
      onKeyDown={handleViewKeyDown}
      title="双击或按 Backspace 编辑原始内容"
    >
      {value.trim() === '' ? (
        <span className="token-placeholder">{placeholder ?? '点击或双击编辑'}</span>
      ) : (
        rawTokens.map((token, index) => {
          if (token.kind === 'text') {
            return <span key={index}>{token.text}</span>;
          }
          const name = token.field ?? '';
          const builtin = token.kind === 'builtin';
          const missing = !builtin && known !== null && !known.has(name);
          const color = missing ? '#dc2626' : builtin ? '#64748b' : fieldColor(name);
          return (
            <span
              key={index}
              className={`field-token ${builtin ? 'builtin' : ''} ${missing ? 'missing' : ''}`.trim()}
              style={{
                color,
                background: hexToRgba(color, missing ? 0.14 : builtin ? 0.1 : 0.13),
                borderColor: hexToRgba(color, missing ? 0.55 : 0.4),
              }}
              title={missing ? `字段「${name}」不在当前步骤的数据流中` : `字段：${name}`}
            >
              {name || '空字段'}
            </span>
          );
        })
      )}
    </div>
  );
}
