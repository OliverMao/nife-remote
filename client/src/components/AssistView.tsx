// 远程协助页（照 UU 的"远程协助"）：本设备的 ID / 验证码 + 复制分享，下面用伙伴的设备 ID 直连另一台电脑。
//
// 数据来源：本设备ID = 本机连接码，验证码 = 配对码（都是真的，来自被控端状态）；
// "允许他人远程协助"的开关、验证方式下拉、星标收藏还没有功能，先摆着。
import { useEffect, useState } from "react";
import { api, type Problem } from "../lib/api";
import { defaultQuality, fmtCode, openSession } from "../lib/actions";
import { useT } from "../lib/i18n";
import { problemText } from "../lib/messages";
import { store, useStore } from "../lib/store";
import { IconCheck, IconChevron, IconCopy, IconEye, IconEyeOff, IconSettings, IconStar, IconTrash } from "./Icons";

/** 这些问题要用户输入配对码（对方的身份变了也是：重新配对才认新的身份） */
const PAIRING_PROBLEMS = new Set(["pairingRequired", "pairingFailed", "pairingUnavailable", "hostKeyChanged"]);

export function AssistView() {
  const t = useT();
  const host = useStore((s) => s.host);
  const profile = useStore((s) => s.profile);
  const recent = profile.recent;
  const [copied, setCopied] = useState(false);
  const [shown, setShown] = useState(true);
  const [saving, setSaving] = useState(false);
  const [verifyOpen, setVerifyOpen] = useState(false);
  const [input, setInput] = useState("");
  const [error, setError] = useState<Problem | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);

  const code = host?.code == null ? null : String(host.code).padStart(6, "0");
  const pairing = host?.pairing ?? null;
  const digits = input.replace(/\D/g, "").slice(0, 6);
  const ready = /^\d{6}$/.test(digits) && digits[0] !== "0";
  // 正在连的那台（进度和错误都在 store 的 draft 里，切页面再回来也不会丢）
  const attempt = useStore((s) => s.draft);
  const busy = attempt?.busy ?? false;
  const problem = attempt?.error ?? null;
  const needsPairing = !!problem && PAIRING_PROBLEMS.has(problem.kind);
  // 配对码就地输入（对方第一次被连时才需要，见 problem.pairingRequired）
  const [pairInput, setPairInput] = useState("");

  // 画质用"设置 → 画质"里的预设（默认均衡）；连上之后还能在会话工具栏里单独调
  const connect = (code6 = digits, pair = "") => {
    if (!/^\d{6}$/.test(code6)) return;
    setError(null);
    void openSession(code6, defaultQuality(), { pairing: pair || null }).catch((e) => setError(e as Problem));
  };

  const share = () => {
    const text = code ? `${code}${pairing ? ` ${pairing}` : ""}` : "";
    if (!text) return;
    void navigator.clipboard?.writeText(text.replace(/\s/g, "")).then(
      () => setCopied(true),
      () => {},
    );
  };

  const [pairError, setPairError] = useState<Problem | null>(null);
  const savePairing = async (e: React.FormEvent) => {
    e.preventDefault();
    setSaving(true);
    setPairError(null);
    try {
      await api.savePairing(draft);
      setEditing(false);
    } catch (err) {
      setPairError(err as Problem);
    } finally {
      setSaving(false);
    }
  };

  /** 改本机设置时禁用一下开关，回来更新状态 */
  const withBusy = async (work: Promise<typeof profile>) => {
    setSaving(true);
    try {
      store.setProfile(await work);
    } finally {
      setSaving(false);
    }
  };

  /** 最近连接也用"设置 → 画质"里的预设 */
  const knownQuality = () => defaultQuality();

  return (
    <div className="assist-page">
      <h1>{t("assist.title")}</h1>

      {/* 本设备 */}
      <section className="as-card">
        <div className="as-card-head">
          <span className="as-card-title">{t("assist.thisDevice")}</span>
          <span className="as-toggle-wrap">
            {t("assist.allow")}
            <button
              className={`as-switch ${profile.allowAssist ? "on" : ""}`}
              role="switch"
              aria-checked={profile.allowAssist}
              aria-label={t("assist.allow")}
              disabled={saving}
              onClick={() =>
                void withBusy(api.profileSetAllow(!profile.allowAssist)).catch((e) => setError(e as Problem))
              }
            >
              <span className="knob" />
            </button>
          </span>
        </div>
        <div className="as-body">
          <div className="as-col">
            <div className="as-label">{t("assist.deviceId")}</div>
            <div className="as-value">{code ? fmtCode(code) : "— — —"}</div>
          </div>
          <div className="as-col wide">
            <div className="as-label-row">
              <span className="as-label">{t("assist.verify")}</span>
              <button
                className="as-inline-select"
                aria-expanded={verifyOpen}
                disabled={saving}
                onClick={() => setVerifyOpen((v) => !v)}
              >
                {t(`assist.verify.${profile.verify}`)}
                <IconChevron size={13} />
              </button>
              {verifyOpen && (
                <span className="as-verify-menu">
                  {(["pairing", "approval", "open"] as const).map((v) => (
                    <button
                      key={v}
                      className={profile.verify === v ? "on" : ""}
                      onClick={() => {
                        setVerifyOpen(false);
                        if (profile.verify !== v) {
                          void withBusy(api.profileSetVerify(v)).catch((e) => setError(e as Problem));
                        }
                      }}
                    >
                      {t(`assist.verify.${v}`)}
                      {profile.verify === v && <IconCheck size={13} />}
                    </button>
                  ))}
                </span>
              )}
            </div>
            <div className="as-value">{pairing ? (shown ? pairing : "••••••") : t("assist.noPairing")}</div>
            <div className="as-sub">{t("assist.verifyHint")}</div>
            {editing && (
              <form className="as-pair-edit" onSubmit={savePairing}>
                <input
                  className="as-input mono"
                  value={draft}
                  onChange={(e) => setDraft(e.target.value.replace(/\D/g, "").slice(0, 6))}
                  inputMode="numeric"
                  maxLength={6}
                  autoFocus
                  aria-label={t("host.pairingInput")}
                  disabled={saving}
                />
                <button type="submit" className="as-connect small" disabled={busy || draft.length !== 6}>
                  {t("host.pairingSave")}
                </button>
                <button type="button" className="as-share" onClick={() => setEditing(false)} disabled={saving}>
                  {t("host.pairingCancel")}
                </button>
                {pairError && <div className="error-box left">{problemText(pairError)}</div>}
              </form>
            )}
          </div>
          <div className="as-actions">
            <button
              className="as-icon"
              title={shown ? t("assist.hide") : t("assist.show")}
              aria-label={shown ? t("assist.hide") : t("assist.show")}
              onClick={() => setShown((v) => !v)}
            >
              {shown ? <IconEye size={18} /> : <IconEyeOff size={18} />}
            </button>
            <button
              className={`as-icon ${editing ? "on" : ""}`}
              title={t("host.pairingEdit")}
              aria-label={t("host.pairingEdit")}
              onClick={() => (setDraft(pairing ?? ""), setEditing((v) => !v))}
            >
              <IconSettings size={18} />
            </button>
            <button className="as-share" onClick={share} disabled={!code}>
              {copied ? <IconCheck size={15} /> : <IconCopy size={15} />}
              {t(copied ? "assist.copied" : "assist.share")}
            </button>
          </div>
        </div>
      </section>

      {/* 远控伙伴设备 */}
      <section className="as-card">
        <div className="as-card-head column">
          <span className="as-card-title">{t("assist.partner")}</span>
          <span className="as-card-sub">{t("assist.partnerHint")}</span>
        </div>
        <div className="as-body column">
          <div className="as-col">
            <div className="as-label">{t("assist.partnerId")}</div>
            <div className="as-input-row">
              <input
                className="as-input"
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && connect()}
                placeholder={t("assist.partnerPlaceholder")}
                inputMode="numeric"
                maxLength={7}
                aria-label={t("assist.partnerPlaceholder")}
              />
              <button className="as-connect" disabled={!ready || busy} onClick={() => connect()}>
                {busy ? <span className="spinner" /> : null}
                {t(busy ? "assist.connecting" : "assist.connect")}
              </button>
            </div>
            {/* 对方要配对码：就地输完再连，不跳页面 */}
            {needsPairing && !busy && (
              <form
                className="as-pair-edit"
                onSubmit={(e) => {
                  e.preventDefault();
                  connect(digits, pairInput);
                }}
              >
                <input
                  className="as-input mono"
                  value={pairInput}
                  onChange={(e) => setPairInput(e.target.value.replace(/\D/g, "").slice(0, 6))}
                  placeholder={t("assist.partnerPairing")}
                  inputMode="numeric"
                  maxLength={6}
                  autoFocus
                  aria-label={t("assist.partnerPairing")}
                />
                <button type="submit" className="as-connect small" disabled={!/^\d{6}$/.test(pairInput)}>
                  {t("assist.connectPaired")}
                </button>
                <div className="as-sub">{t("assist.partnerPairingHint")}</div>
              </form>
            )}
            {problem && !needsPairing && <div className="error-box left">{problemText(problem)}</div>}
            {error && <div className="error-box left">{problemText(error)}</div>}
          </div>

          <div className="as-recent">
            <div className="as-recent-head">
              <span className="as-recent-title">{t("assist.recent")}</span>
              <button
                className="as-clear"
                disabled={recent.length === 0}
                onClick={() => void api.recentDrop().then((p) => store.setProfile(p), () => {})}
              >
                <IconTrash size={14} />
                {t("assist.clearRecent")}
              </button>
            </div>
            {recent.length > 0 && (
              <div className="as-chips">
                {recent.map((r) => {
                  const starred = profile.favorites.includes(r.code);
                  return (
                    <span key={r.code} className="as-chip">
                      <button
                        className={`as-chip-star ${starred ? "on" : ""}`}
                        title={t("assist.star")}
                        onClick={() => void api.favoriteToggle(r.code).then((p) => store.setProfile(p), () => {})}
                      >
                        <IconStar size={13} filled={starred} />
                      </button>
                      <button
                        className="as-chip-code"
                        title={r.name ? `${r.name} · ${fmtCode(r.code)}` : fmtCode(r.code)}
                        onClick={() => void openSession(r.code, knownQuality())}
                      >
                        {r.code}
                      </button>
                      <button
                        className="as-chip-x"
                        title={t("assist.drop")}
                        onClick={() => void api.recentDrop(r.code).then((p) => store.setProfile(p), () => {})}
                      >
                        ✕
                      </button>
                    </span>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      </section>
    </div>
  );
}
