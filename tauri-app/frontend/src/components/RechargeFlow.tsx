import { useEffect, useState } from "react";
import { ShieldCheck, Zap } from "lucide-react";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/shared/cn";
import { invokeCommand } from "@/shared/tauriApi";
import type {
  FeeItem,
  PasswordPad as PasswordPadData,
  RechargeAccounts,
  RechargeCreated,
  RechargeOrder,
  RechargePayMethod,
  RechargePayMethods,
  RechargeStatus,
  RoomStep,
} from "@/shared/types";

/**
 * 电费充值流程（M3.1 批 D，计划 §2.3）：金额 → 支付方式 → 免密或安全键盘 → 结果轮询 → 取消。
 *
 * # 安全红线（计划 Global Constraints，逐条落在此文件）
 *
 * - **密码键盘提交的是「键位下标」**：服务端下发的 `keys`（10 个显示字符）**只用于渲染**按键文字；
 *   用户点第 i 个键就往序列里 push `String(i)`，满 6 位把 `"013579"` 这串**下标** + `uuid` 交给后端。
 *   **绝不还原真实密码、绝不落盘/打日志/回填任何输入框**；`keys`/`uuid` 只在本笔支付的组件状态里
 *   存活，支付结束（成功/取消/放弃/离开页面）即 `clearSecrets` 清空。
 * - **不碰金额语义**：`retainMoney`/`maxmoney` 只做前端提示（不合法就禁用按钮），提交时把用户敲的
 *   原字符串透传给后端，客户端**不自行改写**金额。
 * - **副作用请求不重试**：`recharge_create` / `recharge_submit` 各只发一次（用户手点触发）；
 *   结果一律以 `recharge_status` 轮询兜底判定（2s × 最多 15 次 ≈ 30s）。
 * - **轮询有上限 + 卸载清理**：见 POLL_* 常量与轮询 effect 的 cleanup。
 * - **不实现跳转分支**（`webUrl`/`paysubmit`/`paymentcashierStr`/`qrCodeUrl`）：后端遇到会报错，
 *   这里只把服务端 `msg` 原样展示。
 *
 * # `third_party` 与 `path`（批 C 收口后）
 *
 * 房间上下文串 `third_party`（= 末级 IEC 响应 `map.data` 的 JSON，含户号等 PII）**由后端按房间路径
 * 合成**（`recharge.rs::third_party_for_room`），前端只把**当前级联路径** `path` 交给
 * `recharge_create(feeitemId, tranamt, path)`。`path` 在命令层是 `Option`（只为兼容旧前端），
 * 缺失时后端回可读文案（「缺少房间信息，请返回上一步重新选择房间后再试（客户端需更新）」）——
 * 本组件经 `setMsg(created.message)` **原样展示**，不吞错误。
 */

/** 结果轮询：间隔 2s、上限 15 次（≈30s）。 */
const POLL_INTERVAL_MS = 2_000;
const POLL_MAX_ATTEMPTS = 15;
/** 校园卡查询密码位数（点满自动提交）。 */
const PASSWORD_LEN = 6;

/** 下拉样式（与 `PowerPanel` 同款；面板内联 const 不值得到处导）。 */
const SELECT_CLS =
  "h-9 rounded-control border border-line bg-surface px-2 text-body text-text disabled:opacity-50";

/** 风险声明正文（计划 §2.4，**逐字**，含行首的「·」便于 grep 核对）；标题单独渲染，故不在此列。 */
const RISK_LINES = [
  "· 本功能直接调用学校官方缴费接口，金额与房间信息以学校系统返回为准。",
  "· 客户端不接触、不保存你的校园卡密码：密码在你点击的键盘位置上加密校验，由学校系统完成比对。",
  "· 下单成功后若未完成支付，可点「取消订单」释放；已扣款项请以学校官方系统记录为准。",
  "· 学校接口若调整，本功能可能失败——此时请用「去官网充值」。",
];

/**
 * 流程阶段：`idle` 填金额 → `preparing` 下单+取支付方式+取账户 → `await` 免密确认或输密码
 * → `submitting` 提交 → `polling` 轮询 → `done` / `error`（错误文案原样展示服务端 `msg`）。
 */
type Phase = "idle" | "preparing" | "await" | "submitting" | "polling" | "done" | "error";

export function RechargeFlow({
  feeitem,
  roomLabel,
  path,
}: {
  feeitem: FeeItem;
  /** 当前房间的人类可读标签（面包屑 name 拼接），仅展示 */
  roomLabel: string;
  /** 当前房间的完整级联路径（校区 → 楼栋 → 房间）：后端据它合成 `third_party`（PII 不出后端） */
  path: RoomStep[];
}) {
  const [amount, setAmount] = useState("");
  /** 风险声明勾选（计划 §2.4：勾选后方可继续） */
  const [agreed, setAgreed] = useState(false);
  const [phase, setPhase] = useState<Phase>("idle");
  const [msg, setMsg] = useState("");
  const [order, setOrder] = useState<RechargeOrder | null>(null);
  const [method, setMethod] = useState<RechargePayMethod | null>(null);
  const [accounts, setAccounts] = useState<string[]>([]);
  const [ccctypes, setCcctypes] = useState<string[]>([]);
  const [accountno, setAccountno] = useState("");
  const [ccctype, setCcctype] = useState("");
  /** 安全键盘数据：只在本笔支付期间存活（见文件头注红线） */
  const [pad, setPad] = useState<PasswordPadData | null>(null);
  /** 用户点击的**键位下标**序列（不是键上的字符） */
  const [seq, setSeq] = useState<string[]>([]);
  /** 服务端拒绝后可重来（如密码错误） */
  const [canRetry, setCanRetry] = useState(false);

  const numeric = Number(amount.trim());
  const amountValid = amount.trim() !== "" && Number.isFinite(numeric) && numeric > 0;
  const amountHint =
    amount.trim() === ""
      ? ""
      : !amountValid
        ? "请输入有效金额"
        : feeitem.retainMoney != null && numeric < feeitem.retainMoney
          ? `单次充值不能少于 ¥${feeitem.retainMoney}`
          : feeitem.maxmoney != null && numeric > feeitem.maxmoney
            ? `单次充值不能超过 ¥${feeitem.maxmoney}`
            : "";
  const rangeCaption = [
    feeitem.retainMoney != null ? `单次 ${feeitem.retainMoney} 元起` : "",
    feeitem.maxmoney != null ? `单次最多 ${feeitem.maxmoney} 元` : "",
  ]
    .filter(Boolean)
    .join(" · ");
  const canStart = phase === "idle" && agreed && amountValid && amountHint === "";
  const started = phase !== "idle";
  /** 提交的前提：后端 `submit_pay` 对 accountno/ccctype 都有非空校验（缺一即报错）。 */
  const canSubmit = accountno !== "" && ccctype !== "";

  /** 清掉本笔支付的一切敏感内存（`keys`/`uuid`/已点下标）。 */
  const clearSecrets = () => {
    setPad(null);
    setSeq([]);
  };

  /** 回到填金额态（订单相关的全部状态清掉）。 */
  const backToIdle = () => {
    clearSecrets();
    setOrder(null);
    setMethod(null);
    setAccounts([]);
    setCcctypes([]);
    setAccountno("");
    setCcctype("");
    setCanRetry(false);
    setPhase("idle");
  };

  /** 取某账号的账户类型 + 安全键盘（协议第二步；换账号也要重跑）。 */
  const loadAccountTypes = async (
    orderId: string,
    m: RechargePayMethod,
    acc: string,
  ): Promise<string | null> => {
    const r = await invokeCommand<RechargeAccounts>("recharge_query_account", {
      orderId,
      code: m.code,
      payid: m.payid,
      accountno: acc,
    });
    if (!r.success || !r.data) return r.message ?? "获取扣款账户失败";
    setCcctypes(r.data.ccctypes);
    setCcctype(r.data.ccctypes[0] ?? "");
    setPad(r.data.pad ?? null);
    return null;
  };

  /** 建单 → 取支付方式 → 取扣款账户（两步）。副作用请求，各只发一次。 */
  const start = async () => {
    if (!canStart) return;
    setPhase("preparing");
    setMsg("");
    setCanRetry(false);

    const created = await invokeCommand<RechargeCreated>("recharge_create", {
      // IPC 键名必须是 `feeitemId`（Rust 侧 `feeitem_id` 的 camelCase）；写成 feeItemId 会被
      // Tauri 判为缺参：`invalid args feeitemId ... missing required key feeitemId`（真机点验抓到的坑）
      feeitemId: feeitem.id,
      // 原样透传用户输入（红线：客户端不改写金额）
      tranamt: amount.trim(),
      // 房间路径：`third_party` 由后端按它合成（PII 不出后端）。空路径时后端会回可读文案，
      // 走下面的 setMsg 原样展示。
      path,
    });
    if (!created.success || !created.data?.orderId) {
      setPhase("error");
      setMsg(created.message ?? "下单失败");
      return;
    }
    const orderId = created.data.orderId;

    const pm = await invokeCommand<RechargePayMethods>("recharge_pay_methods", { orderId });
    if (!pm.success || !pm.data) {
      setPhase("error");
      setMsg(pm.message ?? "获取支付方式失败");
      // 留住订单号：这一步已经建单，失败也要能取消
      setOrder({ orderId, status: 0, payExpDate: null, tranamt: null });
      return;
    }
    const o = pm.data.order;
    const live = { ...o, orderId: o.orderId || orderId };
    setOrder(live);
    if (live.status === 1) {
      clearSecrets();
      setPhase("done");
      return;
    }
    const m = pm.data.methods[0];
    if (!m) {
      setPhase("error");
      setMsg("该缴费项没有可用的账户支付方式（本项目不支持第三方渠道）");
      return;
    }
    setMethod(m);

    // 账户两步：不带 accountno ⇒ 账号列表；带已选 accountno ⇒ 账户类型 + 安全键盘。
    // 免密分支同样要跑完（提交体必须带 accountno/ccctype，缺一后端直接拒绝）。
    const first = await invokeCommand<RechargeAccounts>("recharge_query_account", {
      orderId,
      code: m.code,
      payid: m.payid,
    });
    if (!first.success || !first.data) {
      setPhase("error");
      setMsg(first.message ?? "获取扣款账户失败");
      return;
    }
    const list = first.data.accounts;
    setAccounts(list);
    const acc0 = list[0] ?? "";
    setAccountno(acc0);
    // 第一步若已顺带回账户类型/键盘（服务端行为未定），先落上；第二步拿到更准的再覆盖
    setCcctypes(first.data.ccctypes);
    setCcctype(first.data.ccctypes[0] ?? "");
    setPad(first.data.pad ?? null);
    if (acc0) {
      const err = await loadAccountTypes(orderId, m, acc0);
      if (err) {
        setPhase("error");
        setMsg(err);
        return;
      }
    }
    setPhase("await");
  };

  /** 提交支付。`passwordSeq` 为**键位下标序列**；免密分支不传它（也不传 uuid）。 */
  const submit = async (passwordSeq?: string) => {
    if (!order || !method || phase === "submitting" || phase === "polling") return;
    if (!canSubmit) {
      setPhase("error");
      setMsg("未取得扣款账户/账户类型，无法提交——可取消订单后重试");
      return;
    }
    setPhase("submitting");
    setMsg("");
    const r = await invokeCommand("recharge_submit", {
      orderId: order.orderId,
      code: method.code,
      payid: method.payid,
      accountno,
      ccctype,
      passwordSeq,
      uuid: passwordSeq === undefined ? undefined : (pad?.uuid ?? undefined),
    });
    if (!r.success) {
      // 服务端 msg 原样展示（如「密码错误」），允许重新输一次
      setPhase("error");
      setMsg(r.message ?? "支付失败");
      setCanRetry(true);
      setSeq([]);
      return;
    }
    setSeq([]);
    setPhase("polling");
  };

  /** 取消订单（失败/放弃/超时后的清理入口）。
   *  学校侧事实：`POST /charge/order/deleteOrder` **只有 JSON body 才返回 200**（form/query/GET 恒 500）——
   *  由 crate 内部按 JSON body 发，故这里「取消成功」的判定依据不变（`r.success`）。 */
  const cancel = async () => {
    if (!order) {
      backToIdle();
      return;
    }
    const r = await invokeCommand("recharge_cancel", { orderId: order.orderId });
    if (!r.success) {
      setPhase("error");
      setMsg(r.message ?? "取消订单失败");
      return;
    }
    backToIdle();
    setMsg("订单已取消");
  };

  /** 点键：**只记下标**；满 6 位自动提交（计划 §1.3）。 */
  const pressKey = (index: number) => {
    if (phase !== "await" || !canSubmit || seq.length >= PASSWORD_LEN) return;
    const next = [...seq, String(index)];
    setSeq(next);
    if (next.length === PASSWORD_LEN) void submit(next.join(""));
  };

  // 结果轮询：2s × 最多 15 次；离开页面/切阶段（组件卸载或 effect 重跑）即清定时器。
  useEffect(() => {
    if (phase !== "polling" || !order) return;
    let alive = true;
    let attempt = 0;
    let timer = 0;
    const tick = async () => {
      if (!alive) return;
      attempt += 1;
      const r = await invokeCommand<RechargeStatus>("recharge_status", {
        orderId: order.orderId,
      });
      if (!alive) return;
      if (r.success && r.data?.order.status === 1) {
        clearSecrets();
        setPhase("done");
        return;
      }
      if (attempt >= POLL_MAX_ATTEMPTS) {
        setPhase("error");
        setMsg(
          `未在 ${(POLL_INTERVAL_MS * POLL_MAX_ATTEMPTS) / 1000} 秒内拿到支付结果：订单可能仍未支付。` +
            "可继续查询，或取消订单后重来。",
        );
        return;
      }
      timer = window.setTimeout(() => void tick(), POLL_INTERVAL_MS);
    };
    timer = window.setTimeout(() => void tick(), POLL_INTERVAL_MS);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
    // clearSecrets 是纯 setState 包装，不随渲染变化
  }, [phase, order]);

  const orderYuan = (order?.tranamt ?? numeric).toFixed(2);

  return (
    <Surface accent="wallet" className="mt-3 px-4 py-4">
      <div className="flex items-baseline justify-between gap-3">
        <p className="text-body font-medium text-text">充值</p>
        <span className="min-w-0 truncate text-caption text-text-2">{roomLabel}</span>
      </div>

      {/* 金额输入：快捷档取 layout，范围只作提示，不做改写 */}
      <div className="mt-3">
        <label className="mb-1 block text-caption text-text-2" htmlFor="elec-amount">
          充值金额（元）
        </label>
        <div className="flex flex-wrap items-center gap-2">
          <Input
            id="elec-amount"
            className="max-w-[9rem]"
            value={amount}
            inputMode="decimal"
            autoComplete="off"
            placeholder="请输入金额"
            disabled={started}
            aria-invalid={amountHint !== ""}
            onChange={(e) => setAmount(e.target.value)}
          />
          {feeitem.layout.map((v) => (
            <Button
              key={v}
              variant={amount.trim() === String(v) ? "default" : "outline"}
              size="sm"
              disabled={started}
              onClick={() => setAmount(String(v))}
            >
              {v} 元
            </Button>
          ))}
        </div>
        {rangeCaption && <p className="mt-1.5 text-caption text-text-2">{rangeCaption}</p>}
        {amountHint && <p className="mt-1.5 text-caption text-alert">{amountHint}</p>}
      </div>

      {/* 风险声明（常驻，计划 §2.4 逐字） */}
      <div className="mt-3 rounded-inner border border-line bg-surface-2 px-3 py-3">
        <p className="flex items-center gap-1.5 text-body font-medium text-alert">
          <ShieldCheck aria-hidden className="size-4 shrink-0" />
          充值会真实扣款，且不可撤销。
        </p>
        <ul className="mt-1.5 space-y-1">
          {RISK_LINES.map((line) => (
            <li key={line} className="text-caption text-text-2">
              {line}
            </li>
          ))}
        </ul>
        <label
          className="mt-2 flex items-start gap-2 text-caption text-text"
          htmlFor="recharge-risk"
        >
          <input
            id="recharge-risk"
            type="checkbox"
            className="mt-0.5 size-3.5 shrink-0 accent-[var(--color-wallet)]"
            checked={agreed}
            disabled={started}
            onChange={(e) => setAgreed(e.target.checked)}
          />
          我已阅读并同意上述风险说明
        </label>
      </div>

      {/* 订单摘要（已建单后常驻） */}
      {started && order && (
        <dl className="mt-3 rounded-inner border border-line bg-surface-2 px-3 py-2.5">
          <div className="flex items-baseline justify-between gap-3">
            <dt className="text-caption text-text-2">订单金额</dt>
            <dd className="tabular-num text-body text-text">¥ {orderYuan}</dd>
          </div>
          {order.payExpDate && (
            <div className="mt-1 flex items-baseline justify-between gap-3">
              <dt className="text-caption text-text-2">支付截止</dt>
              <dd className="tabular-num text-body text-text-2">{order.payExpDate}</dd>
            </div>
          )}
        </dl>
      )}

      {phase === "idle" && (
        <div className="mt-3">
          <Button disabled={!canStart} onClick={() => void start()}>
            <Zap />
            立即充值
          </Button>
          {msg && <p className="mt-2 text-caption text-text-2">{msg}</p>}
        </div>
      )}

      {phase === "preparing" && (
        <p className="mt-3 text-body text-text-2" aria-busy>
          正在下单…
        </p>
      )}

      {phase === "await" && method && (
        <div className="mt-3">
          <p className="text-caption text-text-2">
            支付方式：{method.name}
            {method.remark ? `（${method.remark}）` : ""}
          </p>

          {/* 多账户/多账户类型才需要选（默认取第一项）；换账号要重跑第二步（类型+键盘随账号变） */}
          {accounts.length > 1 && (
            <select
              className={cn(SELECT_CLS, "mt-2 mr-2")}
              aria-label="扣款账户"
              value={accountno}
              onChange={(e) => {
                const next = e.target.value;
                setAccountno(next);
                setSeq([]);
                if (order && method) void loadAccountTypes(order.orderId, method, next);
              }}
            >
              {accounts.map((a) => (
                <option key={a} value={a}>
                  {a}
                </option>
              ))}
            </select>
          )}
          {ccctypes.length > 1 && (
            <select
              className={cn(SELECT_CLS, "mt-2")}
              aria-label="账户类型"
              value={ccctype}
              onChange={(e) => setCcctype(e.target.value)}
            >
              {ccctypes.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
            </select>
          )}
          {!canSubmit && (
            <p className="mt-2 text-caption text-alert">
              未取得扣款账户或账户类型，暂时无法提交（可取消订单后重试）。
            </p>
          )}

          {method.nopassword ? (
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <Button disabled={!canSubmit} onClick={() => void submit()}>
                确认支付 ¥ {orderYuan}
              </Button>
              <Button variant="outline" onClick={() => void cancel()}>
                取消订单
              </Button>
            </div>
          ) : pad ? (
            <>
              <p className="mt-3 text-caption text-text-2">
                请输入校园卡查询密码（{PASSWORD_LEN} 位，点满自动提交）
              </p>
              <div className="mt-2 flex items-center gap-2">
                {Array.from({ length: PASSWORD_LEN }, (_, i) => (
                  <span
                    key={i}
                    aria-hidden
                    className={cn(
                      "size-2.5 rounded-full border border-line",
                      i < seq.length && "border-transparent bg-[var(--color-wallet)]",
                    )}
                  />
                ))}
              </div>
              <PasswordPad
                keys={pad.keys}
                onKey={pressKey}
                onDelete={() => setSeq((s) => s.slice(0, -1))}
              />
              <div className="mt-3">
                <Button variant="outline" size="sm" onClick={() => void cancel()}>
                  取消订单
                </Button>
              </div>
            </>
          ) : (
            <p className="mt-3 text-body text-alert">
              未获取到安全键盘数据，请取消订单后重试。
            </p>
          )}
        </div>
      )}

      {(phase === "submitting" || phase === "polling") && (
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <Button aria-busy disabled>
            {phase === "submitting" ? "正在提交支付…" : "等待支付结果…"}
          </Button>
          <Button variant="outline" onClick={() => void cancel()}>
            取消订单
          </Button>
          {phase === "polling" && (
            <span className="text-caption text-text-2">
              每 {POLL_INTERVAL_MS / 1000} 秒查询一次，最多{" "}
              {(POLL_INTERVAL_MS * POLL_MAX_ATTEMPTS) / 1000} 秒
            </span>
          )}
        </div>
      )}

      {phase === "done" && (
        <div className="mt-3">
          <p className="text-body text-text">充值成功，订单已完成。</p>
          <Button variant="outline" size="sm" className="mt-2" onClick={backToIdle}>
            再充一笔
          </Button>
        </div>
      )}

      {phase === "error" && (
        <div className="mt-3">
          <p className="text-body text-alert">{msg || "支付未完成"}</p>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            {method && (
              <Button variant="outline" size="sm" onClick={() => setPhase("polling")}>
                继续查询支付结果
              </Button>
            )}
            {canRetry && pad && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  setSeq([]);
                  setPhase("await");
                }}
              >
                重新输入密码
              </Button>
            )}
            {order && (
              <Button variant="outline" size="sm" onClick={() => void cancel()}>
                取消订单
              </Button>
            )}
            <Button variant="ghost" size="sm" onClick={backToIdle}>
              重新开始
            </Button>
          </div>
          <p className="mt-2 text-caption text-text-2">
            若学校接口异常，请改用上方的「去官网充值」。
          </p>
        </div>
      )}
    </Surface>
  );
}

/**
 * 安全键盘（**红线**）：`keys` 只用于渲染按键上的显示字符；回调一律传**键位下标**（第 i 个键 → i），
 * 提交给后端的就是这串下标。绝不把 `keys` 拼成密码、绝不缓存/落盘/打日志/回填输入框。
 * 布局照官方：前 9 键排 3×3，第 10 键前留一空格，末格为删除。
 *
 * ⚠️ 学校侧实测：`passwordMap[uuid]` 是 **10 个字符的字符串**（不是数组），官方前端逐字符渲染。
 * 批 C 的 crate 已按字符拆成数组下发，此处**再兜一层**：真收到整串就逐字符展开——
 * 两种形态下**下标语义完全一致**（第 i 个字符 = 第 i 个键），所以提交永远是下标。
 */
function PasswordPad({
  keys: keysRaw,
  onKey,
  onDelete,
}: {
  keys: string[] | string;
  onKey: (index: number) => void;
  onDelete: () => void;
}) {
  const keys = typeof keysRaw === "string" ? Array.from(keysRaw) : keysRaw;
  const keyCls =
    "tabular-num h-11 rounded-control border border-line bg-surface text-title font-medium text-text hover:border-line-strong";
  return (
    <div className="mt-2 grid max-w-[17rem] grid-cols-3 gap-2">
      {keys.slice(0, 9).map((k, i) => (
        <button key={i} type="button" className={keyCls} onClick={() => onKey(i)}>
          {k}
        </button>
      ))}
      {keys.length > 9 && <span aria-hidden />}
      {keys.slice(9).map((k, i) => (
        <button
          key={`k${9 + i}`}
          type="button"
          className={keyCls}
          onClick={() => onKey(9 + i)}
        >
          {k}
        </button>
      ))}
      <button type="button" className={keyCls} onClick={onDelete}>
        删除
      </button>
    </div>
  );
}
