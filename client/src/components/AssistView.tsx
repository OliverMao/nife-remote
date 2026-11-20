// 远程协助页（照 UU 的"远程协助"）：本设备的 ID / 验证码 + 复制分享，下面用伙伴的设备 ID 直连另一台电脑。
//
// 数据来源：本设备ID = 本机连接码，验证码 = 配对码（都是真的，来自被控端状态）；
// "允许他人远程协助"的开关、验证方式下拉、星标收藏还没有功能，先摆着。
import { useEffect, useState } from "react";
import { api, type Problem } from "../lib/api";
import { fmtCode, openSession } from "../lib/actions";
import { bestCodec } from "../lib/codec";
import { useT } from "../lib/i18n";
import { problemText } from "../lib/messages";
import { DEFAULT_PRESET, presetSettings } from "../lib/presets";
import { store, useStore } from "../lib/store";
import { IconCheck, IconChevron, IconCopy, IconEye, IconEyeOff, IconSettings, IconStar, IconTrash } from "./Icons";

export function AssistView() {
  const t = useT();
  const host = useStore((s) => s.host);
  const profile = useStore((s) => s.profile);
  const recent = profile.recent;
  const devices = useStore((s) => s.devices);
  const decoders = useStore((s) => s.decoders);
  const [copied, setCopied] = useState(false);
  const [shown, setShown] = useState(true);
  const [busy, setBusy] = useState(false);
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
  const known = devices.find((d) => d.code === digits);

  const connect = () => {
    if (!ready) return;
    setError(null);
    void openSession(digits, known?.quality ?? presetSettings(DEFAULT_PRESET, bestCodec(decoders))).catch((e) =>
      setError(e as Problem),
    );
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
    setBusy(true);
    setPairError(null);
    try {
      await api.savePairing(draft);
      setEditing(false);
    } catch (err) {
      setPairError(err as Problem);
    } finally {
      setBusy(false);
    }
  };

  /** 改本机设置时禁用一下开关，回来更新状态 */
  const withBusy = async (work: Promise<typeof profile>) => {
    setBusy(true);
    try {
      store.setProfile(await work);
    } finally {
      setBusy(false);
    }
  };

  /** 最近连接里的设备用上次的画质，没连过的用默认画质 */
  const knownQuality = (c: string) =>
    devices.find((d) => d.code === c)?.quality ?? presetSettings(DEFAULT_PRESET, bestCodec(decoders));

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
              disabled={busy}
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
                disabled={busy}
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
                  disabled={busy}
                />
                <button type="submit" className="as-connect small" disabled={busy || draft.length !== 6}>
                  {t("host.pairingSave")}
                </button>
                <button type="button" className="as-share" onClick={() => setEditing(false)} disabled={busy}>
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
              <button className="as-connect" disabled={!ready} onClick={connect}>
                {t("assist.connect")}
              </button>
            </div>
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
                        onClick={() => void openSession(r.code, knownQuality(r.code))}
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
