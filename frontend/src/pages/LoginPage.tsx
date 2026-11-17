import { useState, type FormEvent } from 'react';
import { useAuth } from '../context/AuthContext';

/**
 * 登录页：未登录时覆盖整个工作台。
 * 默认管理员账号 admin / admin123，首次使用后建议在「操作日志」页新增个人账号。
 */
export default function LoginPage() {
  const { login } = useAuth();
  const [username, setUsername] = useState('admin');
  const [password, setPassword] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!username.trim() || !password) {
      setError('请输入用户名和密码');
      return;
    }
    setBusy(true);
    setError('');
    try {
      await login(username, password);
    } catch (err) {
      setError(err instanceof Error ? err.message : '登录失败');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="login-page">
      <form className="login-card" onSubmit={submit}>
        <div className="login-brand">
          <span className="login-logo">
            <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6">
              <path d="M12 3 3 8l9 5 9-5zM3 12.5l9 5 9-5M3 17l9 5 9-5" strokeLinejoin="round" />
            </svg>
          </span>
          <div>
            <h1 className="login-title">考试数据处理</h1>
            <p className="login-subtitle">登录后即可处理项目数据，所有操作都会记录操作员。</p>
          </div>
        </div>

        <label className="form-row">
          <span className="field-label">用户名</span>
          <input
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            autoComplete="username"
            autoFocus
          />
        </label>
        <label className="form-row">
          <span className="field-label">密码</span>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="current-password"
            placeholder="请输入密码"
          />
        </label>

        {error ? <div className="login-error">{error}</div> : null}

        <button type="submit" className="btn btn-primary login-submit" disabled={busy}>
          {busy ? '登录中…' : '登 录'}
        </button>

        <p className="login-hint">初始管理员：admin / admin123</p>
      </form>
    </div>
  );
}
