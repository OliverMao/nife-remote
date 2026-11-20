// 登录页：没登录的时候整个窗口就是这个页面（不显示左栏和里面的任何功能）。
//
// 账号在中转服务器旁边的账号服务里（server/auth）。第一次用这台电脑要先填账号服务地址
// （打包时可以用 NIFE_REMOTE_AUTH 预置），之后这里只填账号名和密码。
import { useState, type FormEvent } from "react";
import { api, toProblem, type Problem } from "../lib/api";
import { useT } from "../lib/i18n";
import { problemText } from "../lib/messages";
import { store, useStore } from "../lib/store";
import { syncDevicesFromAccount } from "../lib/actions";
import { IconWarn } from "./Icons";
import { Logo } from "./Logo";

export function LoginView() {
  const t = useT();
  const app = useStore((s) => s.app);
  const [mode, setMode] = useState<"login" | "register">("login");
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<Problem | null>(null);
  // 账号服务地址：还没配过时展开
  const [auth, setAuth] = useState("");
  const [editingAuth, setEditingAuth] = useState(false);
  const shown = app?.auth ?? "";
  const needsAuth = !shown;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const info = mode === "login" ? await api.accountLogin(name, password) : await api.accountRegister(name, password);
      store.setAccount(info);
      void syncDevicesFromAccount();
    } catch (err) {
      setError(toProblem(err));
      setBusy(false);
    }
  };

  const saveAuth = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.saveAuth(auth);
      // 地址在启动时读一次，保存后重启让登录页按新地址工作
      await api.restartApp();
    } catch (err) {
      setError(toProblem(err));
      setBusy(false);
    }
  };

  return (
    <div className="login-page">
      <div className="login-card">
        <div className="login-brand">
          <Logo size={34} />
          <div>
            <div className="login-name">Nife Remote</div>
            <div className="login-sub">{t("login.subtitle")}</div>
          </div>
        </div>

        {needsAuth || editingAuth ? (
          <form className="login-form" onSubmit={saveAuth}>
            <label className="login-label" htmlFor="login-auth">
              {t("server.auth")}
            </label>
            <input
              id="login-auth"
              className="login-input mono"
              value={auth}
              onChange={(e) => setAuth(e.target.value)}
              placeholder={t("server.authHint")}
              autoFocus
              spellCheck={false}
              disabled={busy}
            />
            <p className="login-hint">{t("login.authHint")}</p>
            <button className="login-btn" type="submit" disabled={busy || !auth.trim()}>
              {t("login.authSave")}
            </button>
            {!needsAuth && (
              <button className="login-link" type="button" onClick={() => setEditingAuth(false)} disabled={busy}>
                {t("common.cancel")}
              </button>
            )}
            {error && (
              <div className="error-box left" role="alert">
                <IconWarn size={15} />
                <span>{problemText(error)}</span>
              </div>
            )}
          </form>
        ) : (
          <>
            <div className="login-tabs" role="tablist">
              <button
                className={mode === "login" ? "on" : ""}
                onClick={() => (setMode("login"), setError(null))}
                disabled={busy}
              >
                {t("account.login")}
              </button>
              <button
                className={mode === "register" ? "on" : ""}
                onClick={() => (setMode("register"), setError(null))}
                disabled={busy}
              >
                {t("account.register")}
              </button>
            </div>

            <form className="login-form" onSubmit={submit}>
              <input
                className="login-input"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={t("account.name")}
                aria-label={t("account.name")}
                autoFocus
                disabled={busy}
              />
              <input
                className="login-input"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={t("account.password")}
                aria-label={t("account.password")}
                disabled={busy}
              />
              <button className="login-btn" type="submit" disabled={busy || !name.trim() || password.length < 6}>
                {t(mode === "login" ? "account.login" : "account.register")}
              </button>
              {error && (
                <div className="error-box left" role="alert">
                  <IconWarn size={15} />
                  <span>{problemText(error)}</span>
                </div>
              )}
            </form>
          </>
        )}

        <div className="login-foot">
          <span className="mono">{shown || t("server.authNotSet")}</span>
          {!editingAuth && (
            <button className="login-link" onClick={() => (setAuth(shown), setEditingAuth(true))}>
              {t("login.changeAuth")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
