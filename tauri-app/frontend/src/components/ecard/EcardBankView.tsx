import { CreditCard, Eye } from "lucide-react";
import { useState } from "react";
import { Surface } from "@/components/Surface";
import { SecureKeypad, type KeypadInput } from "@/components/ecard/SecureKeypad";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { invokeCommand } from "@/shared/tauriApi";
import type { EcardCard, EcardCheckResult, EcardCodeSent } from "@/shared/types";

/**
 * 银行卡子页（M4.5 批 3）：绑定 / 解绑 / 查看卡号。
 *
 * - 已绑定显示「尾号 xxxx」（`bankaccTail`）；未绑定走绑定流程：
 *   卡号 → `ecard_send_bind_bank_code`（本校 `specialversion=0`，`bankacc` 不随发码带上）
 *   → 短信验证码 → `SecureKeypad`（查询密码）→ `ecard_bind_bank`。
 * - 「查看卡号」：`SecureKeypad` → `ecard_check_pwd` → 通过后展示返回的 `bankCardNo`。
 *   **本校后端恒返回 null**（学校未提供该能力）——此时如实提示「学校未返回卡号」，
 *   绝不伪造号码、不显示脱敏假数据。
 * - 本校 `enabledApps` 清单没有 `bind-campus-card` ⇒ 不做多卡绑定/解绑入口
 *   （后端 `ecard_bind_user`/`ecard_unbind_user` 存在但本页不渲染）。
 * - `account` 一律不传（后端解析「当前卡」）；密码只走 `padId + positions`。
 */

/** 银行卡号：数字、去空格后 12–19 位。 */
const BANKACC_RE = /^\d{12,19}$/;

export function EcardBankView({
  phase,
  card,
  error,
  onRetry,
  onChanged,
}: {
  phase: "loading" | "ready" | "error";
  card: EcardCard | null;
  error: string;
  onRetry: () => void;
  onChanged: () => void;
}) {
  if (phase === "loading") {
    return (
      <Surface className="px-4 py-4">
        <div aria-hidden>
          <div className="h-4 w-32 animate-pulse rounded bg-line" />
          <div className="mt-3 h-8 w-2/3 animate-pulse rounded bg-line" />
        </div>
      </Surface>
    );
  }

  if (phase === "error" || !card) {
    return (
      <Surface accent="wallet" className="flex items-center justify-between gap-3 px-4 py-3">
        <p className="min-w-0 text-body text-text-2">
          {phase === "error" ? `获取失败：${error}` : "没有可用校园卡"}
        </p>
        {phase === "error" && (
          <Button variant="outline" size="sm" className="shrink-0" onClick={onRetry}>
            重试
          </Button>
        )}
      </Surface>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <BankBindSection card={card} onChanged={onChanged} />
      <RevealCardNoSection />
    </div>
  );
}

// ---------------- 绑定 / 解绑 ----------------

function BankBindSection({
  card,
  onChanged,
}: {
  card: EcardCard;
  onChanged: () => void;
}) {
  const bound = card.bankaccTail !== "";
  const [confirming, setConfirming] = useState(false);
  /** 绑定流程：closed → input（卡号+验证码）→ pad（查询密码键盘） */
  const [flow, setFlow] = useState<"closed" | "input" | "pad">("closed");
  const [bankacc, setBankacc] = useState("");
  const [vercode, setVercode] = useState("");
  const [codeId, setCodeId] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [err, setErr] = useState("");
  const [padErr, setPadErr] = useState("");

  const digits = bankacc.replace(/\s+/g, "");
  const bankaccOk = BANKACC_RE.test(digits);

  const sendCode = async () => {
    setBusy(true);
    setErr("");
    setMsg("");
    const r = await invokeCommand<EcardCodeSent>("ecard_send_bind_bank_code");
    setBusy(false);
    if (r.success && r.data) {
      setCodeId(r.data.id);
      setMsg("验证码已发送，请查收短信后输入。");
    } else {
      setErr(r.message ?? "发送验证码失败");
    }
  };

  const doBind = async (v: KeypadInput) => {
    setBusy(true);
    setErr("");
    setMsg("");
    const r = await invokeCommand("ecard_bind_bank", {
      bankacc: digits,
      vercode: vercode.trim(),
      id: codeId,
      padId: v.padId,
      positions: v.positions,
    });
    setBusy(false);
    if (r.success) {
      setFlow("closed");
      setBankacc("");
      setVercode("");
      setCodeId("");
      setMsg("已提交：银行卡绑定成功。");
      onChanged();
    } else {
      // 键盘已被这次提交消耗：错误交给键盘展示，用户重新获取后再输
      setPadErr(r.message ?? "绑定失败");
    }
  };

  const doUnbind = async () => {
    setBusy(true);
    setErr("");
    setMsg("");
    const r = await invokeCommand("ecard_cancel_bank");
    setBusy(false);
    if (r.success) {
      setConfirming(false);
      setMsg("已提交：银行卡已解绑。");
      onChanged();
    } else {
      setErr(r.message ?? "解绑失败");
    }
  };

  return (
    <Surface className="px-4 py-4">
      <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
        <p className="text-body font-medium text-text">银行卡绑定</p>
        <span className="inline-flex items-center gap-1 text-caption text-text-2">
          <CreditCard aria-hidden className="size-3.5" />
          用于卡账户与银行卡之间的转账圈存
        </span>
      </div>
      <p className="mt-2 text-body text-text">
        {bound ? `已绑定 · 尾号 ${card.bankaccTail}` : "未绑定银行卡"}
      </p>

      {msg && (
        <p className="mt-2 text-caption text-text-2" role="status">
          {msg}
        </p>
      )}
      {err && <p className="mt-2 text-caption text-alert">{err}</p>}

      {bound ? (
        confirming ? (
          <div className="mt-3 rounded-inner border border-alert/30 bg-alert/5 px-3 py-3">
            <p className="text-body font-medium text-alert">确认解绑这张银行卡？</p>
            <p className="mt-1 text-caption text-text-2">
              解绑后需重新绑定才能继续使用银行卡相关的转账功能。
            </p>
            <div className="mt-2.5 flex flex-wrap items-center gap-2">
              <Button variant="destructive" size="sm" disabled={busy} onClick={() => void doUnbind()}>
                确认解绑
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={busy}
                onClick={() => setConfirming(false)}
              >
                取消
              </Button>
            </div>
          </div>
        ) : (
          <div className="mt-3">
            <Button
              variant="outline"
              size="sm"
              className="border-alert/40 text-alert hover:bg-alert/5"
              disabled={busy}
              onClick={() => setConfirming(true)}
            >
              解绑
            </Button>
          </div>
        )
      ) : flow === "input" ? (
        <>
          <div className="mt-3">
            <label className="mb-1 block text-caption text-text-2" htmlFor="ecard-bankacc">
              银行卡号（12–19 位数字）
            </label>
            <Input
              id="ecard-bankacc"
              className="tabular-num max-w-[16rem]"
              value={bankacc}
              inputMode="numeric"
              autoComplete="off"
              disabled={busy || codeId !== ""}
              aria-invalid={bankacc !== "" && !bankaccOk}
              onChange={(e) => setBankacc(e.target.value)}
            />
            {bankacc !== "" && !bankaccOk && (
              <p className="mt-1 text-caption text-alert">卡号须为 12–19 位数字</p>
            )}
          </div>
          <div className="mt-3">
            {codeId === "" ? (
              <Button
                size="sm"
                disabled={busy || !bankaccOk}
                onClick={() => void sendCode()}
              >
                发送验证码
              </Button>
            ) : (
              <>
                <label
                  className="mb-1 block text-caption text-text-2"
                  htmlFor="ecard-bind-vercode"
                >
                  短信验证码
                </label>
                <Input
                  id="ecard-bind-vercode"
                  className="max-w-[10rem]"
                  value={vercode}
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  disabled={busy}
                  onChange={(e) => setVercode(e.target.value)}
                />
              </>
            )}
          </div>
          {codeId !== "" && vercode.trim() !== "" && (
            <div className="mt-3">
              <SecureKeypad
                kind="number"
                title="输入查询密码完成绑定"
                busy={busy}
                error={padErr}
                onDone={(v) => void doBind(v)}
                onCancel={() => {
                  setFlow("closed");
                  setPadErr("");
                }}
              />
            </div>
          )}
          <div className="mt-2">
            <Button
              variant="ghost"
              size="sm"
              disabled={busy}
              onClick={() => {
                setFlow("closed");
                setPadErr("");
                setErr("");
              }}
            >
              取消绑定
            </Button>
          </div>
        </>
      ) : (
        <div className="mt-3">
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => {
              setErr("");
              setMsg("");
              setFlow("input");
            }}
          >
            绑定银行卡
          </Button>
        </div>
      )}
    </Surface>
  );
}

// ---------------- 查看卡号 ----------------

function RevealCardNoSection() {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [padErr, setPadErr] = useState("");
  /** 校验通过后的结果：卡号或「学校未返回」；null = 还没查 */
  const [result, setResult] = useState<string | null>(null);
  const [schoolNoCardNo, setSchoolNoCardNo] = useState(false);

  const check = async (v: KeypadInput) => {
    setBusy(true);
    setPadErr("");
    const r = await invokeCommand<EcardCheckResult>("ecard_check_pwd", {
      padId: v.padId,
      positions: v.positions,
    });
    setBusy(false);
    if (r.success && r.data) {
      if (r.data.bankCardNo) {
        setResult(r.data.bankCardNo);
        setSchoolNoCardNo(false);
      } else {
        // 本校后端恒 null（学校未提供校验密码查卡号能力）：如实提示，不伪造号码
        setResult(null);
        setSchoolNoCardNo(true);
      }
    } else {
      setPadErr(r.message ?? "查询密码校验失败");
    }
  };

  return (
    <Surface className="px-4 py-4">
      <p className="text-body font-medium text-text">查看银行卡号</p>
      <p className="mt-1 text-caption text-text-2">
        需先通过查询密码校验；学校系统返回什么就显示什么。
      </p>

      {open ? (
        <div className="mt-3">
          <SecureKeypad
            kind="number"
            title="输入查询密码以查看卡号"
            busy={busy}
            error={padErr}
            onDone={(v) => void check(v)}
            onCancel={() => {
              setOpen(false);
              setPadErr("");
            }}
          />
        </div>
      ) : (
        <div className="mt-3">
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => {
              setResult(null);
              setSchoolNoCardNo(false);
              setOpen(true);
            }}
          >
            <Eye aria-hidden className="size-3.5" />
            查看卡号
          </Button>
        </div>
      )}

      {result !== null && (
        <p className="tabular-num mt-3 text-body font-medium text-text">
          卡号：{result}
        </p>
      )}
      {schoolNoCardNo && (
        <p className="mt-3 text-caption text-text-2" role="status">
          查询密码校验通过，但学校未返回卡号。
        </p>
      )}
    </Surface>
  );
}
