import { useCallback, useEffect, useMemo, useState } from 'react';
import { createUser, listOperationLogs, listUsers } from '../api/client';
import { DialogWindow } from '../components/DialogWindow';
import { EmptyState } from '../components/EmptyState';
import { Spinner } from '../components/Spinner';
import { useToast } from '../components/Toast';
import { useAuth } from '../context/AuthContext';
import type { OperationLog, UserAccount } from '../api/types';

const PAGE_SIZE = 50;

/** 日志中的对象类型 → 中文标签 */
const TARGET_LABELS: Record<string, string> = {
  project: '考试项目',
  dataset: '数据集',
  flow: '处理流程',
  pdf_template: 'PDF 模板',
  field: '字段',
  alias: '字段别名',
  user: '操作员',
  session: '登录会话',
};

const ACTIONS = [
  '登录系统',
  '退出登录',
  '导入数据',
  '运行流程',
  '生成 PDF',
  '新建项目',
  '修改项目',
  '删除项目',
  '新建流程',
  '修改流程',
  '删除流程',
  '新增操作员',
  '保存字段别名',
  '字段映射',
];

/** 表单：新增操作员 */
function CreateUserDialog({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: () => void;
}) {
  const { show } = useToast();
  const [username, setUsername] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [password, setPassword] = useState('');
  const [role, setRole] = useState('operator');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) {
      setUsername('');
      setDisplayName('');
      setPassword('');
      setRole('operator');
    }
  }, [open]);

  const submit = async () => {
    if (!username.trim() || password.length < 6) {
      show('用户名不能为空，密码至少 6 位', 'error');
      return;
    }
    setBusy(true);
    try {
      await createUser({
        username: username.trim(),
        display_name: displayName.trim(),
        password,
        role,
      });
      show('操作员已创建', 'success');
      onCreated();
      onClose();
    } catch (err) {
      show(err instanceof Error ? err.message : '创建操作员失败', 'error');
    } finally {
      setBusy(false);
    }
  };

  return (
    <DialogWindow
      open={open}
      title="新增操作员"
      width={440}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose} disabled={busy}>
            取消
          </button>
          <button type="button" className="btn btn-primary" onClick={submit} disabled={busy}>
            {busy ? '创建中…' : '创建'}
          </button>
        </>
      }
    >
      <div className="dialog-form">
        <label className="form-row">
          <span className="field-label">登录名</span>
          <input
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            placeholder="例如 zhangwei"
            autoFocus
          />
        </label>
        <label className="form-row">
          <span className="field-label">显示名称</span>
          <input
            value={displayName}
            onChange={(e) => setDisplayName(e.target.value)}
            placeholder="例如 张伟（考务组）"
          />
        </label>
        <label className="form-row">
          <span className="field-label">初始密码</span>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="至少 6 位"
          />
        </label>
        <label className="form-row">
          <span className="field-label">角色</span>
          <select value={role} onChange={(e) => setRole(e.target.value)}>
            <option value="operator">操作员（可使用数据与流程）</option>
            <option value="admin">管理员（可新增操作员）</option>
          </select>
        </label>
      </div>
    </DialogWindow>
  );
}

export default function LogsPage() {
  const { show } = useToast();
  const { user } = useAuth();
  const [users, setUsers] = useState<UserAccount[]>([]);
  const [logs, setLogs] = useState<OperationLog[]>([]);
  const [total, setTotal] = useState(0);
  const [offset, setOffset] = useState(0);
  const [loading, setLoading] = useState(false);
  const [username, setUsername] = useState('');
  const [action, setAction] = useState('');
  const [keyword, setKeyword] = useState('');
  const [createOpen, setCreateOpen] = useState(false);

  const loadUsers = useCallback(async () => {
    try {
      setUsers(await listUsers());
    } catch {
      // 用户列表失败不阻塞日志查看
    }
  }, []);

  const loadLogs = useCallback(
    async (nextOffset: number) => {
      setLoading(true);
      try {
        const page = await listOperationLogs({
          username: username || undefined,
          action: action || undefined,
          q: keyword.trim() || undefined,
          offset: nextOffset,
          limit: PAGE_SIZE,
        });
        setLogs(page.items);
        setTotal(page.total);
        setOffset(nextOffset);
      } catch (err) {
        setLogs([]);
        show(err instanceof Error ? err.message : '加载操作日志失败', 'error');
      } finally {
        setLoading(false);
      }
    },
    [username, action, keyword, show],
  );

  useEffect(() => {
    void loadUsers();
  }, [loadUsers]);

  useEffect(() => {
    void loadLogs(0);
    // 首次进入加载全部日志；筛选条件变化由「查询」按钮触发
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const page = Math.floor(offset / PAGE_SIZE) + 1;

  const rows = useMemo(
    () =>
      logs.map((log) => ({
        ...log,
        operator: log.display_name || log.username || '未知',
        target: log.target_name || TARGET_LABELS[log.target_type] || log.target_type || '—',
      })),
    [logs],
  );

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h2 className="page-title">操作日志</h2>
          <p className="page-subtitle">
            记录谁在什么时间对哪个对象做了什么；流程运行与数据导入会附带业务明细。
          </p>
        </div>
        <div className="row-actions">
          {user?.role === 'admin' ? (
            <button className="btn btn-primary" onClick={() => setCreateOpen(true)}>
              新增操作员
            </button>
          ) : null}
          <button className="btn" onClick={() => void loadLogs(offset)} disabled={loading}>
            刷新
          </button>
        </div>
      </div>

      <div className="log-filters">
        <label className="field">
          <span className="field-label">操作员</span>
          <select value={username} onChange={(e) => setUsername(e.target.value)}>
            <option value="">全部操作员</option>
            {users.map((u) => (
              <option key={u.id} value={u.username}>
                {u.display_name ? `${u.display_name}（${u.username}）` : u.username}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          <span className="field-label">操作类型</span>
          <select value={action} onChange={(e) => setAction(e.target.value)}>
            <option value="">全部操作</option>
            {ACTIONS.map((a) => (
              <option key={a} value={a}>
                {a}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          <span className="field-label">关键字</span>
          <input
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void loadLogs(0);
            }}
            placeholder="对象名 / 详情 / 操作员"
          />
        </label>
        <div className="row-actions">
          <button className="btn btn-primary" onClick={() => void loadLogs(0)} disabled={loading}>
            查询
          </button>
          <button
            className="btn"
            onClick={() => {
              setUsername('');
              setAction('');
              setKeyword('');
              void loadLogs(0);
            }}
            disabled={loading}
          >
            重置
          </button>
        </div>
        <div className="toolbar-fill" />
        <span className="field-label">共 {total} 条</span>
      </div>

      <div className="log-table-wrap">
        {loading && logs.length === 0 ? (
          <div className="table-state">
            <Spinner />
            <span className="table-state-text">加载中…</span>
          </div>
        ) : rows.length === 0 ? (
          <EmptyState text="暂无操作日志" />
        ) : (
          <table className="data-table">
            <thead className="data-table__head sticky">
              <tr>
                <th>时间</th>
                <th>操作员</th>
                <th>操作</th>
                <th>对象</th>
                <th>项目</th>
                <th>详情</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((log, index) => (
                <tr key={log.id} className={index % 2 === 1 ? 'even' : undefined}>
                  <td className="log-time">{log.created_at}</td>
                  <td>
                    {log.operator}
                    <span className="log-username">@{log.username}</span>
                  </td>
                  <td>
                    <span className="log-action">{log.action}</span>
                  </td>
                  <td>
                    {log.target}
                    {log.target_id ? <span className="log-id">{log.target_id.slice(0, 8)}</span> : null}
                  </td>
                  <td>{log.project_name || '—'}</td>
                  <td className="log-detail" title={log.detail}>
                    {log.detail || '—'}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      <div className="log-pager">
        <button
          className="btn btn-sm"
          disabled={loading || page <= 1}
          onClick={() => void loadLogs(Math.max(0, offset - PAGE_SIZE))}
        >
          上一页
        </button>
        <span className="field-label">
          第 {page} / {pages} 页
        </span>
        <button
          className="btn btn-sm"
          disabled={loading || page >= pages}
          onClick={() => void loadLogs(offset + PAGE_SIZE)}
        >
          下一页
        </button>
      </div>

      <CreateUserDialog
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        onCreated={() => {
          void loadUsers();
          void loadLogs(0);
        }}
      />
    </div>
  );
}
