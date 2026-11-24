// 登录页：没登录的时候整个窗口就是这个页面（不显示左栏和里面的任何功能）。
//
// 账号在中转服务器旁边的账号服务里（server/auth）。账号服务地址在打包时用 `NIFE_REMOTE_AUTH` 写进程序，
// 这里只填账号名和密码；这个版本没配地址时直接说明，不给输入框。
import { useState, type FormEvent } from "react";
import { api, toProblem, type Problem } from "../lib/api";
import { useT } from "../lib/i18n";
import { problemText } from "../lib/messages";
import { store, useStore } from "../lib/store";
import { refreshAccountDevices } from "../lib/actions";
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
  // 账号服务是打包时定好的，界面不显示也不改；这个版本没配时（构建漏了）只提示一句话
  const ready = app?.accountReady !== false;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const info = mode === "login" ? await api.accountLogin(name, password) : await api.accountRegister(name, password);
      store.setAccount(info);
      void refreshAccountDevices();
    } catch (err) {
      setError(toProblem(err));
      setBusy(false);
    }
  };

  // 网页版没有账号服务：`server/auth` 的账号是给客户端登录用的，浏览器这边不接（见 lib/backend/web.ts）。
  // 中转服务器在容器启动时就配好了（NIFE_REMOTE_SERVER），这里直接进主界面，输对方的控制码就能连。
  if (api.kind === "web") {
    return (
      <div className="login-page">
        <div className="login-card">
          <div className="login-brand">
            <Logo size={34} />
            <div>
              <div className="login-name">Nife Remote</div>
              <div className="login-sub">{t("web.subtitle")}</div>
            </div>
          </div>
          <p className="login-hint">{t("web.noAccountHint")}</p>
          <button
            className="login-btn"
            onClick={() =>
              store.setAccount({ id: "web-local", name: t("web.localName"), createdAt: 0, devices: [], deviceKeys: {}, deviceOnline: {}, deviceInfo: {} })
            }
          >
            {t("web.start")}
          </button>
        </div>
      </div>
    );
  }

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

        {ready ? (
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
        ) : (
          <p className="login-hint">{t("login.noAuthHint")}</p>
        )}
      </div>
    </div>
  );
}
