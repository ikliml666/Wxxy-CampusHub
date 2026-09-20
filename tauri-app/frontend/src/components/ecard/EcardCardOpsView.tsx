import { AlertTriangle, ShieldAlert } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { Surface } from "@/components/Surface";
import { SecureKeypad, type KeypadInput } from "@/components/ecard/SecureKeypad";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/shared/cn";
import { invokeCommand } from "@/shared/tauriApi";
import type { EcardCard, EcardClientConfig, EcardCodeSent } from "@/shared/types";

/**
 * 卡设置子页（M4.5 批 3）：挂失·解挂 / 修改查询密码 / 免密与限额 / 自动转账（圈存）。
 *
 * # 写操作协议（计划 §1.8 / §2.2）
 *
 * - **`account` 一律不传**：卡号原号不暴露给前端（DTO 只有 `accountMasked`），
 *   命令的 `account` 由后端解析「当前卡」。
 * - **密码只走 `padId + positions`**：所有密码输入经 `SecureKeypad`，提交的永远是
 *   位置下标序列；明文不进前端状态、错误文案与 localStorage。
 * - **危险操作二次确认**：挂失在页内做确认区（后果说清），不弹浏览器 confirm。
 * - **成败由后端双层判定**（`code==200 && retcode==="0"`），失败时 `CommandResult.message`
 *   即学校可读原因，原样透出；成功就地内联「已提交」并 `onChanged()` 让容器刷新概览。
 * - 挂失分区受 `config.showLost` 门控（showLost === false 不渲染）。
 */

/** 金额形态（≥0、至多两位小数）。金额一律字符串 state + 校验，不做浮点累加。 */
const YUAN_RE = /^\d+(\.\d{1,2})?$/;

/** 空串视为 0（0 = 未设置，是合法提交值）。 */
function yuanNum(s: string): number {
  const t = s.trim() === "" ? "0" : s.trim();
  return Number(t);
}

function yuanText(v: number): string {
  return v > 0 ? `¥ ${v.toFixed(2)}` : "未设置";
}

/** 内联结果行：成功一句「已提交」提示，失败透出学校原因（不弹 alert）。 */
function OpLine({ ok, text }: { ok: boolean; text: string }) {
  return (
    <p
      className={cn("mt-2 text-caption", ok ? "text-text-2" : "text-alert")}
      role="status"
    >
      {text}
    </p>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <Surface className="px-4 py-4">
      <p className="text-body font-medium text-text">{title}</p>
      <div className="mt-3">{children}</div>
    </Surface>
  );
}

/** 两次新密码一致性判定：两把键盘布局指纹一致才可在本地比对（同布局 + 同下标 ⇒ 同密码）。
 *  布局不同时本地无从判定（明文不进前端），交由学校服务端校验。 */
function positionsComparable(a: KeypadInput, b: KeypadInput): boolean {
  return a.keysFingerprint === b.keysFingerprint;
}

// ---------------- 挂失 · 解挂 ----------------

function LostSection({
  card,
  onChanged,
}: {
  card: EcardCard;
  onChanged: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [err, setErr] = useState("");
  /** 解挂的查询密码键盘 */
  const [keypadOpen, setKeypadOpen] = useState(false);
  const [padErr, setPadErr] = useState("");

  /** 挂失（免密，无键盘）。⚠️ 后端会立即冻结卡片——前端确认区必须先走完。 */
  const doLost = async () => {
    setBusy(true);
    setErr("");
    setMsg("");
    const r = await invokeCommand("ecard_lost");
    setBusy(false);
    if (r.success) {
      setConfirming(false);
      setMsg("已提交：卡片已挂失，恢复使用请回到本页解挂。");
      onChanged();
    } else {
      setErr(r.message ?? "挂失失败");
    }
  };

  /** 解挂（需查询密码；padId/positions 成对提交）。 */
  const doUnlost = async (v: KeypadInput) => {
    setBusy(true);
    setErr("");
    setMsg("");
    const r = await invokeCommand("ecard_unlost", {
      padId: v.padId,
      positions: v.positions,
    });
    setBusy(false);
    if (r.success) {
      setKeypadOpen(false);
      setMsg("已提交：卡片已解挂。");
      onChanged();
    } else {
      // 键盘已被这次提交消耗：错误交给键盘展示，用户点「重新获取键盘」再输
      setPadErr(r.message ?? "解挂失败");
    }
  };

  return (
    <Section title="挂失 · 解挂">
      <p className="text-caption text-text-2">
        当前状态：
        {card.lost ? (
          <span className="font-medium text-todo">已挂失</span>
        ) : (
          <span className="font-medium text-text">正常</span>
        )}
      </p>

      {msg && <OpLine ok text={msg} />}
      {err && <OpLine ok={false} text={err} />}

      {!card.lost ? (
        confirming ? (
          /* 页内确认区：后果说清，不用 window.confirm */
          <div className="mt-3 rounded-inner border border-alert/30 bg-alert/5 px-3 py-3">
            <p className="flex items-start gap-1.5 text-body font-medium text-alert">
              <ShieldAlert aria-hidden className="mt-0.5 size-4 shrink-0" />
              确认挂失这张卡？
            </p>
            <ul className="mt-1.5 space-y-1 text-caption text-text-2">
              <li>· 挂失后该卡立即不能消费，解挂需要查询密码。</li>
              <li>· 挂失由学校系统立即生效，本操作不可由客户端撤销。</li>
            </ul>
            <div className="mt-2.5 flex flex-wrap items-center gap-2">
              <Button variant="destructive" size="sm" disabled={busy} onClick={() => void doLost()}>
                确认挂失
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
              挂失此卡
            </Button>
          </div>
        )
      ) : keypadOpen ? (
        <div className="mt-3">
          <SecureKeypad
            kind="standard"
            title="输入查询密码以解挂"
            busy={busy}
            error={padErr}
            onDone={(v) => void doUnlost(v)}
            onCancel={() => {
              setKeypadOpen(false);
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
              setPadErr("");
              setKeypadOpen(true);
            }}
          >
            解挂（需查询密码）
          </Button>
        </div>
      )}
    </Section>
  );
}

// ---------------- 修改查询密码 ----------------

const PWD_STEPS = ["旧密码", "新密码", "确认新密码"] as const;
const RESET_STEPS = ["新密码", "确认新密码"] as const;

function PwdChangeSection({ onChanged }: { onChanged: () => void }) {
  /** change = 凭旧密码修改；reset = 忘记旧密码，凭短信验证码找回 */
  const [mode, setMode] = useState<"change" | "reset">("change");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [err, setErr] = useState("");
  const [padErr, setPadErr] = useState("");

  /** change 模式：旧 / 新 / 确认 三段依次采集，收集完一次性提交 */
  const [inputs, setInputs] = useState<(KeypadInput | null)[]>([null, null, null]);
  /** 是否已点「修改密码」开始（进页面不主动弹密码键盘——用户可能只是来看限额/圈存的） */
  const [started, setStarted] = useState(false);
  /** reset 模式：验证码 + 新 / 确认 两段 */
  const [resetId, setResetId] = useState("");
  const [vercode, setVercode] = useState("");
  const [resetInputs, setResetInputs] = useState<(KeypadInput | null)[]>([
    null,
    null,
  ]);

  const step = inputs.findIndex((x) => x === null);
  const resetStep = resetInputs.findIndex((x) => x === null);

  /** 两次新密码不一致时的就地报错（不发请求） */
  const mismatch = (a: KeypadInput, b: KeypadInput): boolean =>
    positionsComparable(a, b) && a.positions.join(",") !== b.positions.join(",");

  const submitChange = async (
    oldV: KeypadInput,
    newV: KeypadInput,
    renewV: KeypadInput,
  ) => {
    if (mismatch(newV, renewV)) {
      setErr("两次输入的新密码不一致，请重新输入新密码两段");
      // 保留已验证语义的旧密码段只需重输新密；旧键盘已随提交消耗，仍需重输
      setInputs([null, null, null]);
      return;
    }
    setBusy(true);
    setErr("");
    setMsg("");
    const r = await invokeCommand("ecard_modify_pwd", {
      oldPadId: oldV.padId,
      oldPositions: oldV.positions,
      newPadId: newV.padId,
      newPositions: newV.positions,
      renewPadId: renewV.padId,
      renewPositions: renewV.positions,
    });
    setBusy(false);
    if (r.success) {
      setMsg("已提交：查询密码已修改。");
      setInputs([null, null, null]);
      onChanged();
    } else {
      // 三把键盘都随这次提交消耗，全部重输
      setPadErr(r.message ?? "修改密码失败");
      setInputs([null, null, null]);
    }
  };

  const submitReset = async (newV: KeypadInput, renewV: KeypadInput) => {
    if (vercode.trim() === "") {
      setErr("请先输入短信验证码");
      setResetInputs([null, null]);
      return;
    }
    if (mismatch(newV, renewV)) {
      setErr("两次输入的新密码不一致，请重新输入新密码两段");
      setResetInputs([null, null]);
      return;
    }
    setBusy(true);
    setErr("");
    setMsg("");
    const r = await invokeCommand("ecard_find_pwd", {
      newPadId: newV.padId,
      newPositions: newV.positions,
      renewPadId: renewV.padId,
      renewPositions: renewV.positions,
      vercode: vercode.trim(),
      id: resetId,
    });
    setBusy(false);
    if (r.success) {
      setMsg("已提交：查询密码已重置，请用新密码。");
      setResetInputs([null, null]);
      setVercode("");
      setResetId("");
      setMode("change");
      onChanged();
    } else {
      setPadErr(r.message ?? "重置密码失败");
      setResetInputs([null, null]);
    }
  };

  const sendCode = async () => {
    setBusy(true);
    setErr("");
    setMsg("");
    const r = await invokeCommand<EcardCodeSent>("ecard_send_find_pwd_code");
    setBusy(false);
    if (r.success && r.data) {
      setResetId(r.data.id);
      setMsg("验证码已发送，请查收短信后输入。");
    } else {
      setErr(r.message ?? "发送验证码失败");
    }
  };

  const resetAll = () => {
    setInputs([null, null, null]);
    setResetInputs([null, null]);
    setVercode("");
    setResetId("");
    setErr("");
    setPadErr("");
    setMsg("");
    setStarted(false);
  };

  return (
    <Section title="修改查询密码">
      <p className="text-caption text-text-2">
        查询密码为 6 位（默认身份证后六位，可含字母与符号），密码键盘点满自动进入下一步。
      </p>

      {msg && <OpLine ok text={msg} />}
      {err && <OpLine ok={false} text={err} />}

      {mode === "change" ? (
        <>
          {!started ? (
            <div className="mt-3">
              <Button size="sm" disabled={busy} onClick={() => setStarted(true)}>
                修改密码
              </Button>
            </div>
          ) : step >= 0 ? (
            <div className="mt-3">
              <SecureKeypad
                kind="standard"
                title={`请输入${PWD_STEPS[step]}`}
                busy={busy}
                error={padErr}
                onDone={(v) => {
                  setPadErr("");
                  const next = [...inputs];
                  next[step] = v;
                  setInputs(next);
                  if (step === 2) {
                    void submitChange(next[0]!, next[1]!, next[2]!);
                  }
                }}
                onCancel={resetAll}
              />
            </div>
          ) : (
            <p className="mt-3 text-caption text-text-2" aria-busy={busy}>
              {busy ? "正在提交…" : "三段密码已采集，正在提交…"}
            </p>
          )}
          <div className="mt-2">
            <Button
              variant="link"
              size="sm"
              className="h-auto p-0 text-caption"
              disabled={busy}
              onClick={() => {
                setMode("reset");
                resetAll();
              }}
            >
              忘记旧密码？用短信验证码找回
            </Button>
          </div>
        </>
      ) : (
        <>
          {resetId === "" ? (
            <div className="mt-3">
              <Button size="sm" disabled={busy} onClick={() => void sendCode()}>
                发送验证码
              </Button>
            </div>
          ) : (
            <>
              <div className="mt-3">
                <label
                  className="mb-1 block text-caption text-text-2"
                  htmlFor="ecard-find-pwd-vercode"
                >
                  短信验证码
                </label>
                <Input
                  id="ecard-find-pwd-vercode"
                  className="max-w-[10rem]"
                  value={vercode}
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  disabled={busy}
                  onChange={(e) => setVercode(e.target.value)}
                />
              </div>
              {resetStep >= 0 ? (
                <div className="mt-3">
                  <SecureKeypad
                    kind="standard"
                    title={`请输入${RESET_STEPS[resetStep]}`}
                    busy={busy}
                    error={padErr}
                    onDone={(v) => {
                      setPadErr("");
                      const next = [...resetInputs];
                      next[resetStep] = v;
                      setResetInputs(next);
                      if (resetStep === 1) {
                        void submitReset(next[0]!, next[1]!);
                      }
                    }}
                    onCancel={() => {
                      setMode("change");
                      resetAll();
                    }}
                  />
                </div>
              ) : (
                <p className="mt-3 text-caption text-text-2" aria-busy={busy}>
                  {busy ? "正在提交…" : "两段新密码已采集，正在提交…"}
                </p>
              )}
            </>
          )}
          <div className="mt-2">
            <Button
              variant="link"
              size="sm"
              className="h-auto p-0 text-caption"
              disabled={busy}
              onClick={() => {
                setMode("change");
                resetAll();
              }}
            >
              返回凭旧密码修改
            </Button>
          </div>
        </>
      )}
    </Section>
  );
}

// ---------------- 免密与限额 ----------------

function LimitsSection({
  card,
  onChanged,
}: {
  card: EcardCard;
  onChanged: () => void;
}) {
  const acc = card.accInfos[0];
  const [daycost, setDaycost] = useState(String(card.dayCostLimitYuan));
  const [nonpwd, setNonpwd] = useState(String(card.nonpwdLimitYuan));
  const [single, setSingle] = useState(String(card.singleLimitYuan));
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [err, setErr] = useState("");

  // 概览刷新（onChanged 后容器重取）时同步学校侧最新值
  useEffect(() => {
    setDaycost(String(card.dayCostLimitYuan));
    setNonpwd(String(card.nonpwdLimitYuan));
    setSingle(String(card.singleLimitYuan));
  }, [card]);

  if (!acc) {
    return (
      <Section title="免密与限额">
        <p className="text-caption text-text-2">
          该卡没有可用的电子账户，暂不能设置限额。
        </p>
      </Section>
    );
  }

  const fields = [
    { label: "单日消费限额（元）", value: daycost, set: setDaycost },
    { label: "免密限额（元）", value: nonpwd, set: setNonpwd },
    { label: "单笔限额（元）", value: single, set: setSingle },
  ];
  const invalid = fields.some(
    (f) => f.value.trim() !== "" && !YUAN_RE.test(f.value.trim()),
  );

  const submit = async () => {
    if (invalid) {
      setErr("限额须为不小于 0 的金额，最多两位小数");
      return;
    }
    setBusy(true);
    setErr("");
    setMsg("");
    const r = await invokeCommand("ecard_set_limits", {
      accType: acc.type,
      daycostLimitYuan: yuanNum(daycost),
      nonpwdLimitYuan: yuanNum(nonpwd),
      singleLimitYuan: yuanNum(single),
    });
    setBusy(false);
    if (r.success) {
      setMsg("已提交：限额设置已更新。");
      onChanged();
    } else {
      setErr(r.message ?? "限额设置失败");
    }
  };

  return (
    <Section title="免密与限额">
      <p className="text-caption text-text-2">
        当前：单日消费 {yuanText(card.dayCostLimitYuan)} · 免密{" "}
        {yuanText(card.nonpwdLimitYuan)} · 单笔 {yuanText(card.singleLimitYuan)}
        （填 0 表示不限制）
      </p>
      <div className="mt-3 flex flex-wrap gap-3">
        {fields.map((f) => (
          <div key={f.label}>
            <label
              className="mb-1 block text-caption text-text-2"
              htmlFor={`ecard-limit-${f.label}`}
            >
              {f.label}
            </label>
            <Input
              id={`ecard-limit-${f.label}`}
              className="tabular-num max-w-[8rem]"
              value={f.value}
              inputMode="decimal"
              autoComplete="off"
              disabled={busy}
              aria-invalid={f.value.trim() !== "" && !YUAN_RE.test(f.value.trim())}
              onChange={(e) => f.set(e.target.value)}
            />
          </div>
        ))}
      </div>
      {msg && <OpLine ok text={msg} />}
      {err && <OpLine ok={false} text={err} />}
      <div className="mt-3">
        <Button size="sm" disabled={busy || invalid} onClick={() => void submit()}>
          保存限额
        </Button>
      </div>
    </Section>
  );
}

// ---------------- 自动转账（圈存） ----------------

const AUTOTRANS_OPTIONS = [
  { value: "0", label: "禁止转账" },
  { value: "1", label: "只允许自助转账" },
  { value: "2", label: "自助及自动转账" },
] as const;

function AutotransSection({
  card,
  onChanged,
}: {
  card: EcardCard;
  onChanged: () => void;
}) {
  // 学校侧只返回开/关布尔，无法区分档位 1/2 ⇒ 档位一律由用户明确选择，不猜默认值
  const [flag, setFlag] = useState<"" | "0" | "1" | "2">("");
  const [amt, setAmt] = useState(
    card.autotransAmtYuan > 0 ? String(card.autotransAmtYuan) : "",
  );
  const [limite, setLimite] = useState(
    card.autotransLimiteYuan > 0 ? String(card.autotransLimiteYuan) : "",
  );
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [err, setErr] = useState("");

  useEffect(() => {
    setAmt(card.autotransAmtYuan > 0 ? String(card.autotransAmtYuan) : "");
    setLimite(
      card.autotransLimiteYuan > 0 ? String(card.autotransLimiteYuan) : "",
    );
  }, [card]);

  const amtInvalid = amt.trim() !== "" && !YUAN_RE.test(amt.trim());
  const limiteInvalid = limite.trim() !== "" && !YUAN_RE.test(limite.trim());
  const canSubmit =
    flag !== "" && !amtInvalid && !limiteInvalid &&
    (flag === "0" || (amt.trim() !== "" && Number(amt.trim()) > 0));

  const submit = async () => {
    if (flag === "") {
      setErr("请先选择转账档位");
      return;
    }
    if (flag !== "0" && (amt.trim() === "" || Number(amt.trim()) <= 0)) {
      setErr("开启转账时，每次圈存金额须为大于 0 的金额");
      return;
    }
    setBusy(true);
    setErr("");
    setMsg("");
    const r = await invokeCommand("ecard_set_autotrans", {
      flag: Number(flag),
      amtYuan: flag === "0" ? 0 : yuanNum(amt),
      limiteYuan:
        flag === "2" && limite.trim() !== "" ? yuanNum(limite) : undefined,
    });
    setBusy(false);
    if (r.success) {
      setMsg("已提交：自动转账设置已更新。");
      onChanged();
    } else {
      setErr(r.message ?? "自动转账设置失败");
    }
  };

  return (
    <Section title="自动转账（圈存）">
      <p className="text-caption text-text-2">
        当前：{card.autotransFlag ? "开启" : "关闭"} · 每次圈{" "}
        {yuanText(card.autotransAmtYuan)} · 余额下限{" "}
        {yuanText(card.autotransLimiteYuan)}
        。学校侧只返回开/关，具体档位请选择后提交。
      </p>
      <div className="mt-3 flex flex-wrap items-end gap-3">
        <div>
          <label
            className="mb-1 block text-caption text-text-2"
            htmlFor="ecard-autotrans-flag"
          >
            转账档位
          </label>
          <select
            id="ecard-autotrans-flag"
            className="h-9 rounded-control border border-line bg-surface px-2 text-body text-text disabled:opacity-50"
            value={flag}
            disabled={busy}
            onChange={(e) => setFlag(e.target.value as "" | "0" | "1" | "2")}
          >
            <option value="" disabled>
              请选择档位
            </option>
            {AUTOTRANS_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </div>
        {flag !== "" && flag !== "0" && (
          <div>
            <label
              className="mb-1 block text-caption text-text-2"
              htmlFor="ecard-autotrans-amt"
            >
              每次圈存金额（元）
            </label>
            <Input
              id="ecard-autotrans-amt"
              className="tabular-num max-w-[8rem]"
              value={amt}
              inputMode="decimal"
              autoComplete="off"
              disabled={busy}
              aria-invalid={amtInvalid}
              onChange={(e) => setAmt(e.target.value)}
            />
          </div>
        )}
        {flag === "2" && (
          <div>
            <label
              className="mb-1 block text-caption text-text-2"
              htmlFor="ecard-autotrans-limite"
            >
              余额下限（元，可空）
            </label>
            <Input
              id="ecard-autotrans-limite"
              className="tabular-num max-w-[8rem]"
              value={limite}
              inputMode="decimal"
              autoComplete="off"
              disabled={busy}
              aria-invalid={limiteInvalid}
              onChange={(e) => setLimite(e.target.value)}
            />
          </div>
        )}
      </div>
      {flag !== "" && flag !== "0" && (
        <p className="mt-1.5 flex items-center gap-1 text-caption text-text-2">
          <AlertTriangle aria-hidden className="size-3 shrink-0" />
          提交后学校会在卡余额不足时按此设置自动从银行卡圈存，请确认金额。
        </p>
      )}
      {msg && <OpLine ok text={msg} />}
      {err && <OpLine ok={false} text={err} />}
      <div className="mt-3">
        <Button size="sm" disabled={busy || !canSubmit} onClick={() => void submit()}>
          保存自动转账设置
        </Button>
      </div>
    </Section>
  );
}

// ---------------- 子页入口 ----------------

export function EcardCardOpsView({
  phase,
  card,
  config,
  error,
  onRetry,
  onChanged,
}: {
  phase: "loading" | "ready" | "error";
  /** 当前卡（`overview.cards[0]`）；由容器传入，缺省时本页给空/加载态 */
  card: EcardCard | null;
  config: EcardClientConfig | null;
  error: string;
  onRetry: () => void;
  /** 写操作成功后让容器刷新概览（`get_ecard_overview`） */
  onChanged: () => void;
}) {
  if (phase === "loading") {
    return (
      <div aria-hidden className="flex flex-col gap-3">
        {[0, 1, 2].map((i) => (
          <Surface key={i} className="px-4 py-4">
            <div className="h-4 w-28 animate-pulse rounded bg-line" />
            <div className="mt-3 h-8 w-2/3 animate-pulse rounded bg-line" />
          </Surface>
        ))}
      </div>
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

  const showLost = config?.showLost !== false;

  return (
    <div className="flex flex-col gap-3">
      <p className="text-caption text-text-2">
        当前卡 {card.accountMasked} · {card.statusLabel}
      </p>
      {showLost && <LostSection card={card} onChanged={onChanged} />}
      <PwdChangeSection onChanged={onChanged} />
      <LimitsSection card={card} onChanged={onChanged} />
      <AutotransSection card={card} onChanged={onChanged} />
    </div>
  );
}
