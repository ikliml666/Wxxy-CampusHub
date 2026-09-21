import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { BrowserHistoryItem, BrowserNavState } from "../shared/types";
import { browserHostOf } from "../shared/constants";
import {
  appBrowserNavigate,
  closeAppBrowser,
  invokeCommand,
  openInAppBrowser,
} from "../shared/tauriApi";

// ==================== 内置浏览器 store（Task 4） ====================
//
// 职责边界：本 store 只提供 state/actions。browser://load / browser://blocked /
// browser://nav 三个事件的 listen 由 Task 5 的 BrowserOverlay 组件挂载后调
// setNav / setLoading / setBlocked 回写；browser://opened 的常驻 listen 由
// BrowserEventsBridge（挂 AppShell）调 storeSyncOpen 回写，此处不写 listen。
//
// canBack/canForward 首批恒 false：BrowserNavState 已预留字段、setNav 可设置，
// 但本批没有导航栈事件源（browser://nav 只带 url），工具栏按钮按禁用态渲染；
// 待下一批补导航栈契约后由事件回调传入真实值。

/** history 上限（persist 落盘条数上限，最新在前）。 */
const HISTORY_LIMIT = 20;

/** loading 保险时长（ms）：到期仍 loading 则强制复位（I2：browser://load
 * finished 丢失 / 副 webview 异常时进度条卡死的兜底，slow 10s 提示条之外的
 * 第二道保险）。 */
const LOADING_WATCHDOG_MS = 30_000;

/** 30s 保险 timer 句柄（模块级运行时状态，非 persist 字段——partialize 只落
 * 盘 history，天然不进 localStorage）。 */
let loadingWatchdog: ReturnType<typeof setTimeout> | null = null;

/** 撤销 30s 保险（loading 复位 / 弹层关闭时）。 */
const clearLoadingWatchdog = () => {
  if (loadingWatchdog != null) {
    clearTimeout(loadingWatchdog);
    loadingWatchdog = null;
  }
};

/** 重置 30s 保险：清旧 timer 再计时（browserOpen inApp 分支 / browserNav 置
 * loading 处调用；到期仍 loading 才强制复位，不影响正常 finished 回写）。 */
const armLoadingWatchdog = () => {
  clearLoadingWatchdog();
  loadingWatchdog = setTimeout(() => {
    loadingWatchdog = null;
    if (useBrowserStore.getState().loading) {
      useBrowserStore.setState({ loading: false });
    }
  }, LOADING_WATCHDOG_MS);
};

/** 同 url 去重置顶 + 截断到上限。 */
const pushHistory = (history: BrowserHistoryItem[], url: string): BrowserHistoryItem[] => {
  const item: BrowserHistoryItem = { url, host: browserHostOf(url), at: Date.now() };
  return [item, ...history.filter((h) => h.url !== url)].slice(0, HISTORY_LIMIT);
};

export const useBrowserStore = create<{
  /** 弹层是否打开 */
  open: boolean;
  /** 当前导航态（canBack/canForward 首批恒 false，见文件头注释） */
  nav: BrowserNavState;
  /** 页面加载中（browser://load started → true / finished → false，回写在 Task 5） */
  loading: boolean;
  /** 最近一次被白名单拦截的 url（null = 无；Task 5 事件回写） */
  blockedUrl: string | null;
  /** 最近一次失败信息（open/nav 降级失败等），供 UI 显示；null = 无 */
  errorMsg: string | null;
  /** 访问历史（persist 唯一落盘字段；同 url 去重置顶，上限 20） */
  history: BrowserHistoryItem[];
  /** 打开内置浏览器；域外应用自动降级系统浏览器 */
  browserOpen: (url: string) => Promise<void>;
  /** 关闭弹层并复位弹层态（history 保留） */
  browserClose: () => Promise<void>;
  /** 弹层内导航到新 url */
  browserNav: (url: string) => Promise<void>;
  /** 事件/工具栏回写导航态（patch 合并） */
  setNav: (patch: Partial<BrowserNavState>) => void;
  setLoading: (b: boolean) => void;
  setBlocked: (url: string | null) => void;
  /** 清空 errorMsg（状态提示条的关闭按钮回写，Task 5） */
  clearError: () => void;
  /** Rust 侧直开副 webview（browser://opened 事件）的 store 同步（C1）：行为
   * 等价 browserOpen 的 inApp 分支但**不调任何 Rust 命令**（webview 已建好）。 */
  storeSyncOpen: (url: string) => void;
}>()(
  persist(
    (set) => ({
      open: false,
      nav: { url: "", canBack: false, canForward: false },
      loading: false,
      blockedUrl: null,
      errorMsg: null,
      history: [],

      browserOpen: async (url) => {
        const res = await openInAppBrowser(url);
        if (!res.success) {
          set({ errorMsg: res.message ?? "打开浏览器失败" });
          return;
        }
        const data = res.data;
        if (!data) {
          set({ errorMsg: "打开浏览器失败：后端未返回结果" });
          return;
        }
        if (data.inApp) {
          // 内置浏览器接管：loading 置 true，等 browser://load finished 回写（Task 5）
          set({
            open: true,
            nav: { url: data.url, canBack: false, canForward: false },
            loading: true,
            blockedUrl: null,
            errorMsg: null,
          });
          set((s) => ({ history: pushHistory(s.history, data.url) }));
          // I2：30s 保险兜进度条卡死（finished 丢失等异常不再永久 loading）
          armLoadingWatchdog();
          return;
        }
        // 降级：inApp=false = 域外应用走系统浏览器；open_app 的 isCas 为旧契约
        // 占位参数，域外应用恒 false（Rust 侧按协议白名单校验 url）。
        const fallback = await invokeCommand("open_app", { url, isCas: false });
        if (!fallback.success) {
          set({ errorMsg: fallback.message ?? "在系统浏览器中打开失败" });
        } else {
          set({ errorMsg: null });
        }
      },

      browserClose: async () => {
        // 命令失败不再回写 errorMsg：弹层即将关闭，UI 不再展示；弹层态一律复位。
        await closeAppBrowser();
        // I2：弹层关闭即撤 30s 保险（loading 已复位，timer 无意义）
        clearLoadingWatchdog();
        set({
          open: false,
          nav: { url: "", canBack: false, canForward: false },
          loading: false,
          blockedUrl: null,
          errorMsg: null,
        });
      },

      browserNav: async (url) => {
        set({ loading: true, errorMsg: null });
        // 成功时 loading 保持 true，等 browser://load finished 回写（Task 5）
        // I2：导航侧同挂 30s 保险（重开导航时清旧 timer 再计时）
        armLoadingWatchdog();
        const res = await appBrowserNavigate(url);
        if (!res.success) {
          set({ loading: false, errorMsg: res.message ?? "页面导航失败" });
        }
      },

      setNav: (patch) => set((s) => ({ nav: { ...s.nav, ...patch } })),
      setLoading: (b) => {
        // I2：置 true 也重新 arm——页内导航/刷新（browser://load started 回写）
        // 同样可能丢 finished，兜底须对全部 loading 场景生效；置 false 撤保险
        if (b) armLoadingWatchdog();
        else clearLoadingWatchdog();
        set({ loading: b });
      },
      setBlocked: (url) => set({ blockedUrl: url }),
      clearError: () => set({ errorMsg: null }),

      // C1：Rust 直开副 webview（电费 open_recharge_in_browser 等不经本 store
      // 的入口）后，BrowserEventsBridge 收 browser://opened 调此 action。行为
      // 等价 browserOpen 的 inApp 分支但不调 Rust 命令；与命令链路同开时幂等
      // （同值重置 + pushHistory 去重置顶）。
      storeSyncOpen: (url) => {
        set({
          open: true,
          nav: { url, canBack: false, canForward: false },
          loading: true,
          blockedUrl: null,
          errorMsg: null,
        });
        set((s) => ({ history: pushHistory(s.history, url) }));
        armLoadingWatchdog();
      },
    }),
    {
      name: "campushub-browser",
      version: 1,
      // 只落盘 history：弹层开关/导航/加载/拦截/报错都是一次性运行时状态，不进 localStorage
      partialize: (s) => ({ history: s.history }),
      // v1 首版无历史版本可迁移，空实现占位；后续升版在此追加（惯例同 uiStore 版本纪律）
      migrate: (persisted) => persisted as { history: BrowserHistoryItem[] },
    },
  ),
);
