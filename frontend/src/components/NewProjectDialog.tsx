import { useEffect, useState } from 'react';
import { createProject } from '../api/client';
import type { ExamProject } from '../api/types';
import { useProject } from '../context/ProjectContext';
import { isDesktop, pickFolder } from '../lib/native';
import { Field } from './Field';
import { DialogWindow } from './DialogWindow';
import { useToast } from './Toast';

interface Props {
  open: boolean;
  onClose: () => void;
  onCreated?: (project: ExamProject) => void;
}

/** 新建考试项目：以独立窗口形式弹出，交互方式对齐常见 IDE 的「新建项目」对话框。 */
export function NewProjectDialog({ open, onClose, onCreated }: Props) {
  const { show } = useToast();
  const { refresh, selectProject } = useProject();

  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [folder, setFolder] = useState('');
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) return;
    setName('');
    setDescription('');
    setFolder('');
  }, [open]);

  const browse = async () => {
    if (!isDesktop()) {
      show('浏览器预览环境无法调起系统文件夹窗口，请手动输入路径', 'error');
      return;
    }
    const picked = await pickFolder(folder || undefined);
    if (picked) setFolder(picked);
  };

  const submit = async () => {
    if (!name.trim()) {
      show('请填写项目名称', 'error');
      return;
    }
    setSaving(true);
    try {
      const project = await createProject({
        name: name.trim(),
        description: description.trim(),
        folder: folder.trim(),
      });
      await refresh();
      selectProject(project.id);
      onCreated?.(project);
      onClose();
      show(`已创建项目「${project.name}」`, 'success');
    } catch (err) {
      show(err instanceof Error ? err.message : '创建项目失败', 'error');
    } finally {
      setSaving(false);
    }
  };

  return (
    <DialogWindow
      open={open}
      title="新建考试项目"
      width={560}
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            取消
          </button>
          <button className="btn btn-primary" onClick={submit} disabled={saving}>
            {saving ? '创建中…' : '创建项目'}
          </button>
        </>
      }
    >
      <Field label="项目名称" htmlFor="np-name">
        <input
          id="np-name"
          type="text"
          autoFocus
          placeholder="例如：2026 年春季教师资格考试"
          value={name}
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') submit();
          }}
        />
      </Field>
      <Field label="说明（可选）" htmlFor="np-desc">
        <input
          id="np-desc"
          type="text"
          placeholder="一句话描述本项目的用途"
          value={description}
          onChange={(event) => setDescription(event.target.value)}
        />
      </Field>
      <Field label="项目文件夹" htmlFor="np-folder">
        <div className="browse-row">
          <input
            id="np-folder"
            type="text"
            placeholder="选择一个用于存放项目配置文件与导出结果的文件夹"
            value={folder}
            onChange={(event) => setFolder(event.target.value)}
          />
          <button className="btn" onClick={browse}>
            浏览…
          </button>
        </div>
      </Field>
      <p className="dialog-hint">
        项目文件夹用于写入 <code>exam-project.json</code>。创建后可在「项目流程」中配置数据导入与 PDF 输出节点。
      </p>
    </DialogWindow>
  );
}

export default NewProjectDialog;
