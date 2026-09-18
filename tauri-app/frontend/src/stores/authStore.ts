import { create } from "zustand";
import { persist } from "zustand/middleware";
import { invokeCommand } from "@/shared/tauriApi";
import type { CommandResult } from "@/shared/types";

// 登录态单一来源：status 三态（unknown=启动探测中 / guest=未登录 / authed=已登录）。
// 只持久化 username+displayName（重启后先显示上次身份，check_session 兜底纠正）；
// 头像与账号列表不持久化，由命令面拉取（凭据与 DPAPI 密文永不进 localStorage）。
export type AuthStatus = "unknown" | "guest" | "authed";

export interface SavedAccount {
  username: string;
  lastLogin?: string;
  displayName?: string;
}

/** login 系命令成功时 data 的形态（auth.rs::LoginData）。 */
export interface LoginOk {
  username: string;
  displayName: string;
}

/** CAPTCHA_MANUAL 时 data 的形态（auth.rs::CaptchaData）。 */
export interface CaptchaPayload {
  uid: string;
  pngBase64: string;
}

/** 头像命令统一返回形态（profile.rs::AvatarData）。 */
export interface AvatarPayload {
  imageBase64: string | null;
  source: "local" | "official" | null;
}

/** 登录成功判定：untagged 双形态里靠 "username" 收窄。 */
export function isLoginOk(d: unknown): d is LoginOk {
  return !!d && typeof d === "object" && "username" in d;
}

/** 手动验证码数据判定：untagged 双形态里靠 "uid" 收窄。 */
export function isCaptchaPayload(d: unknown): d is CaptchaPayload {
  return !!d && typeof d === "object" && "uid" in d;
}

interface AuthState {
  status: AuthStatus;
  username: string | null;
  displayName: string | null;
  avatarBase64: string | null;
  avatarSource: "local" | "official" | null;
  accounts: SavedAccount[];

  checkSession: () => Promise<void>;
  refreshAccounts: () => Promise<void>;
  refreshAvatar: () => Promise<void>;
  login: (username: string, password: string) => Promise<CommandResult<unknown>>;
  loginManual: (args: {
    username: string;
    password: string;
    captchaUid: string;
    captchaCode: string;
  }) => Promise<CommandResult<unknown>>;
  loginSaved: (username: string) => Promise<CommandResult<unknown>>;
  logout: () => Promise<void>;
  uploadAvatar: (base64: string) => Promise<CommandResult<unknown>>;
  uploadOfficialAvatar: (imageDataUrl: string) => Promise<CommandResult<unknown>>;
  syncOfficialAvatar: () => Promise<CommandResult<unknown>>;
  clearAvatar: () => Promise<CommandResult<unknown>>;
  removeAccount: (username: string) => Promise<CommandResult<unknown>>;
}

/** 头像命令返回值 → store 字段（成功且带 data 时才覆盖）。 */
function applyAvatar(
  r: CommandResult<AvatarPayload>,
  set: (patch: Partial<AuthState>) => void,
): CommandResult<AvatarPayload> {
  if (r.success && r.data) {
    set({
      avatarBase64: r.data.imageBase64 ?? null,
      avatarSource: r.data.source ?? null,
    });
  }
  return r;
}

export const useAuthStore = create<AuthState>()(
  persist(
    (set, get) => ({
      status: "unknown",
      username: null,
      displayName: null,
      avatarBase64: null,
      avatarSource: null,
      accounts: [],

      // 启动探测：不阻塞渲染（status 从 unknown 落到 guest/authed，UI 自行处理两态）
      checkSession: async () => {
        const r = await invokeCommand<{ loggedIn: boolean }>("check_session");
        const loggedIn = r.success && r.data?.loggedIn === true;
        if (loggedIn) {
          const { username, displayName } = get();
          set({ status: "authed", username: username ?? null, displayName });
        } else {
          set({
            status: "guest",
            username: null,
            displayName: null,
            avatarBase64: null,
            avatarSource: null,
          });
        }
        await get().refreshAccounts();
        await get().refreshAvatar();
      },

      refreshAccounts: async () => {
        const r = await invokeCommand<{ accounts: SavedAccount[] }>("list_accounts");
        if (r.success && r.data) set({ accounts: r.data.accounts });
      },

      refreshAvatar: async () => {
        // 游客不展示账号头像：本地与官方头像都属账号资产，退出后回落「锡」占位
        if (get().status !== "authed") {
          set({ avatarBase64: null, avatarSource: null });
          return;
        }
        const r = await invokeCommand<AvatarPayload>("get_avatar");
        if (r.success && r.data) {
          set({
            avatarBase64: r.data.imageBase64 ?? null,
            avatarSource: r.data.source ?? null,
          });
        }
      },

      login: async (username, password) => {
        const r = await invokeCommand<unknown>("login", {
          account: { username, password },
        });
        if (r.success && isLoginOk(r.data)) {
          set({
            status: "authed",
            username: r.data.username,
            displayName: r.data.displayName || r.data.username,
          });
          await get().refreshAccounts();
          await get().refreshAvatar();
          // 首次登录且本地无头像时，后台补一次官方头像（失败静默，不打扰登录流程）
          if (!get().avatarBase64) void get().syncOfficialAvatar();
        }
        return r;
      },

      loginManual: async ({ username, password, captchaUid, captchaCode }) => {
        const r = await invokeCommand<unknown>("login_manual", {
          account: { username, password, captchaUid, captchaCode },
        });
        if (r.success && isLoginOk(r.data)) {
          set({
            status: "authed",
            username: r.data.username,
            displayName: r.data.displayName || r.data.username,
          });
          await get().refreshAccounts();
          await get().refreshAvatar();
        }
        return r;
      },

      loginSaved: async (username) => {
        const r = await invokeCommand<unknown>("login_saved", {
          account: { username },
        });
        if (r.success && isLoginOk(r.data)) {
          set({
            status: "authed",
            username: r.data.username,
            displayName: r.data.displayName || r.data.username,
          });
          await get().refreshAccounts();
          await get().refreshAvatar();
          if (!get().avatarBase64) void get().syncOfficialAvatar();
        }
        return r;
      },

      logout: async () => {
        await invokeCommand("logout");
        set({
          status: "guest",
          username: null,
          displayName: null,
          avatarBase64: null,
          avatarSource: null,
        });
      },

      uploadAvatar: async (base64) => {
        const r = await invokeCommand<AvatarPayload>("set_avatar", {
          imageBase64: base64,
        });
        return applyAvatar(r, set);
      },

      // 学校上传：传完整 data URL（data:image/jpeg;base64,…），成功后返回的
      // AvatarData 即服务端最新头像，applyAvatar 直接落 store。
      uploadOfficialAvatar: async (imageDataUrl) => {
        const r = await invokeCommand<AvatarPayload>("upload_official_avatar", {
          imageDataUrl,
        });
        return applyAvatar(r, set);
      },

      syncOfficialAvatar: async () => {
        const r = await invokeCommand<AvatarPayload>("sync_official_avatar");
        return applyAvatar(r, set);
      },

      clearAvatar: async () => {
        const r = await invokeCommand<AvatarPayload>("clear_avatar");
        return applyAvatar(r, set);
      },

      removeAccount: async (username) => {
        const r = await invokeCommand("remove_account", { username });
        if (r.success) await get().refreshAccounts();
        return r;
      },
    }),
    {
      name: "campushub-auth",
      partialize: (s) => ({ username: s.username, displayName: s.displayName }),
    },
  ),
);
