import { History, RefreshCw, Zap } from "lucide-react";
import { useEffect, useState } from "react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader } from "@/components/PanelHeader";
import { RechargeFlow } from "@/components/RechargeFlow";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import { invokeCommand } from "@/shared/tauriApi";
import { cn } from "@/shared/cn";
import type {
  ElectricityChoice,
  ElectricityLevel,
  ElectricityView,
  FeeItem,
  RoomStep,
  SavedRoom,
} from "@/shared/types";

/** 片区目录三态：免登录即可取（`/charge/feeitem` 是该服务唯一匿名端点）。 */
type ListPhase = "loading" | "ready" | "error";
/** 级联查询三态。 */
type QueryPhase = "idle" | "loading" | "error";

const SELECT_CLS =
  "h-9 rounded-control border border-line bg-surface px-2 text-body text-text disabled:opacity-50";

/**
 * 服务端那句展示文本 → 折行数组。**只按分隔符折行、不做语义解构**（2026-09-19 live 实测：
 * 三个片区的 `信息` 文案格式各不相同且由学校侧自由填写，任何「剩余金额/单价」式的键名或
 * 格式硬编码都会随文案漂移而静默失效）。
 */
export function fieldLines(value: string): string[] {
  return value
    .split(/[,，;；]/)
    .map((s) => s.trim())
    .filter(Boolean);
}

/** 含负数的行（如「剩余金额：-545.70」= 欠费）用警示色，标签无关。 */
export function isNegativeLine(line: string): boolean {
  return /-\d/.test(line);
}

/**
 * 电费页（M3 批 3）：片区 → 校区 → 楼栋 → 房间 级联查余额，常用房间本地存，充值走官方页面
 * （系统浏览器打开；客户端直调官方接口的充值在后续批次接入）。
 *
 * 两条 live 实测约束体现在 UI 上：
 * ① 末级（房间）是**输入级**（片区 `lastLevelIsInput`）：服务端在末级前一档不下发选项，
 *    故 `options` 为空**不是**出错，此处渲染房间号输入框；
 * ② 末级结果只有一句服务端自由文本（`fields`），**通用渲染 + 折行**，不解构、不硬编码标签。
 */
export function PowerPanel() {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const authed = status === "authed";

  const [items, setItems] = useState<FeeItem[]>([]);
  const [itemsPhase, setItemsPhase] = useState<ListPhase>("loading");
  const [itemsError, setItemsError] = useState("");
  const [itemsTick, setItemsTick] = useState(0);

  const [areaId, setAreaId] = useState("");
  const [levels, setLevels] = useState<ElectricityLevel[]>([]);
  const [options, setOptions] = useState<ElectricityChoice[]>([]);
  const [steps, setSteps] = useState<RoomStep[]>([]);
  const [view, setView] = useState<ElectricityView | null>(null);
  const [phase, setPhase] = useState<QueryPhase>("idle");
  const [error, setError] = useState("");
  const [roomText, setRoomText] = useState("");

  const [rooms, setRooms] = useState<SavedRoom[]>([]);
  const [saveLabel, setSaveLabel] = useState("");
  const [notice, setNotice] = useState("");

  const area = items.find((i) => i.id === areaId);
  const depth = steps.length;
  const level = levels[depth];

  // 片区目录：免登录也能渲染（未登录时级联处给登录引导）
  useEffect(() => {
    let alive = true;
    setItemsPhase("loading");
    invokeCommand<FeeItem[]>("list_feeitems").then((r) => {
      if (!alive) return;
      if (r.success && r.data) {
        setItems(r.data);
        setItemsPhase("ready");
      } else {
        setItemsError(r.message ?? "片区列表获取失败");
        setItemsPhase("error");
      }
    });
    return () => {
      alive = false;
    };
  }, [itemsTick]);

  // 常用房间：本机 JSON，读失败后端回空列表
  useEffect(() => {
    let alive = true;
    invokeCommand<SavedRoom[]>("get_electricity_rooms").then((r) => {
      if (alive && r.success && r.data) setRooms(r.data);
    });
    return () => {
      alive = false;
    };
  }, []);

  /** 一次级联查询：path 为空取第 1 级选项；到末级则出 view。 */
  const runQuery = async (feeitemId: string, path: RoomStep[]) => {
    setPhase("loading");
    setError("");
    setNotice("");
    const r = await invokeCommand<{
      levels: ElectricityLevel[];
      options: ElectricityChoice[];
      isFinal: boolean;
      view: ElectricityView | null;
    }>("query_electricity", { feeitemId, path });
    if (!r.success || !r.data) {
      setPhase("error");
      setError(r.message ?? "电费查询失败");
      setOptions([]);
      setView(null);
      return;
    }
    setPhase("idle");
    setLevels(r.data.levels);
    setOptions(r.data.options);
    setView(r.data.isFinal ? (r.data.view ?? null) : null);
  };

  const pickArea = (id: string) => {
    if (!authed) {
      openLoginDialog();
      return;
    }
    setAreaId(id);
    setSteps([]);
    setLevels([]);
    setOptions([]);
    setView(null);
    setRoomText("");
    setSaveLabel("");
    void runQuery(id, []);
  };

  /** 选中第 depth 级的一项：截断更深的选择后重查。 */
  const pickChoice = (c: ElectricityChoice) => {
    const step: RoomStep = { level: c.level, code: c.code, value: c.value, name: c.label };
    const path = [...steps.slice(0, depth), step];
    setSteps(path);
    setView(null);
    void runQuery(areaId, path);
  };

  /** 末级输入级（房间号）提交。 */
  const submitRoom = (raw: string) => {
    const text = raw.trim();
    if (!level) return;
    if (!text) {
      setPhase("error");
      setError("请输入房间号");
      return;
    }
    const step: RoomStep = { level: level.level, code: level.code, value: text, name: text };
    const path = [...steps.slice(0, depth), step];
    setSteps(path);
    setView(null);
    void runQuery(areaId, path);
  };

  /** 面包屑回退：点第 i 段 ⇒ 只保留前 i 段并重查（重查即回该层的选择项）。 */
  const backTo = (i: number) => {
    const path = steps.slice(0, i);
    setSteps(path);
    setView(null);
    setRoomText("");
    void runQuery(areaId, path);
  };

  const resetAll = () => {
    setSteps([]);
    setLevels([]);
    setOptions([]);
    setView(null);
    setPhase("idle");
    setError("");
    setRoomText("");
    if (areaId) void runQuery(areaId, []);
  };

  const saveRoom = async () => {
    if (!area) return;
    const r = await invokeCommand<SavedRoom[]>("save_electricity_room", {
      room: {
        id: "",
        feeitemId: area.id,
        feeitemName: area.name,
        path: steps,
        label: saveLabel,
      },
    });
    if (r.success && r.data) {
      setRooms(r.data);
      setSaveLabel("");
      setNotice("已保存为常用房间");
    } else {
      setNotice(r.message ?? "保存失败");
    }
  };

  const removeRoom = async (id: string) => {
    const r = await invokeCommand<SavedRoom[]>("delete_electricity_room", { id });
    if (r.success && r.data) setRooms(r.data);
    else setNotice(r.message ?? "删除失败");
  };

  const openRoom = (room: SavedRoom) => {
    setAreaId(room.feeitemId);
    setSteps(room.path);
    setLevels([]);
    setOptions([]);
    setView(null);
    setRoomText("");
    void runQuery(room.feeitemId, room.path);
  };

  /** 充值：在系统浏览器打开官方充值页（2026-09-19 裁决：不做内嵌官方界面）。 */
  const recharge = async () => {
    if (!areaId) return;
    const r = await invokeCommand("open_recharge_in_browser", { feeitemId: areaId });
    setNotice(r.success ? "" : (r.message ?? "打开充值页失败"));
  };

  const isInputLevel = !!level && depth === levels.length - 1 && area?.lastLevelIsInput === true;
  const complete = steps.length > 0 && steps.length === levels.length;

  return (
    <section className="mx-auto mt-8 max-w-3xl px-4">
      <PanelHeader title="电费" description="宿舍电费查询与充值" domain="wallet" />

      {/* 片区：该接口免登录（唯一匿名端点），未登录也先渲染出来 */}
      <Surface className="px-4 py-4">
        <div className="flex items-baseline justify-between gap-3">
          <p className="text-body font-medium text-text">缴费片区</p>
          {itemsPhase === "ready" && (
            <span className="text-caption text-text-2">共 {items.length} 个启用片区</span>
          )}
        </div>
        {itemsPhase === "loading" ? (
          <div aria-hidden className="mt-3 flex gap-2">
            {[0, 1, 2].map((i) => (
              <div key={i} className="h-8 w-32 animate-pulse rounded-control bg-line" />
            ))}
          </div>
        ) : itemsPhase === "error" ? (
          <div className="mt-3 flex items-center justify-between gap-3">
            <p className="min-w-0 truncate text-body text-text-2">{itemsError}</p>
            <Button variant="outline" size="sm" onClick={() => setItemsTick((t) => t + 1)}>
              <RefreshCw />
              重试
            </Button>
          </div>
        ) : items.length === 0 ? (
          <EmptyState compact icon={Zap} domain="wallet" title="暂无启用的电费片区" />
        ) : (
          <div className="mt-3 flex flex-wrap gap-2">
            {items.map((it) => (
              <Button
                key={it.id}
                variant={it.id === areaId ? "default" : "outline"}
                size="sm"
                aria-pressed={it.id === areaId}
                onClick={() => pickArea(it.id)}
              >
                {it.name}
              </Button>
            ))}
          </div>
        )}
      </Surface>

      {/* 级联查询 */}
      <Surface accent="wallet" className="mt-3 px-4 py-4">
        <div className="flex items-baseline justify-between gap-3">
          <p className="text-body font-medium text-text">查询房间余额</p>
          {complete && (
            <Button variant="ghost" size="xs" onClick={resetAll}>
              重选
            </Button>
          )}
        </div>

        {!authed ? (
          <EmptyState
            compact
            icon={Zap}
            domain="wallet"
            title="登录后可查询房间电费"
            hint="电费接口需慧新E校会话；片区列表已可查看。"
            action={<Button onClick={openLoginDialog}>登录</Button>}
          />
        ) : !areaId ? (
          <EmptyState
            compact
            icon={Zap}
            domain="wallet"
            title="先选择缴费片区"
            hint="再逐级选择校区、楼栋并填写房间号。"
          />
        ) : (
          <>
            {/* 面包屑：可点回退到任一级 */}
            {steps.length > 0 && (
              <div className="mt-3 flex flex-wrap items-center gap-x-2 gap-y-1">
                {steps.map((s, i) => (
                  <span key={`${s.code}-${i}`} className="flex items-center gap-2">
                    {i > 0 && (
                      <span aria-hidden className="text-caption text-text-2">
                        ›
                      </span>
                    )}
                    <button
                      type="button"
                      className="rounded-control px-1 text-caption text-text-2 underline-offset-4 hover:text-text hover:underline"
                      onClick={() => backTo(i)}
                    >
                      {s.name || s.value}
                    </button>
                  </span>
                ))}
              </div>
            )}

            {/* 当前该选的一级（level 名来自服务端 map.total） */}
            {phase !== "error" && level && (
              <div className="mt-3">
                <label className="mb-1 block text-caption text-text-2" htmlFor="elec-cascade">
                  {level.name}
                </label>
                {options.length > 0 ? (
                  <select
                    id="elec-cascade"
                    className={SELECT_CLS}
                    value=""
                    disabled={phase === "loading"}
                    onChange={(e) => {
                      const hit = options.find((o) => o.value === e.target.value);
                      if (hit) pickChoice(hit);
                    }}
                  >
                    <option value="">
                      {phase === "loading" ? "加载中…" : `请选择${level.name}`}
                    </option>
                    {options.map((o) => (
                      <option key={o.value} value={o.value}>
                        {o.label}
                      </option>
                    ))}
                  </select>
                ) : isInputLevel ? (
                  <form
                    className="flex gap-2"
                    onSubmit={(e) => {
                      e.preventDefault();
                      submitRoom(roomText);
                    }}
                  >
                    <Input
                      id="elec-cascade"
                      value={roomText}
                      inputMode="numeric"
                      placeholder={`请输入${level.name}（例如 101）`}
                      disabled={phase === "loading"}
                      onChange={(e) => setRoomText(e.target.value)}
                    />
                    <Button type="submit" disabled={phase === "loading"}>
                      查询
                    </Button>
                  </form>
                ) : (
                  <p className="text-body text-text-2">
                    该层暂无可选项，请点上一级重选。
                  </p>
                )}
              </div>
            )}

            {phase === "loading" && (
              <div aria-hidden className="mt-3 h-9 w-full animate-pulse rounded bg-line" />
            )}

            {phase === "error" && (
              <div className="mt-3 flex items-center justify-between gap-3">
                <p className="min-w-0 text-body text-text-2">{error}</p>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => void runQuery(areaId, steps)}
                >
                  重试
                </Button>
              </div>
            )}

            {/* 末级结果：通用字典渲染（键名与文案都来自服务端） */}
            {view && (
              <div className="mt-3 rounded-inner border border-line bg-surface-2 px-3 py-3">
                {view.tip ? (
                  <p className="text-body text-alert">{view.tip}</p>
                ) : view.fields.length === 0 && view.money == null ? (
                  <p className="text-body text-text-2">该房间暂无用电信息</p>
                ) : (
                  <dl className="space-y-1.5">
                    {view.fields.map((f) => (
                      <div key={f.label}>
                        <dt className="text-caption text-text-2">{f.label}</dt>
                        {fieldLines(f.value).map((line) => (
                          <dd
                            key={line}
                            className={cn(
                              "tabular-num text-body",
                              isNegativeLine(line) ? "text-alert" : "text-text",
                            )}
                          >
                            {line}
                          </dd>
                        ))}
                      </div>
                    ))}
                    {view.money != null && (
                      <div>
                        <dt className="text-caption text-text-2">金额</dt>
                        <dd className="tabular-num text-body text-text">
                          ¥ {view.money.toFixed(2)}
                        </dd>
                      </div>
                    )}
                  </dl>
                )}

                {/* 保存为常用房间（用户起名，留空则用「楼栋 房间号」） */}
                {complete && (
                  <form
                    className="mt-3 flex gap-2"
                    onSubmit={(e) => {
                      e.preventDefault();
                      void saveRoom();
                    }}
                  >
                    <Input
                      value={saveLabel}
                      placeholder="给这个房间起个名（选填）"
                      onChange={(e) => setSaveLabel(e.target.value)}
                    />
                    <Button type="submit" variant="outline" size="sm">
                      保存房间
                    </Button>
                  </form>
                )}

                <div className="mt-3 flex flex-wrap items-center gap-2">
                  <Button size="sm" onClick={() => void recharge()}>
                    <Zap />
                    去官网充值
                  </Button>
                  {notice && <span className="text-caption text-text-2">{notice}</span>}
                </div>
              </div>
            )}

            {/* 未到末级也允许直接充值（官方页里可以自己选房间） */}
            {!view && phase !== "error" && (
              <div className="mt-3 flex flex-wrap items-center gap-2">
                <Button variant="outline" size="sm" onClick={() => void recharge()}>
                  去官网充值
                </Button>
                {notice && <span className="text-caption text-text-2">{notice}</span>}
              </div>
            )}
          </>
        )}
      </Surface>

      {/* 充值（M3.1 批 D）：房间选全且视图无 tip 才出现——金额 → 风险声明 → 支付方式 →
          免密/安全键盘 → 轮询。thirdParty 只透传后端给的（见 RechargeFlow 头注：前端不拼）。 */}
      {complete && area && view && !view.tip && (
        <RechargeFlow
          feeitem={area}
          roomLabel={steps.map((s) => s.name || s.value).join(" · ")}
          thirdParty={view.thirdParty ?? undefined}
        />
      )}

      {/* 常用房间：本地存（重启仍在），一键复查余额 */}
      <Surface className="mt-3 px-4 py-4">
        <p className="text-body font-medium text-text">常用房间</p>
        {rooms.length === 0 ? (
          <EmptyState
            compact
            icon={History}
            domain="wallet"
            title="还没有常用房间"
            hint="查到房间后点「保存房间」，之后可一键查余额。"
          />
        ) : (
          <ul className="mt-2 divide-y divide-line">
            {rooms.map((r) => (
              <li key={r.id} className="flex items-center gap-3 py-2.5">
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-body text-text">{r.label}</span>
                  <span className="mt-0.5 block truncate text-caption text-text-2">
                    {[r.feeitemName, r.path.map((s) => s.name || s.value).join(" · ")]
                      .filter(Boolean)
                      .join(" · ")}
                  </span>
                </span>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!authed}
                  onClick={() => (authed ? openRoom(r) : openLoginDialog())}
                >
                  查余额
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  aria-label={`删除 ${r.label}`}
                  onClick={() => void removeRoom(r.id)}
                >
                  删除
                </Button>
              </li>
            ))}
          </ul>
        )}
        {!authed && rooms.length > 0 && (
          <p className="mt-2 text-caption text-text-2">登录后才能查询，房间保存在本机。</p>
        )}
      </Surface>
    </section>
  );
}
