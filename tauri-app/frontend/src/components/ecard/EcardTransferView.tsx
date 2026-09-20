import { ArrowRight, Wallet } from "lucide-react";
import { useEffect, useState } from "react";
import { EmptyState } from "@/components/EmptyState";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { invokeCommand } from "@/shared/tauriApi";
import type { EcardTransferAccount } from "@/shared/types";

/**
 * 卡账户 ↔ 电子账户转账子页（M4.5 批 3）。
 *
 * - 账户列表来自 `get_ecard_transfer_accounts`：`account` 是**原号**（转账必须原样
 *   回传——这是唯一允许前端持有原号的场景，契约 §2.5 明确保留），`payAcc` 是学校侧
 *   账户类型码（`src_acctype`/`dst_acctype` 提交原样回传）。
 * - 其余写操作不同，`ecard_transfer` 不涉及「当前卡」解析，转出/转入都由用户选择。
 * - 金额字符串 state + 两位小数校验，> 0 且不超过转出账户余额（超了就地报错不发请求）；
 *   提交前页内二次确认；成功展示结果卡 + 「再转一笔」，失败透出学校 `message`。
 */

const YUAN_RE = /^\d+(\.\d{1,2})?$/;

const SELECT_CLS =
  "h-9 rounded-control border border-line bg-surface px-2 text-body text-text disabled:opacity-50";

interface TransferDone {
  amountYuan: number;
  srcLabel: string;
  dstLabel: string;
  at: string;
}

export function EcardTransferView({ onChanged }: { onChanged: () => void }) {
  const [list, setList] = useState<EcardTransferAccount[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadErr, setLoadErr] = useState("");

  // ⚠️ 选项的 value 用 `code`（CARD/ACCOUNT）而不是 `account`——本校两账户的
  // `account` 是同一个卡号（42940），value 重复会导致 select 选中后再也换不了。
  const [srcAcc, setSrcAcc] = useState("");
  const [dstAcc, setDstAcc] = useState("");
  const [amount, setAmount] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [done, setDone] = useState<TransferDone | null>(null);

  const load = async () => {
    setLoading(true);
    setLoadErr("");
    const r = await invokeCommand<EcardTransferAccount[]>(
      "get_ecard_transfer_accounts",
    );
    if (r.success && r.data) {
      setList(r.data);
    } else {
      setLoadErr(r.message ?? "转账账户获取失败");
    }
    setLoading(false);
  };

  useEffect(() => {
    void load();
  }, []);

  const src = list?.find((a) => a.code === srcAcc) ?? null;
  const dst = list?.find((a) => a.code === dstAcc) ?? null;

  const amountErr = (() => {
    const s = amount.trim();
    if (s === "") return "";
    if (!YUAN_RE.test(s)) return "金额最多支持两位小数";
    const n = Number(s);
    if (n <= 0) return "转账金额必须大于 0";
    if (src && n > src.balanceYuan) return "转账金额不能超过转出账户余额";
    return "";
  })();

  const canConfirm =
    src !== null && dst !== null && src !== dst && amountErr === "" && amount.trim() !== "";

  const submit = async () => {
    if (!src || !dst) return;
    setBusy(true);
    setErr("");
    const r = await invokeCommand("ecard_transfer", {
      dstAccount: dst.account,
      srcAccount: src.account,
      amountYuan: Number(amount.trim()),
      srcAccType: src.payAcc,
      dstAccType: dst.payAcc,
    });
    setBusy(false);
    if (r.success) {
      setDone({
        amountYuan: Number(amount.trim()),
        srcLabel: src.label,
        dstLabel: dst.label,
        at: new Date().toLocaleString(),
      });
      setAmount("");
      setConfirming(false);
      // 余额已变：让容器刷新概览（宫格余额带、余额子页）
      onChanged();
    } else {
      setConfirming(false);
      const m = r.message ?? "转账失败";
      // 该校服务端对卡间转账四变体（正/反向、整数/小数、新旧端点）均报 400，
      // 官方手机版也无此入口——大概率学校未开通。失败文案后追加可操作说明。
      setErr(
        /操作失败|业务异常/.test(m)
          ? `${m}。多次尝试均失败时，可能是学校未开通卡间转账（官方手机版也无此入口），请改用卡片充值或到校服务终端办理。`
          : m,
      );
    }
  };

  if (loading) {
    return (
      <Surface className="px-4 py-4">
        <div aria-hidden>
          <div className="h-4 w-32 animate-pulse rounded bg-line" />
          <div className="mt-3 space-y-2">
            {[0, 1, 2].map((i) => (
              <div key={i} className="h-8 animate-pulse rounded bg-line" />
            ))}
          </div>
        </div>
      </Surface>
    );
  }

  if (loadErr || !list || list.length === 0) {
    return (
      <Surface accent="wallet" className="px-4 py-4">
        {loadErr ? (
          <div className="flex items-center justify-between gap-3">
            <p className="min-w-0 text-body text-text-2">获取失败：{loadErr}</p>
            <Button variant="outline" size="sm" className="shrink-0" onClick={() => void load()}>
              重试
            </Button>
          </div>
        ) : (
          <EmptyState icon={Wallet} domain="wallet" title="没有可转账的账户" />
        )}
      </Surface>
    );
  }

  if (done) {
    return (
      <Surface accent="wallet" className="px-4 py-5">
        <p className="text-body font-medium text-text">转账已提交。</p>
        <dl className="mt-3 rounded-inner border border-line bg-surface-2 px-3 py-2.5">
          <div className="flex items-baseline justify-between gap-3">
            <dt className="text-caption text-text-2">转出金额</dt>
            <dd className="tabular-num text-body font-medium text-text">
              ¥ {done.amountYuan.toFixed(2)}
            </dd>
          </div>
          <div className="mt-1 flex items-baseline justify-between gap-3">
            <dt className="text-caption text-text-2">转出账户</dt>
            <dd className="text-body text-text">{done.srcLabel}</dd>
          </div>
          <div className="mt-1 flex items-baseline justify-between gap-3">
            <dt className="text-caption text-text-2">转入账户</dt>
            <dd className="text-body text-text">{done.dstLabel}</dd>
          </div>
          <div className="mt-1 flex items-baseline justify-between gap-3">
            <dt className="text-caption text-text-2">完成时间</dt>
            <dd className="tabular-num text-caption text-text-2">{done.at}</dd>
          </div>
        </dl>
        <div className="mt-3">
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setDone(null);
              setErr("");
            }}
          >
            再转一笔
          </Button>
        </div>
      </Surface>
    );
  }

  return (
    <Surface accent="wallet" className="px-4 py-4">
      <p className="text-body font-medium text-text">账户转账</p>
      <p className="mt-1 text-caption text-text-2">
        在卡账户与电子账户之间划转余额；到账以学校系统为准。
      </p>

      <div className="mt-3 flex flex-wrap items-end gap-3">
        <div>
          <label className="mb-1 block text-caption text-text-2" htmlFor="transfer-src">
            转出账户
          </label>
          <select
            id="transfer-src"
            className={SELECT_CLS}
            value={srcAcc}
            onChange={(e) => {
              setSrcAcc(e.target.value);
              setConfirming(false);
            }}
          >
            <option value="" disabled>
              选择转出账户
            </option>
            {list
              .filter((a) => a.canTransferOut && !a.lostFlag)
              .map((a) => (
                <option key={a.code} value={a.code}>
                  {a.label}（余额 ¥ {a.balanceYuan.toFixed(2)}）
                </option>
              ))}
          </select>
        </div>
        <ArrowRight aria-hidden className="mb-2 size-4 shrink-0 text-text-2" />
        <div>
          <label className="mb-1 block text-caption text-text-2" htmlFor="transfer-dst">
            转入账户
          </label>
          <select
            id="transfer-dst"
            className={SELECT_CLS}
            value={dstAcc}
            onChange={(e) => {
              setDstAcc(e.target.value);
              setConfirming(false);
            }}
          >
            <option value="" disabled>
              选择转入账户
            </option>
            {list.map((a) => (
              <option key={a.code} value={a.code} disabled={a.lostFlag}>
                {a.label}
                {a.lostFlag ? "（已挂失）" : ""}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label className="mb-1 block text-caption text-text-2" htmlFor="transfer-amount">
            转账金额（元）
          </label>
          <Input
            id="transfer-amount"
            className="tabular-num max-w-[9rem]"
            value={amount}
            inputMode="decimal"
            autoComplete="off"
            placeholder="0.00"
            aria-invalid={amountErr !== ""}
            onChange={(e) => {
              setAmount(e.target.value);
              setConfirming(false);
            }}
          />
        </div>
      </div>
      {src && (
        <p className="tabular-num mt-1.5 text-caption text-text-2">
          转出账户余额 ¥ {src.balanceYuan.toFixed(2)}
        </p>
      )}
      {amountErr && <p className="mt-1.5 text-caption text-alert">{amountErr}</p>}
      {err && <p className="mt-1.5 text-caption text-alert">{err}</p>}

      {confirming && src && dst ? (
        <div className="mt-3 rounded-inner border border-line bg-surface-2 px-3 py-3">
          <p className="text-body text-text">
            确认从 {src.label} 转出{" "}
            <span className="tabular-num font-medium">
              ¥ {Number(amount.trim()).toFixed(2)}
            </span>{" "}
            到 {dst.label}？
          </p>
          <div className="mt-2.5 flex flex-wrap items-center gap-2">
            <Button size="sm" disabled={busy} onClick={() => void submit()}>
              确认转账
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={() => setConfirming(false)}
            >
              返回修改
            </Button>
          </div>
        </div>
      ) : (
        <div className="mt-3">
          <Button
            size="sm"
            disabled={!canConfirm}
            onClick={() => {
              setErr("");
              setConfirming(true);
            }}
          >
            转账
          </Button>
        </div>
      )}
    </Surface>
  );
}
