# M0+M1 基础设施实施计划（脚手架 + CAS 登录闭环）· v2 评审修订版

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
> 执行分身按任务逐个领取：只读自己任务 + Global Constraints + Spec；接口以各任务 Interfaces 块为准。
> v2（2026-09-17）：经 deepseek-flash 独立评审修订——5 P0（workspace 登记/golden 固化/会话持久化/beforeDevCommand/shadcn CLI）+ 10 P1 + 14 P2 全部消化；golden 测试值由评审员独立 BigInt 实现对拍 rsa30.js 实测固化。

**Goal:** 搭起 Tauri 2 + React 19 桌面脚手架（含域色 Dock 导航），并完成 CAS 登录端到端闭环（含算术验证码自动识别、DPAPI 多账号存储、会话持久化与过期检测）。

**Architecture:** 根 Cargo workspace 汇聚 `crates/*`（协议核心，无 Tauri 依赖，安卓可复用）与 `tauri-app/src-tauri`（接线层）；前端 `tauri-app/frontend`（React 19 + Tailwind v4 + shadcn 组件源码入库 + zustand），IPC 唯一出口 `tauriApi.ts`。CAS 协议实现在 `crates/campus-auth`，golden 测试对拍已验证的 JS 探针。

**Tech Stack:** Tauri 2 / Rust 2021 / React 19 / TypeScript 5 strict / Vite ^6 / Tailwind CSS v4（@tailwindcss/vite）/ shadcn 组件（button/input/card/tooltip 源码入库）/ zustand / framer-motion ^12 / gsap ^3.15 / lucide-react ^0.446 / reqwest 0.12（rustls + cookies）/ num-bigint-dig 0.8 / image 0.25（仅 png）

**Spec:**
- `PLAN.md`（里程碑与已确认决策）
- `docs/design/frontend-design.md`（§4 Dock 规格 / §6 token / §7 选型）
- `docs/cas-recon/REPORT.md`（CAS 协议全定案：端点、RSA 参数、错误码、请求样例）
- `docs/cas-recon/rsa30.js`（线上 RSA 原样模块，**导出仅 `{a,b,c,d}`**：`a=setMaxDigits, b=RSAKeyPair, c=encryptedString`；正确用法见 `cas.js:13-17`）、`probe.js`（kaptcha 实现在其 50-55 行）、`cas.js`（kaptcha 会话参照）
- 参考项目 `../Wxxy-CampusLogin/tauri-app/`（Tauri 2 配置、CommandResult 契约 `src-tauri/src/infra/state/mod.rs`、DPAPI 裸 FFI `src-tauri/src/account/crypto.rs`、异步命令纪律 `commands/login.rs:87`）

## Global Constraints（每个任务隐含遵守）

- Windows 先行；包管理器一律 **npm**（对齐参考项目，不引入 pnpm/yarn）。
- TypeScript `strict: true`；React 19；不引入 Ant Design / Arco / Semi。
- IPC 唯一出口 `tauri-app/frontend/src/shared/tauriApi.ts`；所有命令返回 `CommandResult<T>`。**Rust 侧统一形态**：`#[serde(rename_all="camelCase")] struct CommandResult<T> { success: bool, message: Option<String>, data: Option<T> }`；**所有命令参数/返回 DTO 一律 `#[serde(rename_all = "camelCase")]`**（否则前端 camelCase 参数收不到）。
- `crates/*` 协议核心**不得依赖** tauri 等桌面运行时；`src-tauri` 只做接线与状态管理。
- 敏感纪律：密码/凭据不明文进日志、错误消息、commit；日志输出一律脱敏（用户名打码、密码永不出现）。
- Apache-2.0 合规三件套已就位（campus-schedule），campus-auth 为本项目原创。
- 分支 `feat/sess-b27a282c-m0-m1`；**commit 一律 `git add <明确路径>`，禁止 `git add -A`**（仓库根有未跟踪的用户脚本「一键填分4.user.js」，不得误带）。
- 会话保活边界：M1 做「启动 `check_session`（jar 检查 + 门户轻探测）+ 请求失败降级回登录页」；定时保活轮询属 M5。
- 验收命令以实际输出为准；首次 cargo 编译 tauri 较慢属正常。
- 凭据文件 `账号与密码.txt` 已在 .gitignore；集成测试从 `CAMPUS_HUB_CREDS` 环境变量指定的文件读取，禁止硬编码。
- 文件路径一律相对仓库根书写（`tauri-app/frontend/...`），不得在仓库根另造 `frontend/`。
- 目录范围声明：M1 只建 `src-tauri/{commands,infra,account}` 与 `frontend/{src/shared,src/stores,src/components,src/panels}`；参考项目其余目录（auth/config/network/hooks 等）M2+ 按需再建。
- workspace 成员的 Cargo.toml **不得写 `[profile.*]`**（根 workspace 已有，写了会被忽略并告警）。

---

### Task 1: 根 Cargo workspace

**Files:**
- Create: `Cargo.toml`（仓库根）
- Delete: `crates/campus-schedule/Cargo.lock`（workspace 统一到根 lock）

**Interfaces:**
- Produces: workspace members 起步 = `["crates/campus-schedule"]`；后续 Task 6 追加 `crates/campus-auth`、Task 9 追加 `tauri-app/src-tauri`（**同一时间只有一个任务改根 Cargo.toml**，见派单编排）。

- [ ] **Step 1: 写根 Cargo.toml**

```toml
[workspace]
resolver = "2"
members = ["crates/campus-schedule"]

[profile.release]
lto = "thin"
```

- [ ] **Step 2: 并入 campus-schedule 并验证**

```bash
rm crates/campus-schedule/Cargo.lock
cargo test -p campus-schedule
```
Expected: 12 个测试全通过；根目录生成 `Cargo.lock`。
- [ ] **Step 3: Commit** `chore: 根 Cargo workspace（并入 campus-schedule）`

---

### Task 2: tauri-app 前端脚手架（Vite + React 19 + TS + Tailwind v4）

**Files:**
- Modify: `.gitignore`（追加 `dist/`、`node_modules/`）
- Create: `tauri-app/frontend/{package.json, vite.config.ts, tsconfig.json, tsconfig.node.json, index.html}`
- Create: `tauri-app/frontend/src/{main.tsx, App.tsx, index.css, vite-env.d.ts}`

**Interfaces:**
- Produces: 可 `npm run build` 的空应用；`index.css` 起步仅 `@import "tailwindcss";`（token 由 Task 3 填）。

- [ ] **Step 1: .gitignore 追加 `dist/` 与 `node_modules/`**（`tauri-app/frontend/dist` 是 frontendDist 指向的构建产物，必须挡在 git 外）。

- [ ] **Step 2: 初始化 npm 项目并安装依赖（版本锁定，对齐参考项目已验证组合）**

```bash
mkdir -p tauri-app/frontend && cd tauri-app/frontend
npm init -y
npm i react@^19 react-dom@^19 @tauri-apps/api@^2 zustand@^5 framer-motion@^12 gsap@^3.15 lucide-react@^0.446 clsx tailwind-merge
npm i -D typescript@^5 vite@^6 @vitejs/plugin-react@^4 tailwindcss@^4 @tailwindcss/vite@^4 @types/react@^19 @types/react-dom@^19 @tauri-apps/cli@^2
```
package.json scripts: `"dev": "vite"`, `"build": "tsc -b && vite build"`, `"tauri": "tauri"`；`"type": "module"`。

- [ ] **Step 3: 配置 vite / ts**

`vite.config.ts`：
```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
});
```
`tsconfig.json`：strict、`"moduleResolution": "bundler"`、jsx react-jsx、include src。

- [ ] **Step 4: Outfit 字体** — `npm i @fontsource/outfit`，把包内 latin-400/500/600 woff2 复制到 `tauri-app/frontend/public/fonts/`；`index.css` 加对应 `@font-face`（`font-display: swap`）；在 `THIRD-PARTY-NOTICES.md` 补 OFL 许可条目（随字体分发许可文本，拷 `node_modules/@fontsource/outfit` 内的 LICENSE）。
- [ ] **Step 5: 最小入口** — `index.html`（`<div id="root">` + `/src/main.tsx`）、`src/main.tsx`（createRoot 渲染 App）、`src/App.tsx`（渲染 `<div className="min-h-screen bg-bg text-text">锡院助手</div>`）。
- [ ] **Step 6: 验证** `npm run build` — Expected: tsc 零错误 + dist 产出。
- [ ] **Step 7: Commit**（明确路径）`feat(frontend): Vite+React19+TS+Tailwind v4 脚手架`

---

### Task 3: 视觉 token 系统（域色 CSS 变量 + Tailwind @theme）

**Files:**
- Modify: `tauri-app/frontend/src/index.css`
- Create: `tauri-app/frontend/src/shared/cn.ts`

**Interfaces:**
- Produces: Tailwind 类可直接用 `bg-bg text-text text-text-2 bg-surface border-line bg-brand text-wallet text-info text-todo text-sched text-alert`；`cn()`（clsx + tailwind-merge，**全项目唯一 cn**，后续 shadcn 组件的 import 一律指到这里）。本任务同时预埋 shadcn 语义变量映射块（Task 4 组件消费）。

- [ ] **Step 1: index.css 写入 token（值与设计文档 §6 逐字一致）**

```css
@import "tailwindcss";

@custom-variant dark (&:where(.dark, .dark *));

@theme {
  --color-brand: #5b2e90;      /* 锡院紫 */
  --color-bg: #f7f6f9;         /* 纸灰页底 */
  --color-surface: #ffffff;    /* 卡片 */
  --color-text: #1f1b29;       /* 墨紫黑 */
  --color-text-2: #6e6878;     /* 次要文字 */
  --color-line: #e8e5ee;       /* 1px 边线 */
  --color-wallet: #0f9d77;     /* 域绿：一卡通/余额 */
  --color-info: #7c5cbf;       /* 域紫：资讯/公告 */
  --color-todo: #d9822b;       /* 域琥珀：待办 */
  --color-sched: #2b7dbf;      /* 域湖蓝：日程/会议 */
  --color-alert: #d9455b;      /* 低余额/告警 */
  --font-sans: "Segoe UI", "Microsoft YaHei UI", system-ui, sans-serif;
  --font-num: "Outfit", "Segoe UI", sans-serif;
}

:root {
  color-scheme: light;
  /* shadcn 语义变量 → 域色映射（Task 4 组件源码消费这些名字） */
  --background: var(--color-bg);
  --foreground: var(--color-text);
  --primary: var(--color-brand);
  --primary-foreground: #ffffff;
  --secondary: var(--color-bg);
  --secondary-foreground: var(--color-text);
  --muted: var(--color-bg);
  --muted-foreground: var(--color-text-2);
  --accent: var(--color-bg);
  --accent-foreground: var(--color-text);
  --destructive: var(--color-alert);
  --border: var(--color-line);
  --input: var(--color-line);
  --ring: var(--color-brand);
  --radius: 10px;
}
:root.dark { color-scheme: dark; }
.dark {
  --color-bg: #17151c;
  --color-surface: #201d28;
  --color-text: #efedf4;
  --color-text-2: #a29aad;
  --color-line: #35303f;
  --color-brand: #8b5cf6;      /* 深主题品牌提亮一档（设计文档 §6 自评） */
  --color-wallet: #2ec49a;     /* 域色深主题各提亮一档 */
  --color-info: #a48bd9;
  --color-todo: #eda45c;
  --color-sched: #5da3e0;
  --color-alert: #e56b7d;
}
body { @apply bg-bg text-text font-sans antialiased; }
.tabular-num { font-family: var(--font-num); font-variant-numeric: tabular-nums; }
```

- [ ] **Step 2: `src/shared/cn.ts`**

```ts
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";
export function cn(...inputs: ClassValue[]) { return twMerge(clsx(inputs)); }
```

- [ ] **Step 3: 验证** `npm run build` 通过即 token 编译成功。
- [ ] **Step 4: Commit** `feat(frontend): 域色 token 系统（浅/深主题 + shadcn 语义映射 + Outfit）`

---

### Task 4: IPC 契约 + zustand + AppShell + Dock 导航骨架

**Files:**
- Create: `tauri-app/frontend/src/shared/tauriApi.ts`、`tauri-app/frontend/src/shared/types.ts`
- Create: `tauri-app/frontend/src/stores/uiStore.ts`
- Create: `tauri-app/frontend/src/components/AppShell.tsx`、`tauri-app/frontend/src/components/DockNav.tsx`、`tauri-app/frontend/src/components.json`
- Create: `tauri-app/frontend/src/components/ui/{button,input,card,tooltip}.tsx`
- Create: `tauri-app/frontend/src/panels/{TodayPanel,InfoPanel,TodoPanel,SchedulePanel,AppsPanel,WalletPanel,PowerPanel,SettingsPanel}.tsx`（本任务全为占位实现）
- Modify: `tauri-app/frontend/src/App.tsx`

**Interfaces:**
- Produces（后续任务消费，签名冻结）:

```ts
// shared/types.ts
export type CommandResult<T> = { success: boolean; message?: string; data?: T };
// PanelId 冻结 8 项；M2.5 课表页届时追加 "timetable" 需同步改此处 + DOCK_ITEMS + persist 兼容
export type PanelId = "today" | "info" | "todo" | "schedule" | "apps" | "wallet" | "power" | "settings";

// shared/tauriApi.ts —— IPC 唯一出口
export async function invokeCommand<T>(cmd: string, args?: Record<string, unknown>): Promise<CommandResult<T>>;
// 实现: import { invoke } from "@tauri-apps/api/core";
//   invoke 抛错时 catch 后包装为 { success:false, message:String(e) }。

// stores/uiStore.ts (zustand + persist localStorage key "campushub-ui")
export const useUiStore = create<{ activePanel: PanelId; setActivePanel: (p: PanelId) => void; }>()(
  persist((set) => ({ activePanel: "today", setActivePanel: (p) => set({ activePanel: p }) }), { name: "campushub-ui" })
);
```

- Dock 配置常量（`DockNav.tsx` 导出，域色对齐 token）:

```ts
export const DOCK_ITEMS: { id: PanelId; label: string; icon: LucideIcon; color: string }[] = [
  { id: "today",    label: "今日", icon: SunMedium,    color: "var(--color-brand)" },
  { id: "info",     label: "资讯", icon: Newspaper,    color: "var(--color-info)" },
  { id: "todo",     label: "待办", icon: ListChecks,   color: "var(--color-todo)" },
  { id: "schedule", label: "日程", icon: CalendarDays, color: "var(--color-sched)" },
  { id: "apps",     label: "应用", icon: LayoutGrid,   color: "var(--color-sched)" },
  { id: "wallet",   label: "钱包", icon: Wallet,       color: "var(--color-wallet)" },
  { id: "power",    label: "电费", icon: Zap,          color: "var(--color-wallet)" },
  { id: "settings", label: "设置", icon: Settings,     color: "var(--color-text-2)" },
];
```

- [ ] **Step 1: tauriApi + types + uiStore**（按上面签名，含 persist 中间件）。
- [ ] **Step 2: shadcn 组件源码入库（不跑 init CLI，避免其重写 index.css 与生成第二个 cn）** — 手写 `components.json`（`"tailwind": {"css": "src/index.css"}`, `"aliases": {"utils": "@/shared/cn", "components": "@/components"}`）；`npm i @radix-ui/react-tooltip @radix-ui/react-slot`；从 shadcn registry 取 button/input/card/tooltip 源码写入 `src/components/ui/`（`npx shadcn@4.21.0 add button input card tooltip --yes` 可用则用它——前提 components.json 就位；CLI 行为异常就手工从 https://ui.shadcn.com 拷源码），组件内 `cn` import 改指 `@/shared/cn`，语义类（bg-primary/text-foreground 等）消费 Task 3 映射块。
- [ ] **Step 3: DockNav 骨架** — 容器 `fixed bottom-5 left-1/2 -translate-x-1/2 z-30 flex items-center gap-1 rounded-[18px] border border-white/60 bg-white/80 px-3 py-2 shadow-[0_8px_30px_rgb(0_0_0/0.12),0_1px_0_rgb(255_255_255/0.6)_inset] dark:border-white/10 dark:bg-[#201d28]/85`；**不用 backdrop-filter**（性能，设计文档 §4）。每项 `<button aria-label={label}>` 内 lucide 图标 size 18；hover tooltip 用 shadcn Tooltip 且**在 DockNav 顶层包一次 `<TooltipProvider delayDuration={250}>`**（Radix 硬要求）。激活项本任务先静态 `bg-black/5 rounded-full`（域色版在 Task 5）。
- [ ] **Step 4: AppShell** — 顶条：左「⌘K 搜索」按钮占位 + 右侧 🔔/🌓/👤 图标按钮占位；`🌓` 点击 `document.documentElement.classList.toggle("dark")`。内容区 `<main className="pb-28">` 包 AnimatePresence 切面板。
- [ ] **Step 5: 8 个占位面板** — 统一形态：左上 8px 域色角标 + 面板标题 +「建设中」空态；App.tsx 按 activePanel 渲染。
- [ ] **Step 6: 验证** `npm run build`；`npm run dev` 起 1420 端口浏览器目测 Dock/顶条/深浅切换，截图存 `docs/verify/m0-dock.png`。
- [ ] **Step 7: Commit** `feat(frontend): IPC 契约 + AppShell + Dock 导航骨架（8 面板）`

---

### Task 5: Dock 动效完整版（域色胶囊 + 磁吸放大）

**Files:**
- Modify: `tauri-app/frontend/src/components/DockNav.tsx`、`tauri-app/frontend/src/components/AppShell.tsx`

**Interfaces:**
- Consumes: Task 4 的 DOCK_ITEMS / uiStore。
- Produces: 最终交互 = 设计文档 §4（激活胶囊+指示条随 spring 滑动、80px 磁吸、reduced-motion 降级）。

- [ ] **Step 1: 激活态** — framer-motion `<motion.span layoutId="dock-pill">` 圆角胶囊 + 底部 3px 圆点指示条，`transition={{ type: "spring", stiffness: 500, damping: 34 }}`；胶囊与图标激活色 = 该项 `color`（域色），未激活图标 `text-text-2`。
- [ ] **Step 2: 磁吸** — gsap `quickTo` per-item 控制 `scale`（1→1.35）与 `y`（0→-14px）：容器 onMouseMove 计算每项中心距离，<80px 按比例插值目标值；RAF 节流；`onMouseLeave` 全部复位。`matchMedia("(prefers-reduced-motion: reduce)")` 命中则不注册磁吸。
- [ ] **Step 3: 面板切换过渡** — AppShell 中 AnimatePresence `mode="wait"`：进 `y: 8→0, opacity 0→1`（spring），退 0.04s 快出；快速连切用 `useDeferredValue(activePanel)` 只渲染最终面板。
- [ ] **Step 4: 验证** dev 目测胶囊滑动/磁吸/连切不闪烁，截图 `docs/verify/m0-dock-motion.png`。
- [ ] **Step 5: Commit** `feat(frontend): Dock 域色胶囊 + gsap 磁吸 + 面板过渡`

---

### Task 6: crates/campus-auth — RSA 加密模块

**Files:**
- Create: `crates/campus-auth/{Cargo.toml, src/lib.rs, src/rsa.rs, src/error.rs, tests/rsa_golden.rs}`
- Modify: 根 `Cargo.toml`（**members 追加 `crates/campus-auth`**——不登记则 Task 6/7/8 的 `cargo test -p campus-auth` 全部报「not a member」错）

**Interfaces:**
- Produces:
```rust
// crates/campus-auth/src/rsa.rs
pub const CAS_RSA_N_HEX: &str; // 258 hex 含前导 00（见 REPORT.md）
pub fn rsa_encrypt_hex(plaintext: &str) -> Result<String, CampusAuthError>; // 输入含非 ASCII → Err（JS charCodeAt 语义对非 ASCII 不等价，显式拒绝，P2-11）
pub fn cas_token_header(now_ms: u64) -> Result<String, CampusAuthError>; // = rsa_encrypt_hex(&format!("lyasp{now_ms}"))
```
- Cargo 依赖（**本任务一次配齐全部后续所需**）：`num-bigint-dig = "0.8"`、`num-traits`、`thiserror`、`serde = { version = "1", features = ["derive"] }`、`serde_json`、`base64 = "0.22"`、`reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "cookies", "charset"] }`。

**算法事实（REPORT.md 已实测 + 评审员独立 BigInt 实现对拍 rsa30.js 通过）：** modulus 1024 位、e=0x010001、chunkSize=126 字节、明文按 little-endian 组块（块内字节序反转后即大端整数）、块尾补 0 至 126 字节、输出 hex 不补前导零、块间直接拼接。密码 < 126 字节（现实必然）单块。

- [ ] **Step 1: 写失败测试** `tests/rsa_golden.rs` — golden 值已由评审实测固化（对拍 `docs/cas-recon/rsa30.js`，如需复核：`const rsa=require('./rsa30.js'); rsa.a(131); const k=rsa.b('010001','',MOD.slice(2)); rsa.c(k, plaintext)`，导出名只有 a/b/c/d）：

```rust
const GOLDEN: &[(&str, &str)] = &[
    ("lyasp1726500000000", "7678b328a692e0745cd039bb6d0cba0af71f54111ec105b11965fc7430ee46abedb4457bdaf1f79a5c0b0ca4a67af838c5ef1152cfe63baf71021030ea8790575e97847312f2815281dca867461d693e626c2969309ce465b45dc67437f73f3f31944b66dca4a02377082a1bd8678f2435433cac8bc2bcb8bb4cc34a70fc2826"),
    ("Test@Password123",   "7fd7ab7bc2860811f82876860a634f4cf1097ed9e6d0098ac680eb875b1c0fa965758a69bc240a402a982d4293a675295cf4a4df659374600f009052f6ff3d97fb905a9b506efd82c131fb016ec6f9b6639d0419fa615ff53e55e984a22ffc4612124b9ce813b16add489e7b0ef500f40915774e145ee72dca83776519618228"),
];
// 测试1：逐对 assert_eq!(rsa_encrypt_hex(p), h)
// 测试2：非 ASCII 输入（"密码123"）assert Err
```
- [ ] **Step 2: `cargo test -p campus-auth`** — Expected: 编译失败（rsa_encrypt_hex 未定义）。
- [ ] **Step 3: 实现 rsa.rs** — `chunks(126)` → 块尾补零 → `reverse()` → `BigUint::from_bytes_be` → `modpow(&e, &n)` → `to_str_radix(16)` 拼接。
- [ ] **Step 4: `cargo test -p campus-auth`** — Expected: golden 2 对 + 非 ASCII 拒绝全过。
- [ ] **Step 5: Commit** `feat(campus-auth): CAS textbook RSA（golden 固化对拍线上 JS）`

---

### Task 7: crates/campus-auth — CAS 客户端 + 会话 Jar

**Files:**
- Create: `crates/campus-auth/src/{cas.rs, jar.rs, tests dir}`（`tests/cas_parse.rs`、`tests/cas_live.rs`）
- Modify: `crates/campus-auth/src/lib.rs`

**Interfaces:**
- Produces（Task 9/10 消费）:

```rust
pub const CAS_BASE: &str = "https://wxcas.cwxu.edu.cn/lyuapServer";
pub const PORTAL_SERVICE: &str = "https://my.cwxu.edu.cn/shiro-cas";
pub const WEBVPN_SERVICE: &str = "https://webvpn.cwxu.edu.cn/login?cas_login=true";
pub const JWGL_SERVICE: &str = "https://jwgl.cwxu.edu.cn/sso/lyiotlogin";

/// reqwest 0.12 无法读回内部 Jar（私有类型）——自实现 CookieStore trait（reqwest::cookie::CookieStore，set_cookies/cookies 两方法），委托内置 Jar 并记录 (名,值) 供 check_session/持久化使用。
pub struct RecordingJar { inner: reqwest::cookie::Jar, recorded: std::sync::Mutex<Vec<(String, String)>> }
impl RecordingJar {
  pub fn new(Arc<Self> 供 cookie_provider);
  pub fn snapshot(&self) -> Vec<(String, String)>;          // 全部 cookie（供 session.json 持久化，DPAPI 加密由上层做）
  pub fn restore(&self, cookies: &[(String, String)]);      // 启动回填
  pub fn has(&self, name: &str) -> bool;                     // check_session 浅检测用（如 customsid）
}

pub struct CasClient { http: reqwest::Client } // CasClient::new() 内部 .cookie_provider(Arc<RecordingJar>)；UA 用真实 Chrome UA
pub struct CaptchaInfo { pub uid: String, pub png_base64: String, pub kaptcha_type: String } // png_base64 已剥掉 data:image/png;base64, 前缀（probe.js:52 同款处理）
pub struct CasLoginOk { pub tgt: String, pub ticket: String }
pub enum CasLoginError { WrongUserOrPwd, WrongCaptcha, UserLocked, NeedTwoVerify(String), Unknown(String), Network(String) }

impl CasClient {
  pub fn new() -> Result<Self, CampusAuthError>;
  pub fn jar(&self) -> Arc<RecordingJar>;
  pub async fn kaptcha(&self) -> Result<CaptchaInfo, CampusAuthError>;  // GET {CAS_BASE}/kaptcha → {kaptchaType,uid,content}
  pub async fn login(&self, username: &str, password_rsa_hex: &str, captcha_uid: &str, captcha_code: &str) -> Result<CasLoginOk, CasLoginError>;
  // POST {CAS_BASE}/v1/tickets, x-www-form-urlencoded；headers: token=cas_token_header(毫秒)
  // body 六字段: username / password(密文) / service=PORTAL_SERVICE / loginType / id=captcha_uid / code=captcha_code / otpcode（loginType/otpcode 取值按 REPORT.md）
  // 成功: 顶层 {tgt, ticket}；失败映射: NOUSER→WrongUserOrPwd / CODEFALSE→WrongCaptcha / USERLOCK→UserLocked / TWOVERIFY→NeedTwoVerify / 其余→Unknown(原样码)——全集以 REPORT.md 为准
  // 请求体构造抽纯函数 build_login_body(...) 供离线单测
  pub async fn sso_follow(&self, service_url: &str, ticket: &str) -> Result<reqwest::Url, CampusAuthError>; // GET service?ticket= 跟 302 链到落点
  pub async fn portal_probe(&self) -> SessionState; // SessionState::{Alive, Expired}：GET 门户首页，customsid 存在且未被打回 CAS 登录页 = Alive
}
```

- [ ] **Step 1: 写失败测试**（离线，`tests/cas_parse.rs`）：① 错误码 JSON → 枚举映射全覆盖（样例内联自 REPORT.md）；② 顶层 `{tgt,ticket}` 成功解析；③ `build_login_body` 输出含 6 字段且 urlencoded 正确；④ RecordingJar set/restore/has 往返（不起网络）。
- [ ] **Step 2: 实现 cas.rs + jar.rs**。
- [ ] **Step 3: 集成测试 `tests/cas_live.rs`（`#[ignore]`）** — 读 `CAMPUS_HUB_CREDS` 指向的凭据文件（格式 `账号:xxx` / `密码:xxx`，解析写在测试内）：kaptcha → 手动占位答案（Task 8 完成后换 `captcha::solve` 自动识别）→ login → sso_follow(PORTAL_SERVICE) → 断言 `portal_probe()==Alive` 且 jar `has("customsid")`。仅主智能体验收时 `-- --ignored` 运行。
- [ ] **Step 4: `cargo test -p campus-auth`** — 离线测试全过。
- [ ] **Step 5: Commit** `feat(campus-auth): CAS 客户端 + RecordingJar 会话捕获`

---

### Task 8: 算术验证码识别（模板匹配）· 拆 8a 采集标注 / 8b 实现

**Files:**
- Create: `scripts/captcha-collect.mjs`、`scripts/captcha-samples/{NN.png, labels.json}`（labels 进 git，PNG 不进——`.gitignore` 追加 `scripts/captcha-samples/*.png`）
- Create: `crates/campus-auth/src/captcha.rs`、`crates/campus-auth/templates/kaptcha-templates.json`
- Modify: `crates/campus-auth/Cargo.toml`（`image = { version = "0.25", default-features = false, features = ["png"] }`）

**Interfaces:**
- Produces:
```rust
pub struct KaptchaTemplates; // include_str!("../templates/kaptcha-templates.json")
impl KaptchaTemplates { pub fn load() -> Self; }
/// PNG 字节 → 算术答案字符串（如 "63"）。None = 无法识别（上层刷新重试，重试穷尽转手动）。
pub fn solve(png_bytes: &[u8], t: &KaptchaTemplates) -> Option<String>;
```

**实现规格：**
1. `image::load_from_memory` → `to_luma8()`（image 0.25 方法名）→ Otsu 阈值二值化（手写 ~15 行：直方图 + 类间方差）。
2. 垂直投影切分：按列统计前景像素，连续空列切字符；题面「两一位数 + 运算符(+/-/×) + = ?」→ 取前 3 个非空片段；数量对不上 → None（实测 captcha.png 100×25、4 片段，策略成立）。
3. 每片段最近邻缩放 24×24 → 与模板（JSON `{"char":"7","grid":[0/1 ×576]}`）逐位**汉明距离**（576 维 0/1，上限 576；**阈值 90 为 Hamming 口径初值**，在样本上标定）；最小距离 ≥90 或次近距离差 <8% → None。
4. 识别 `d1 op d2` → 求值；`-` 可为负（含负号字符串）。
5. **类覆盖约束**：13 类字符（0-9 + - *）每类模板 ≥5 样本；某类缺模板时该字符识别返回 None → 上层兜底（不硬猜）。

**8a · 采集与标注（主智能体 + 标注分身，与批 1/2 并行，不占执行分身位）：**
- [ ] **Step A1: 采集脚本 `scripts/captcha-collect.mjs`** — 参照 `docs/cas-recon/probe.js:50-55` 的 kaptcha 请求（**cas.js 无 kaptcha 导出**）：GET kaptcha ≥100 次，PNG 存 `scripts/captcha-samples/NN.png`，uid 记入 `samples.json`；**不请求 login**（避免错误计数）。主智能体在校园网环境跑。
- [ ] **Step A2: 标注** — 派多模态分身 Read 样本图，产出 `labels.json`（`{"01.png":"8+3",...}` 算式，不含答案）；主智能体抽查 5 张核对。
- [ ] **Step A3: 模板生成** — campus-auth 内 `#[test] #[ignore] build_templates()`：读 `../../scripts/captcha-samples`（cargo test cwd = crate 根）+ labels.json → 切分 → 每类取 4 张做模板、全部样本按 70/30 划模板集/holdout → 写 `templates/kaptcha-templates.json`。主智能体跑一次生成。

**8b · 实现与评测（执行分身）：**
- [ ] **Step B1: 失败测试先行** `tests/captcha_solve.rs`（`#[ignore]`，依赖 samples）：**分别报告**模板集与 holdout 准确率，双指标均 ≥80% 且 holdout ≥24/30 过关。
- [ ] **Step B2: 实现 captcha.rs** 按「实现规格」。
- [ ] **Step B3: `cargo test -p campus-auth -- --ignored captcha_solve`**。未达 80% → 调阈值/切分；仍不达 → `ponytail:` 注释记录实际正确率，保留手动兜底（登录流程已设计），不阻塞主线。
- [ ] **Step B4: Commit** `feat(campus-auth): 算术验证码模板匹配（模板+双指标评测）`

---

### Task 9: src-tauri 骨架 + 会话状态 + DPAPI 账号存储（M0 冒烟前移到此）

**Files:**
- Create: `tauri-app/src-tauri/{Cargo.toml, build.rs, tauri.conf.json, capabilities/default.json, icons/*}`
- Create: `tauri-app/src-tauri/src/{main.rs, lib.rs, commands/mod.rs, commands/auth.rs, infra/mod.rs, infra/state.rs, account/{mod.rs, crypto.rs, store.rs}}`
- Modify: 根 `Cargo.toml`（**members 追加 `tauri-app/src-tauri`**；此任务期 Task 8 不改根文件，无并行冲突）
- Modify: `tauri-app/src-tauri/Cargo.toml`（**path 依赖 `campus-auth = { path = "../../crates/campus-auth" }`**——缺失则 `cargo check -p campus-hub` 断）

**Interfaces:**
- Consumes: `campus_auth::{CasClient, RecordingJar, rsa, captcha}`（Task 6/7/8b）。
- Produces:
  - `infra/state.rs`: `AppState` 用 **`tokio::sync::Mutex<Option<CasSession>>`**（std Mutex 守卫非 Send，跨 await 编译不过；参考项目 `commands/login.rs:87` 同款教训）。`CasSession { client: CasClient, username: String }`。**锁纪律：锁内只 clone 出 client（reqwest::Client 为 Arc 包装，clone 廉价），drop guard 后再 await**。
  - 会话持久化：登录成功后把 `jar.snapshot()` 序列化写 `%APPDATA%/campushub/session.json`（cookie 值经 DPAPI 加密后再落盘）；启动时回填 `jar.restore(...)` → **「重启保持」成立**（P0-3 修正）。
  - `commands::mod` 注册表：`lib.rs::run()` 内 `invoke_handler![commands::auth::*]`（本任务空实现/占位，Task 10 填肉）+ `commands::account::{list_accounts, login_saved}`（Task 10 填肉）。
  - `account::crypto`: **直接拷贝 `Wxxy-CampusLogin/tauri-app/src-tauri/src/account/crypto.rs` 的裸 FFI 实现**（`extern "system"` + `#[link(name="crypt32")]`，零新依赖；参考项目**不用** windows crate 做 DPAPI）：`dpapi_protect(plain)->Result<String>` / `dpapi_unprotect(b64)->Result<String>`。
  - `account::store`: `%APPDATA%/campushub/accounts.json`：`{ accounts: [{ username, password_b64(DPAPI 密文), last_login, display_name? }] }`；`save_account/load_accounts/remove_account`；密码永不落明文。
  - `capabilities/default.json` **只含**：`{ "identifier": "default", "windows": ["main"], "permissions": ["core:default"] }`（**不得照抄 CampusLogin 的 notification/shell/autostart 权限——插件未装，构建期报 permission not found**）。
  - `tauri.conf.json`: identifier `com.cwxu.campushub`、`productName: "锡院助手"`、`frontendDist: "../frontend/dist"`、`devUrl: "http://localhost:1420"`、`"beforeDevCommand": "cd frontend && npm run dev"`、**`"beforeBuildCommand": "cd frontend && npm run build"`**（默认 cwd 是 tauri-app 根，必须 cd，对齐参考项目写法）；`app.security.csp` 含 `img-src 'self' data:`（验证码以 data URI 内联展示）；窗口 1280×860 min 1080×720。icons 从 `../Wxxy-CampusLogin/tauri-app/src-tauri/icons/` 拷全套占位（bundle.icon 列表一并抄）。
- [ ] **Step 1: 写全套骨架**（Cargo.toml 依赖对齐参考项目：tauri 2、serde、tokio、reqwest rustls+cookies、base64、dirs）。
- [ ] **Step 2: DPAPI 单测**：protect→unprotect roundtrip 相等；密文不含明文子串。
- [ ] **Step 3: store 单测**：临时目录 JSON 往返；密码字段为 base64 且非明文。
- [ ] **Step 4: `cargo test -p campus-hub` + `cargo check -p campus-hub`** — 过（crate 名 `campus-hub`，lib 名 `campus_hub_lib`）。
- [ ] **Step 5: **M0 冒烟前移**：`cd tauri-app/frontend && npm run tauri dev` — Expected: Windows 空窗口弹出（此时前端已是 Task 5 完成态，Dock 可见）。**M0 的 tauri dev 验收在此完成，不留到 Task 11。**
- [ ] **Step 6: Commit** `feat(tauri): src-tauri 骨架 + AppState + DPAPI + 会话持久化`

---

### Task 10: Tauri 命令接线（登录全流程 + 多账号重登）

**Files:**
- Modify: `tauri-app/src-tauri/src/commands/{auth.rs, mod.rs}`、`tauri-app/src-tauri/src/infra/state.rs`

**Interfaces:**
- Produces（前端 tauriApi 对接面；**全部 DTO `#[serde(rename_all="camelCase")]`**）:

```ts
// 命令面（invoke 名 → 参数 → data）
get_captcha()      → { uid: string; pngBase64: string }          // 手动模式用；pngBase64 已是裸 base64（无 data: 前缀），前端自行拼 data:image/png;base64,
login(account: { username: string; password: string })
  → 成功 data: { username: string; displayName: string }
  → 三次自动识别穷尽: success=false, message="CAPTCHA_MANUAL", data: { uid, pngBase64 }（前端切手动模式）
login_manual(account: { username; password; captchaUid; captchaCode }) → 同 login
login_saved(account: { username: string }) → 同 login           // DPAPI 解密已存密码复用 login 流程（一键重登）
check_session()    → { loggedIn: boolean }                        // jar has("customsid") + portal_probe()==Alive 双确认；false 时前端清 session.json
logout()           → null                                          // 清 jar/session.json/accounts 选中态 + 调官方登出语义（ REPORT.md：CAS 全局会话销毁）
list_accounts()    → { accounts: { username: string; lastLogin: string }[] }
```
**登录内部流程**：`kaptcha → captcha::solve → login`；**重试策略（锁号防护，REPORT.md 有连续错误计数）**：仅 `WrongCaptcha` 与 solve 失败时重试，总尝试 ≤3；`WrongUserOrPwd` **绝不自动重试**立即返回；连续错误计数提示（错误码含阈值语义）出现则立即转手动。成功后 `sso_follow(PORTAL_SERVICE)` + `save_account`(DPAPI) + session.json 落盘 + AppState 写入。日志宏一律脱敏。
- [ ] **Step 1: 实现命令 + state 接线**；错误映射：`WrongUserOrPwd → "账号或密码错误"`、`UserLocked → "账号已锁定"`、其余原样码。
- [ ] **Step 2: `cargo check -p campus-hub` + `cargo test -p campus-hub`** 过。
- [ ] **Step 3: Commit** `feat(tauri): CAS 登录命令接线（自动验证码+手动兜底+一键重登）`

---

### Task 11: 登录页 UI + 今日页骨架 + 端到端验证

**Files:**
- Create: `tauri-app/frontend/src/panels/LoginPanel.tsx`
- Modify: `tauri-app/frontend/src/App.tsx`（mount 时 `check_session`：false → LoginPanel）、`tauri-app/frontend/src/panels/TodayPanel.tsx`（占位→骨架）

**Interfaces:**
- Consumes: Task 10 命令面（经 tauriApi）、Task 3 token、Task 4 uiStore。

**登录页规格：** 居中卡片（bg-surface 圆角 10px 1px 边线）；顶部「锡院助手」+ 锡院紫 logo 块；表单：学号 input、密码 input（type=password）、主按钮 `bg-brand text-white`；状态机 `idle → logging-in（loading +「正在识别验证码…」）→ success（进 today）/ manual（message=="CAPTCHA_MANUAL"：验证码图 + 答案 input + 重试）/ error（message 红字）`；**已保存账号区**：`list_accounts()` 有数据时展示账号 chip 列表，点击 → `login_saved`（免输密码）；无数据整块隐藏。**密码绝不进 localStorage。**

**今日页骨架规格：** 问候行（按时段 早安/午安/晚上好 + 显示名）+ 钱包三卡占位（域绿角标、数值 `--`）+ 下一节课横幅（无数据整行隐藏——现在即验证隐藏逻辑）+ 快捷动作 5 按钮占位（查电费/卡片充值/网络报修/办事大厅/全部应用）。

- [ ] **Step 1: LoginPanel + App 路由逻辑**（本地 state `loggedIn`，无需路由库）。
- [ ] **Step 2: TodayPanel 骨架**。
- [ ] **Step 3: `npm run build`** 零错误。
- [ ] **Step 4: 端到端验证（主智能体执行）** — `npm run tauri dev`：真实账号登录 → 今日页问候 → 重启应用 `check_session` 会话保持（session.json 回填）→ 登出回登录页 → 已存账号一键重登。live 测试先行：`CAMPUS_HUB_CREDS=… cargo test -p campus-auth -- --ignored`。
- [ ] **Step 5: 日志脱敏检查（M1 验收项）** — 登录全程开 RUST_LOG=debug，人工核对输出无密码明文、无完整用户名（打码）。
- [ ] **Step 6: Commit** `feat(frontend): 登录页 + 今日页骨架`

---

### Task 12: CodeWiki 初始化 + 收尾

- [ ] **Step 1:** `cw init` + `cw index` + `cw meta update`；手写 `.codewiki/_architecture.md`（协议单点+平台外壳、CommandResult 契约、crates 边界、Dock 导航、RecordingJar/DPAPI/会话持久化要点）。
- [ ] **Step 2:** `CHANGELOG.md` 追加本轮条目；PLAN.md 勾选 M0/M1 完成项。
- [ ] **Step 3:** 全量回归：`cargo test --workspace`（含 campus-schedule 12 项）+ `npm run build`。
- [ ] **Step 4:** 主智能体本地合并：`git switch master && git merge --ff-only feat/sess-b27a282c-m0-m1`（**本仓库无远端，不走 git-merge-push.sh**），`git-branch-audit.sh` 复查后删分支。

---

## 附：任务依赖与派单编排（并发红线 ≤2）

```
Task 1 → A 轨(2→3→4→5)        ─┐
       → B 轨(6→7)             ─┼→ 批2: 8b(captcha 实现) ∥ C 轨(9)  → 批3: 10 ∥ 11 → 12
主智能体并行: 8a 采集(批1 期间) → 8a 标注(批2) ─┘
```
- 根 Cargo.toml 写权时序：Task 1 → Task 6（加 campus-auth）→ Task 9（加 src-tauri）；Task 8 全程不改根文件 → 并行无冲突。
- 批1 = A 轨 ∥ B 轨；批1 期间主智能体跑采集脚本（Bash 非分身）；批2 = 标注分身 ∥ (8b ∥ 9 合并为一个执行分身任务)；批3 = 10 ∥ 11。
- 计划已派 deepseek-flash 独立评审（5 P0/10 P1/14 P2 已全部消化进本 v2）。
