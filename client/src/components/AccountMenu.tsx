// 账号菜单（标题栏右上角那个人像）：登录 / 注册 / 退出。
//
// 账号是本机的（accounts.json，见 Rust 的 accounts 模块）：还没有服务器侧账号服务，
// 所以这里登录的是这台电脑上的账号，密码只存盐 + 摘要。
import { useState, type FormEvent } from "react";
import { api, toProblem, type AccountInfo, type Problem } from "../lib/api";
import { useT } from "../lib/i18n";
import { problemText } from "../lib/messages";
import { syncDevicesFromAccount } from "../lib/actions";
import { store, useStore } from "../lib/store";
import { IconUser } from "./Icons";
import { Popover } from "./Popover";

export function AccountButton() {
  const t = useT();
  const account = useStore((s) => s.account);
  const [open, setOpen] = useState(false);
  return (
    <div className="menu-anchor">
      <button
        className={`ta-icon ${open ? "on" : ""} ${account ? "signed-in" : ""}`}
        title={account ? account.name : t("account.title")}
        aria-label={account ? account.name : t("account.title")}
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        {account ? (
          <span className="ta-avatar">{account.name.slice(0, 1).toUpperCase()}</span>
        ) : (
          <IconUser size={17} />
        )}
      </button>
      {open && <AccountMenu onClose={() => setOpen(false)} />}
    </div>
  );
}

function AccountMenu({ onClose }: { onClose: () => void }) {
  const t = useT();
  const account = useStore((s) => s.account);
  const [mode, setMode] = useState<"login" | "register">("login");
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<Problem | null>(null);
  const [changing, setChanging] = useState(false);
  const [oldPassword, setOldPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [changed, setChanged] = useState(false);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const info =
        mode === "login" ? await api.accountLogin(name, password) : await api.accountRegister(name, password);
      store.setAccount(info);
      // 登录后把账号里的设备拉回来（换机器登录也能看到自己的设备）
      void syncDevicesFromAccount();
      onClose();
    } catch (err) {
      setError(toProblem(err));
    } finally {
      setBusy(false);
    }
  };

  const changePassword = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    setChanged(false);
    try {
      const info = await api.accountChangePassword(oldPassword, newPassword);
      store.setAccount(info);
      setChanged(true);
      setOldPassword("");
      setNewPassword("");
    } catch (err) {
      setError(toProblem(err));
    } finally {
      setBusy(false);
    }
  };

  if (account) {
    return (
      <Popover className="account-menu" onClose={onClose}>
        <div className="am-head">
          <span className="ta-avatar big">{account.name.slice(0, 1).toUpperCase()}</span>
          <div className="am-head-text">
            <div className="am-name">{account.name}</div>
            <div className="muted small">{t("account.local")}</div>
          </div>
        </div>
        <div className="am-actions">
          <button className={`am-item ${changing ? "open" : ""}`} onClick={() => setChanging((c) => !c)}>
            <span className="am-item-text">{t("account.change")}</span>
          </button>
          {changing && (
            <form className="acc-form" onSubmit={changePassword}>
              <input
                className="acc-input"
                type="password"
                value={oldPassword}
                onChange={(e) => setOldPassword(e.target.value)}
                placeholder={t("account.oldPassword")}
                aria-label={t("account.oldPassword")}
                disabled={busy}
              />
              <input
                className="acc-input"
                type="password"
                value={newPassword}
                onChange={(e) => setNewPassword(e.target.value)}
                placeholder={t("account.newPassword")}
                aria-label={t("account.newPassword")}
                disabled={busy}
              />
              <button
                className="primary-btn"
                type="submit"
                disabled={busy || oldPassword.length < 6 || newPassword.length < 6}
              >
                {t("account.savePassword")}
              </button>
              {changed && <div className="muted small">{t("account.passwordChanged")}</div>}
            </form>
          )}
          <button
            className="am-item"
            onClick={() =>
              void api.accountLogout().then(
                () => {
                  store.setAccount(null);
                  onClose();
                },
                (e) => setError(toProblem(e)),
              )
            }
          >
            <span className="am-item-text">{t("account.logout")}</span>
          </button>
        </div>
        {error && <div className="error-box left">{problemText(error)}</div>}
      </Popover>
    );
  }

  return (
    <Popover className="account-menu" onClose={onClose}>
      <div className="acc-tabs">
        <button className={mode === "login" ? "on" : ""} onClick={() => (setMode("login"), setError(null))}>
          {t("account.login")}
        </button>
        <button className={mode === "register" ? "on" : ""} onClick={() => (setMode("register"), setError(null))}>
          {t("account.register")}
        </button>
      </div>
      <form className="acc-form" onSubmit={submit}>
        <input
          className="acc-input"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={t("account.name")}
          autoFocus
          aria-label={t("account.name")}
          disabled={busy}
        />
        <input
          className="acc-input"
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          placeholder={t("account.password")}
          aria-label={t("account.password")}
          disabled={busy}
        />
        <button className="primary-btn" type="submit" disabled={busy || !name.trim() || password.length < 6}>
          {t(mode === "login" ? "account.login" : "account.register")}
        </button>
        <div className="muted small">{t("account.hint")}</div>
        {error && <div className="error-box left">{problemText(error)}</div>}
      </form>
    </Popover>
  );
}

export type { AccountInfo };
