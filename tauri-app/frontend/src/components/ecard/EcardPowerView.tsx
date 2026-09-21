import { Home, History, RefreshCw, Zap } from "lucide-react";
import { useEffect, useState } from "react";
import { ElectricityPaymentsCard } from "@/components/ElectricityPaymentsCard";
import { ElectricityTrendCard } from "@/components/ElectricityTrendCard";
import { EmptyState } from "@/components/EmptyState";
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
  ElectricitySnapshot,
  ElectricityLevel,
  ElectricityView,
  FeeItem,
  RoomStep,
  SavedRoom,
} from "@/shared/types";

/**
 * 一卡通 · 电费子页（M4.5 批 3）：`panels/PowerPanel.tsx` 的电费内容整体搬入（2026-09-19 拆分：
 * wallet + power 两页合并进 ecard，见 `docs/superpowers/plans/2026-09-19-ecard-full-replica.md` §3）。
 *
 * 与 PowerPanel 的差异只有三处，**交互与四态写法零回归**：
 * ① 不再自带 `PanelHeader` 与 `mx-auto max-w-5xl px-4` 外壳——本组件被放进容器已带
 *    `mx-auto max-w-5xl px-4` 的子页框架里，根元素直接是双列 grid；
 * ② 新增 `onNavigate`：提供时在「缴费片区」卡头部给一个「一卡通充值」快捷入口（跳
 *    EcardRechargeView）。电费房间的充值仍走下方内联 `RechargeFlow`（带房间路径合成
 *    `third_party`，401 充的是卡账户与房间无关），故不改结果卡的按钮语义；
 * ③ `fieldLines`/`isNegativeLine`/`roomPathKey`/`loadRecent` 等纯函数在本文件各自保留一份
 *    （本批不抽公共模块，避免与 PowerPanel 的删除节奏冲突）。
 *
 * 两条 live 实测约束仍体现在 UI 上（详见 `panels/PowerPanel.tsx` 头注）：
 * ① 末级（房间）是**输入级**（片区 `lastLevelIsInput`）：`options` 为空**不是**出错，渲染房间号输入框；
 * ② 末级结果 `fields` 是服务端自由文本，**通用渲染 + 折行**；主数字用后端结构化的
 *    `view.balanceYuan`（`null` = 没提取到 ⇒ 显示「无数据」，绝不当 0）。
 */

/** 片区目录三态：免登录即可取（`/charge/feeitem` 是该服务唯一匿名端点）。 */
type ListPhase = "loading" | "ready" | "error";
/** 级联查询三态。 */
type QueryPhase = "idle" | "loading" | "error";

/**
 * 自动级联深度护栏：自动选定只可能逐级推进 path（见 runQuery 内防死循环注释），
 * 这里给整条链一个绝对上限（远大于真实层数 3），即使学校侧将来改成循环层级也不会挂死。
 */
const MAX_CASCADE_STEPS = 10;

/**
 * 「最近查过」的房间（**本机 localStorage 记录**，不是已保存房间）。
 * 每次查到末级房间即记一条（片区 + 完整路径），chips 显示**房间号**（路径末级的 `value`）。
 */
type RecentRoom = { feeitemId: string; path: RoomStep[]; at: number };

const RECENT_KEY = "campushub-elec-recent";
const RECENT_MAX = 8;

/** 房间的稳定比对键（片区 + 各层 `value`）：用于「同房间只留最新一条」。 */
function roomPathKey(feeitemId: string, path: RoomStep[]): string {
  return `${feeitemId}|${path.map((s) => s.value).join(">")}`;
}

function loadRecent(): RecentRoom[] {
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    const v = raw ? JSON.parse(raw) : [];
    return Array.isArray(v) ? (v as RecentRoom[]) : [];
  } catch {
    // 存储被禁用或内容损坏：静默回空，绝不影响查询主流程
    return [];
  }
}

function saveRecent(list: RecentRoom[]) {
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(list.slice(0, RECENT_MAX)));
  } catch {
    /* 配额满/被禁用：这只是便利记录，忽略 */
  }
}

/** 服务端那句展示文本 → 折行数组。**只按分隔符折行、不做语义解构**（各片区文案格式由学校侧自由填写）。 */
function fieldLines(value: string): string[] {
  return value
    .split(/[,，;；]/)
    .map((s) => s.trim())
    .filter(Boolean);
}

/** 含负数的行（如「剩余金额：-545.70」= 欠费）用警示色，标签无关。 */
function isNegativeLine(line: string): boolean {
  return /-\d/.test(line);
}

/** 一卡通 · 电费子页。布局与交互同 PowerPanel：宽屏双列 `lg:grid-cols-[minmax(0,1fr)_340px]`。 */
export function EcardPowerView({
  onNavigate,
}: {
  /** 提供时在片区卡头部显示「一卡通充值」快捷入口（跳 EcardRechargeView）。 */
  onNavigate?: (view: "ecard-recharge") => void;
}) {
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
  const [recent, setRecent] = useState<RecentRoom[]>(() => loadRecent());
  const [saveLabel, setSaveLabel] = useState("");
  const [notice, setNotice] = useState("");
  const [noticeBad, setNoticeBad] = useState(false);
  const [roomsNotice, setRoomsNotice] = useState("");

  /** 自动选定的层（唯一选项 ⇒ 自动选定）；渲染成弱提示，不占交互位。 */
  const [autoLevels, setAutoLevels] = useState<number[]>([]);
  /** 右列电费变化卡的刷新信号：采集成功 / 绑定变化后自增。 */
  const [trendTick, setTrendTick] = useState(0);
  const [snapBusy, setSnapBusy] = useState(false);
  const [bindBusy, setBindBusy] = useState(false);

  const area = items.find((i) => i.id === areaId);
  const depth = steps.length;
  const level = levels[depth];
  const boundRoom = rooms.find((r) => r.bound === true);

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

  /**
   * 一次级联查询：path 为空取第 1 级选项；到末级则出 view。
   *
   * **校区自动选定 + 防死循环**：`options.length === 1` 且该层不是末级时自动选定并携带
   * **增长后的** path 续查。收敛闸：① path 每次严格增长一级；② path 末段已等于该唯一选项
   * （服务端同层重发病态）则停止；③ `MAX_CASCADE_STEPS` 绝对上限兜底。
   * 判据只有「选项数 == 1」，不写死「校区」。
   */
  const runQuery = async (feeitemId: string, path: RoomStep[]) => {
    setPhase("loading");
    setError("");
    setNotice("");
    setNoticeBad(false);
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

    // 查到末级房间即记入「最近查过」（本机）：同房间去重后置顶、最多 RECENT_MAX 条。
    if (r.data.isFinal && path.length > 0) {
      setRecent((prev) => {
        const key = roomPathKey(feeitemId, path);
        const next = [
          { feeitemId, path, at: Date.now() },
          ...prev.filter((x) => roomPathKey(x.feeitemId, x.path) !== key),
        ].slice(0, RECENT_MAX);
        saveRecent(next);
        return next;
      });
    }

    const only = r.data.options.length === 1 ? r.data.options[0] : undefined;
    const last = path[path.length - 1];
    if (
      !r.data.isFinal &&
      only &&
      !(last && last.level === only.level && last.value === only.value) &&
      path.length < MAX_CASCADE_STEPS
    ) {
      setAutoLevels((prev) => (prev.includes(only.level) ? prev : [...prev, only.level]));
      const nextPath: RoomStep[] = [
        ...path,
        { level: only.level, code: only.code, value: only.value, name: only.label },
      ];
      setSteps(nextPath);
      setView(null);
      void runQuery(feeitemId, nextPath);
    }
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
    setAutoLevels([]);
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

  /** 面包屑回退：点第 i 段 ⇒ 只保留前 i 段并重查。 */
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
    setAutoLevels([]);
    setPhase("idle");
    setError("");
    setRoomText("");
    if (areaId) void runQuery(areaId, []);
  };

  /** 当前已选路径对应的常用房间（未保存过则为 undefined）。 */
  const currentRoom =
    area && steps.length > 0
      ? rooms.find(
          (r) =>
            r.feeitemId === areaId &&
            r.path.length === steps.length &&
            steps.every((s, i) => r.path[i]?.value === s.value),
        )
      : undefined;

  /**
   * 房间号输入级的一键重查 chips：**本机记录**中，片区与已选层级（除房间号那步）完全一致者。
   */
  const recentRooms =
    area && steps.length > 0 && isInputLevelSafe(levels, steps, area)
      ? recent.filter(
          (r) =>
            r.feeitemId === areaId &&
            r.path.length === steps.length + 1 &&
            steps.every((s, i) => r.path[i]?.value === s.value),
        )
      : [];

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
      setNoticeBad(false);
    } else {
      setNotice(r.message ?? "保存失败");
      setNoticeBad(true);
    }
  };

  /** 绑定当前房间为「我的宿舍」（同一时刻最多一个，绑新的后端自动解绑旧的）。 */
  const bindCurrent = async () => {
    if (!area) return;
    setBindBusy(true);
    setNotice("");
    setNoticeBad(false);
    try {
      let target = currentRoom;
      if (!target) {
        const saved = await invokeCommand<SavedRoom[]>("save_electricity_room", {
          room: {
            id: "",
            feeitemId: area.id,
            feeitemName: area.name,
            path: steps,
            label: "",
          },
        });
        if (!(saved.success && saved.data)) {
          setNotice(saved.message ?? "保存房间失败，无法绑定");
          setNoticeBad(true);
          return;
        }
        setRooms(saved.data);
        target = saved.data.find(
          (x) =>
            x.feeitemId === area.id &&
            x.path.length === steps.length &&
            steps.every((s, i) => x.path[i]?.value === s.value),
        );
      }
      if (!target) {
        setNotice("已保存但未能定位新房间，请重试");
        setNoticeBad(true);
        return;
      }
      const b = await invokeCommand<SavedRoom[]>("bind_electricity_room", {
        id: target.id,
        bound: true,
      });
      if (b.success && b.data) {
        setRooms(b.data);
        setTrendTick((t) => t + 1);
        setNotice(`已绑定「${target.label || "当前房间"}」为我的宿舍`);
        setNoticeBad(false);
      } else {
        setNotice(b.message ?? "绑定失败");
        setNoticeBad(true);
      }
    } finally {
      setBindBusy(false);
    }
  };

  /** 常用房间卡内的绑定/解绑（本地操作，不需登录）。 */
  const toggleBind = async (room: SavedRoom) => {
    setRoomsNotice("");
    const b = await invokeCommand<SavedRoom[]>("bind_electricity_room", {
      id: room.id,
      bound: !(room.bound === true),
    });
    if (b.success && b.data) {
      setRooms(b.data);
      setTrendTick((t) => t + 1);
    } else {
      setRoomsNotice(b.message ?? "绑定状态修改失败");
    }
  };

  const removeRoom = async (id: string) => {
    const r = await invokeCommand<SavedRoom[]>("delete_electricity_room", { id });
    if (r.success && r.data) setRooms(r.data);
    else setRoomsNotice(r.message ?? "删除失败");
  };

  const openRoom = (room: SavedRoom) => {
    if (!authed) {
      openLoginDialog();
      return;
    }
    setAreaId(room.feeitemId);
    setSteps(room.path);
    setLevels([]);
    setOptions([]);
    setView(null);
    setAutoLevels([]);
    setRoomText("");
    void runQuery(room.feeitemId, room.path);
  };

  /** 点「最近查过」的 chip：直接重查该房间（本机记录，无需先保存为常用房间）。 */
  const openRecent = (r: RecentRoom) => {
    setAreaId(r.feeitemId);
    setSteps(r.path);
    setLevels([]);
    setOptions([]);
    setView(null);
    setAutoLevels([]);
    setRoomText("");
    void runQuery(r.feeitemId, r.path);
  };

  /** 充值：应用内打开官方充值页（Rust 侧直建副 webview，经 browser://opened 事件同步前端工具栏）。 */
  const recharge = async () => {
    if (!areaId) return;
    const r = await invokeCommand("open_recharge_in_browser", { feeitemId: areaId });
    setNotice(r.success ? "" : (r.message ?? "打开充值页失败"));
    setNoticeBad(!r.success);
  };

  /** 立即采集：对「我的宿舍」跑一次余额快照（未绑定/未登录时后端回可读中文）。 */
  const runSnapshot = async () => {
    setSnapBusy(true);
    setNotice("");
    setNoticeBad(false);
    const r = await invokeCommand<ElectricitySnapshot>("run_electricity_snapshot");
    setSnapBusy(false);
    if (r.success && r.data) {
      setNotice(r.data.replaced ? "已更新今日余额记录" : "已记录今日余额");
      setTrendTick((t) => t + 1);
    } else {
      setNotice(r.message ?? "采集失败");
      setNoticeBad(true);
    }
  };

  const isInputLevel = !!level && depth === levels.length - 1 && area?.lastLevelIsInput === true;
  const complete = steps.length > 0 && steps.length === levels.length;
  /** 楼栋卡片「上次查过」角标的数据集（已存房间走过的全部层值）。 */
  const seenKeys = new Set(rooms.flatMap((r) => r.path.map((s) => `${s.level}:${s.value}`)));
  /** 常用房间列表：绑定项置顶。 */
  const sortedRooms = [...rooms].sort(
    (a, b) => Number(b.bound === true) - Number(a.bound === true),
  );

  return (
    <div className="grid items-start gap-3 lg:grid-cols-[minmax(0,1fr)_340px] lg:gap-4">
      {/* ── 左列：操作主线 ── */}
      <div className="flex min-w-0 flex-col gap-3">
        {/* 片区：该接口免登录（唯一匿名端点），未登录也先渲染出来 */}
        <Surface className="px-4 py-4">
          <div className="flex items-baseline justify-between gap-3">
            <p className="text-body font-medium text-text">缴费片区</p>
            <span className="flex shrink-0 items-baseline gap-2">
              {itemsPhase === "ready" && (
                <span className="text-caption text-text-2">共 {items.length} 个启用片区</span>
              )}
              {onNavigate && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="-mx-1"
                  onClick={() => onNavigate("ecard-recharge")}
                >
                  一卡通充值
                </Button>
              )}
            </span>
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

        {/* 选择卡：面包屑（自动层收成弱提示）→ 楼栋卡片网格 / 房间号输入 */}
        <Surface accent="wallet" className="px-4 py-4">
          <div className="flex items-baseline justify-between gap-3">
            <p className="text-body font-medium text-text">查询房间余额</p>
            {complete && (
              <Button variant="ghost" size="sm" onClick={resetAll}>
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
            /* 未选片区：矮提示条代替大空态卡——左列初始高度收敛 */
            <div className="mt-3 flex items-start gap-2.5 rounded-inner border border-line bg-surface-2 px-3 py-3">
              <Zap aria-hidden className="text-wallet mt-0.5 size-4 shrink-0" />
              <div className="min-w-0">
                <p className="text-body text-text">选择缴费片区后开始查询</p>
                <p className="mt-0.5 text-caption text-text-2">
                  选好片区会自动带出校区，再点楼栋卡片、填房间号。
                </p>
              </div>
            </div>
          ) : (
            <>
              {/* 面包屑：手动选过的层可点回退；自动选定的层收成弱提示（不占交互位） */}
              {steps.length > 0 && (
                <div className="mt-3 flex flex-wrap items-center gap-x-2 gap-y-1">
                  {steps.map((s, i) => {
                    const auto = autoLevels.includes(s.level);
                    const lvName = levels.find((l) => l.level === s.level)?.name;
                    return (
                      <span key={`${s.code}-${i}`} className="flex items-center gap-2">
                        {i > 0 && (
                          <span aria-hidden className="text-caption text-text-2">
                            ›
                          </span>
                        )}
                        {auto ? (
                          <span className="text-caption text-text-2">
                            {lvName ? `${lvName} · ` : ""}
                            {s.name || s.value}（自动）
                          </span>
                        ) : (
                          <button
                            type="button"
                            className="rounded-control px-1.5 py-1 text-caption text-text-2 underline-offset-4 hover:text-text hover:underline"
                            onClick={() => backTo(i)}
                          >
                            {s.name || s.value}
                          </button>
                        )}
                      </span>
                    );
                  })}
                </div>
              )}

              {/* 当前该选的一级（level 名来自服务端 map.total） */}
              {phase !== "error" && level && (
                <div className="mt-3">
                  <p className="mb-1 text-caption text-text-2" id="ecard-elec-level-name">
                    {level.name}
                  </p>
                  {options.length > 0 && !isInputLevel ? (
                    /* 楼栋卡片网格：单击即进 */
                    <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
                      {options.map((o) => (
                        <button
                          key={o.value}
                          type="button"
                          aria-label={`选择${o.label}`}
                          className="min-h-9 rounded-inner border border-line bg-surface px-2.5 py-2 text-left transition-[border-color,box-shadow] duration-[var(--dur-fast)] ease-out-soft hover:border-line-strong hover:shadow-card"
                          onClick={() => pickChoice(o)}
                        >
                          <span className="block truncate text-body text-text">{o.label}</span>
                          {seenKeys.has(`${o.level}:${o.value}`) && (
                            <span className="mt-0.5 block text-caption text-text-2">
                              上次查过
                            </span>
                          )}
                        </button>
                      ))}
                    </div>
                  ) : isInputLevel ? (
                    <>
                      <form
                        className="flex gap-2"
                        onSubmit={(e) => {
                          e.preventDefault();
                          submitRoom(roomText);
                        }}
                      >
                        <Input
                          id="ecard-elec-room"
                          autoFocus
                          aria-describedby="ecard-elec-level-name"
                          value={roomText}
                          inputMode="numeric"
                          autoComplete="off"
                          placeholder={`请输入${level.name}（例如 101）`}
                          disabled={phase === "loading"}
                          onChange={(e) => setRoomText(e.target.value)}
                        />
                        <Button type="submit" disabled={phase === "loading"}>
                          查询
                        </Button>
                      </form>
                      {recentRooms.length > 0 && (
                        <div className="mt-2 flex flex-wrap items-center gap-1.5">
                          <span className="text-caption text-text-2">最近查过：</span>
                          {recentRooms.map((r) => {
                            // 显示房间号（路径末级的 value），不是用户自起的房间名
                            const room = r.path[r.path.length - 1];
                            return (
                              <Button
                                key={roomPathKey(r.feeitemId, r.path)}
                                variant="outline"
                                size="sm"
                                onClick={() => openRecent(r)}
                              >
                                {room?.value || room?.name || "—"}
                              </Button>
                            );
                          })}
                        </div>
                      )}
                    </>
                  ) : (
                    <p className="text-body text-text-2">该层暂无可选项，请点上一级重选。</p>
                  )}
                </div>
              )}

              {phase === "loading" && options.length === 0 && (
                <div aria-hidden className="mt-3 h-9 w-full animate-pulse rounded bg-line" />
              )}

              {phase === "error" && (
                <div className="mt-3 flex items-center justify-between gap-3">
                  <p className="min-w-0 text-body text-text-2">{error}</p>
                  <Button
                    variant="outline"
                    size="sm"
                    className="shrink-0"
                    onClick={() => void runQuery(areaId, steps)}
                  >
                    重试
                  </Button>
                </div>
              )}
            </>
          )}

          {/* 常用房间（与「查询房间余额」同一张卡）：它本质是查询入口，融合进来少一张卡 */}
          <div className="mt-4 border-t border-line pt-3">
            <div className="flex items-baseline justify-between gap-3">
              <p className="text-body font-medium text-text">常用房间</p>
              {sortedRooms.length > 0 && (
                <span className="text-caption text-text-2">共 {sortedRooms.length} 个</span>
              )}
            </div>
            {sortedRooms.length === 0 ? (
              <EmptyState
                compact
                icon={History}
                domain="wallet"
                title="还没有常用房间"
                hint="查到房间后点「保存房间」，之后可一键查余额。"
              />
            ) : (
              <ul className="mt-2 divide-y divide-line">
                {sortedRooms.map((r) => (
                  <li key={r.id} className="flex items-center gap-3 py-2.5">
                    <span className="min-w-0 flex-1">
                      <span className="flex items-center gap-1.5">
                        {r.bound === true && (
                          <span
                            aria-label="我的宿舍"
                            className="text-wallet inline-flex items-center"
                          >
                            <Home aria-hidden className="size-3.5" />
                          </span>
                        )}
                        <span className="block truncate text-body text-text">{r.label}</span>
                      </span>
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
                      onClick={() => openRoom(r)}
                    >
                      查余额
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-pressed={r.bound === true}
                      onClick={() => void toggleBind(r)}
                    >
                      {r.bound === true ? "解绑" : "绑定"}
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
            {roomsNotice && <p className="mt-2 text-caption text-alert">{roomsNotice}</p>}
            {!authed && rooms.length > 0 && (
              <p className="mt-2 text-caption text-text-2">登录后才能查询，房间保存在本机。</p>
            )}
          </div>
        </Surface>

        {/* 结果卡：主数字 = 后端结构化余额 balanceYuan（null ⇒ 无数据，绝不当 0） */}
        {view && (
          <Surface accent="wallet" className="px-4 py-4">
            <div className="flex items-baseline justify-between gap-3">
              <p className="text-body font-medium text-text">房间余额</p>
              {complete && (
                <span className="min-w-0 truncate text-caption text-text-2">
                  {steps.map((s) => s.name || s.value).join(" · ")}
                </span>
              )}
            </div>

            {view.tip ? (
              <p className="mt-3 text-body text-alert">{view.tip}</p>
            ) : view.fields.length === 0 && view.money == null && view.balanceYuan == null ? (
              <p className="mt-3 text-body text-text-2">该房间暂无用电信息</p>
            ) : (
              <>
                {view.balanceYuan != null ? (
                  <>
                    <p className="mt-3 text-caption text-text-2">当前余额</p>
                    <p
                      className={cn(
                        "tabular-num text-display font-semibold",
                        view.balanceYuan < 0 ? "text-alert" : "text-text",
                      )}
                    >
                      ¥ {view.balanceYuan.toFixed(2)}
                    </p>
                    {view.balanceYuan < 0 && (
                      <p className="mt-1 text-caption text-alert">余额为负，已欠费</p>
                    )}
                  </>
                ) : (
                  !view.tip && (
                    <p className="mt-3 text-body text-text-2">
                      余额无数据：学校返回的文本里没有可识别的金额（未采到 ≠ 0 元）。
                    </p>
                  )
                )}

                {/* 服务端明细行：通用渲染 + 折行 + 负数标红 */}
                {view.fields.length > 0 && (
                  <dl className="mt-3 space-y-1.5">
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
              </>
            )}

            {area?.maxmoney != null && (
              <p className="mt-2 text-caption text-text-2">单笔充值限额 ¥ {area.maxmoney}</p>
            )}

            {/* 保存为常用房间（用户起名，留空则用「楼栋 房间号」） */}
            {complete && !currentRoom && (
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
            {complete && currentRoom && (
              <p className="mt-2 text-caption text-text-2">已存为常用房间：{currentRoom.label}</p>
            )}

            <div className="mt-3 flex flex-wrap items-center gap-2">
              <Button size="sm" onClick={() => void recharge()}>
                <Zap />
                去官网充值
              </Button>
              <Button
                size="sm"
                variant="outline"
                aria-busy={snapBusy}
                disabled={snapBusy}
                onClick={() => void runSnapshot()}
              >
                立即采集
              </Button>
              {currentRoom?.bound === true ? (
                <span className="text-wallet inline-flex items-center gap-1 rounded-control bg-surface-2 px-2 py-1.5 text-caption font-medium">
                  <Home aria-hidden className="size-3.5" />
                  我的宿舍
                </span>
              ) : (
                <Button
                  size="sm"
                  variant="outline"
                  aria-busy={bindBusy}
                  disabled={bindBusy}
                  onClick={() => void bindCurrent()}
                >
                  绑定为我的宿舍
                </Button>
              )}
            </div>
            {notice && (
              <p className={cn("mt-2 text-caption", noticeBad ? "text-alert" : "text-text-2")}>
                {notice}
              </p>
            )}
          </Surface>
        )}

        {/* 充值（M3.1 批 D）：房间选全且视图无 tip 才出现——金额 → 风险声明 → 支付方式 →
            免密/安全键盘 → 轮询。房间路径交给后端合成 third_party（PII 不出后端）。 */}
        {complete && area && view && !view.tip && (
          <RechargeFlow
            feeitem={area}
            roomLabel={steps.map((s) => s.name || s.value).join(" · ")}
            path={steps}
          />
        )}
      </div>

      {/* ── 右列：数据侧栏 ── */}
      <div className="flex min-w-0 flex-col gap-3">
        <ElectricityTrendCard
          roomId={boundRoom?.id ?? null}
          roomLabel={boundRoom?.label ?? null}
          reloadTick={trendTick}
        />

        <ElectricityPaymentsCard authed={authed} openLoginDialog={openLoginDialog} />
      </div>
    </div>
  );
}

/**
 * 输入级判定（`recentRooms` 的守卫用；与渲染处 `isInputLevel` 同口径）：
 * 最后一层 + 片区 `lastLevelIsInput`。提出来是因为 `recentRooms` 在 `isInputLevel`
 * 声明之前求值（两者都在组件体顶层，无 Hook 顺序问题）。
 */
function isInputLevelSafe(
  levels: ElectricityLevel[],
  steps: RoomStep[],
  area: FeeItem,
): boolean {
  return steps.length === levels.length - 1 && area.lastLevelIsInput === true;
}
