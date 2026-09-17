# M0+M1 基础设施实施计划（脚手架 + CAS 登录闭环）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
> 执行分身按任务逐个领取：只读自己任务 + Global Constraints + Spec；接口以各任务 Interfaces 块为准。

**Goal:** 搭起 Tauri 2 + React 19 桌面脚手架（含域色 Dock 导航），并完成 CAS 登录端到端闭环（含算术验证码自动识别、DPAPI 多账号存储）。

**Architecture:** 根 Cargo workspace 汇聚 `crates/*`（协议核心，无 Tauri 依赖，安卓可复用）与 `tauri-app/src-tauri`（接线层）；前端 `tauri-app/frontend`（React 19 + Tailwind v4 + shadcn/ui + zustand），IPC 唯一出口 `tauriApi.ts`。CAS 协议实现在 `crates/campus-auth`，对拍 `docs/cas-recon/` 已验证的 JS 探针。

**Tech Stack:** Tauri 2 / Rust 2021 / React 19 / TypeScript 5 strict / Vite / Tailwind CSS v4（@tailwindcss/vite）/ shadcn-ui / zustand / framer-motion ^12 / gsap ^3.15 / lucide-react / reqwest 0.12（rustls + cookies）/ num-bigint-dig / image（PNG 解码）

**Spec:**
- `PLAN.md`（里程碑与已确认决策）
- `docs/design/frontend-design.md`（§4 Dock 规格 / §6 token / §7 选型）
- `docs/cas-recon/REPORT.md`（CAS 协议全定案：端点、RSA 参数、错误码、请求样例）
- `docs/cas-recon/rsa30.js` `cas.js` `probe.js`（已验证的线上 RSA 模块与端到端探针，Rust 实现与之对拍）
- 参考项目 `../Wxxy-CampusLogin/tauri-app/`（Tauri 2 配置、CommandResult 契约 `src-tauri/src/infra/state/mod.rs`、DPAPI `src-tauri/src/account/`）

## Global Constraints（每个任务隐含遵守）

- Windows 先行；包管理器一律 **npm**（对齐参考项目，不引入 pnpm/yarn）。
- TypeScript `strict: true`；React 19；不引入 Ant Design / Arco / Semi。
- IPC 唯一出口 `frontend/src/shared/tauriApi.ts`；所有命令返回 `CommandResult { success: boolean, message?: string, data?: T }`。
- `crates/*` 协议核心**不得依赖** tauri/tokio 之外的桌面运行时；`src-tauri` 只做接线与状态管理。
- 敏感纪律：密码/凭据不明文进日志、错误消息、commit；日志输出一律脱敏（用户名打码、密码永不出现）。
- Apache-2.0 合规三件套已就位（campus-schedule），campus-auth 为本项目原创，无需上游声明。
- 分支 `feat/sess-b27a282c-m0-m1`，小步 commit（`feat:`/`fix:`/`chore:`/`docs:` 前缀）；不 push（本仓库无远端）。
- 验收命令以实际输出为准；首次 cargo 编译 tauri 较慢属正常。
- 凭据文件 `账号与密码.txt` 已在 .gitignore；集成测试从 `CAMPUS_HUB_CREDS` 环境变量指定的文件读取，禁止硬编码。
- **会话保活边界**：M1 只做「启动时 `check_session` 浅检测（customsid cookie 存在）+ 请求失败降级回登录页」；定时保活轮询属 M5，本计划不实现。

---

### Task 1: 根 Cargo workspace

**Files:**
- Create: `Cargo.toml`（仓库根）
- Delete: `crates/campus-schedule/Cargo.lock`（workspace 统一到根 lock）

**Interfaces:**
- Produces: workspace members = `["crates/campus-schedule", "crates/campus-auth", "tauri-app/src-tauri"]`（campus-auth 与 src-tauri 后续任务创建；先用 `exclude` 之外的占位注释？不行——**workspace members 只列现存目录**，Task 2/6 创建对应 crate 时再把条目加进来。本任务 members = `["crates/campus-schedule"]`，后续任务各自追加。）

- [ ] **Step 1: 写根 Cargo.toml**

```toml
[workspace]
resolver = "2"
members = ["crates/campus-schedule"]

[workspace.package]
edition = "2021"
version = "0.1.0"

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
- Create: `tauri-app/frontend/{package.json, vite.config.ts, tsconfig.json, tsconfig.node.json, index.html}`
- Create: `tauri-app/frontend/src/{main.tsx, App.tsx, index.css, vite-env.d.ts}`
- Create: `tauri-app/frontend/public/fonts/`（Outfit 数字子集 woff2）

**Interfaces:**
- Produces: 可 `npm run build` 的空应用；`index.css` 内含 Task 3 的 token（本任务先放最小 `@import "tailwindcss";`，token 由 Task 3 填）。

- [ ] **Step 1: 初始化 npm 项目并安装依赖**

```bash
mkdir -p tauri-app/frontend && cd tauri-app/frontend
npm init -y
npm i react react-dom @tauri-apps/api zustand framer-motion gsap lucide-react clsx tailwind-merge
npm i -D typescript vite @vitejs/plugin-react tailwindcss @tailwindcss/vite @types/react @types/react-dom @tauri-apps/cli
```
package.json scripts: `"dev": "vite"`, `"build": "tsc -b && vite build"`, `"tauri": "tauri"`；`"type": "module"`。

- [ ] **Step 2: 配置 vite / ts**

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

- [ ] **Step 3: 最小入口** — `index.html`（`<div id="root">` + `/src/main.tsx`）、`src/main.tsx`（createRoot 渲染 App）、`src/App.tsx`（渲染 `<div className="min-h-screen bg-bg text-text">锡院助手</div>`）、`src/index.css`（`@import "tailwindcss";`）。

- [ ] **Step 4: Outfit 字体子集落地** — 从 Google Fonts 下载 Outfit latin 子集 woff2（数字/拉丁，~15KB）存 `public/fonts/outfit-latin.woff2`；`index.css` 加 `@font-face { font-family: "Outfit"; src: url("/fonts/outfit-latin.woff2") format("woff2"); font-display: swap; }`。下载失败（网络）则跳过并在此任务 commit message 注明，字体栈回退 Segoe UI。

- [ ] **Step 5: 验证** `npm run build` — Expected: tsc 零错误 + dist 产出。
- [ ] **Step 6: Commit** `feat(frontend): Vite+React19+TS+Tailwind v4 脚手架`

---

### Task 3: 视觉 token 系统（域色 CSS 变量 + Tailwind @theme）

**Files:**
- Modify: `tauri-app/frontend/src/index.css`
- Create: `tauri-app/frontend/src/shared/cn.ts`

**Interfaces:**
- Produces: Tailwind 类可直接用 `bg-bg text-text text-text-2 bg-surface border-line bg-brand text-wallet text-info text-todo text-sched text-alert`；`cn()` 合并类名工具（clsx + tailwind-merge，全项目唯一 cn）。

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

:root { color-scheme: light; }
:root.dark { color-scheme: dark; }
.dark {
  --color-bg: #17151c;
  --color-surface: #201d28;
  --color-text: #efedf4;
  --color-text-2: #a29aad;
  --color-line: #35303f;
  --color-brand: #8b5cf6;
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

- [ ] **Step 3: 验证** `npm run build` + App.tsx 临时用 `bg-brand text-white` 目测类生效（build 过即视为 token 编译通过）。
- [ ] **Step 4: Commit** `feat(frontend): 域色 token 系统（浅/深主题 + Outfit 数字字体）`

---

### Task 4: IPC 契约 + zustand + AppShell + Dock 导航骨架

**Files:**
- Create: `frontend/src/shared/tauriApi.ts`、`frontend/src/shared/types.ts`
- Create: `frontend/src/stores/uiStore.ts`
- Create: `frontend/src/components/AppShell.tsx`、`frontend/src/components/DockNav.tsx`
- Create: `frontend/src/panels/{TodayPanel,InfoPanel,TodoPanel,SchedulePanel,AppsPanel,WalletPanel,PowerPanel,SettingsPanel}.tsx`（本任务全为占位实现）
- Modify: `frontend/src/App.tsx`

**Interfaces:**
- Produces（后续任务消费，签名冻结）:

```ts
// shared/types.ts
export type CommandResult<T> = { success: boolean; message?: string; data?: T };
export type PanelId = "today" | "info" | "todo" | "schedule" | "apps" | "wallet" | "power" | "settings";

// shared/tauriApi.ts —— IPC 唯一出口
export async function invokeCommand<T>(cmd: string, args?: Record<string, unknown>): Promise<CommandResult<T>>;
// 实现: import { invoke } from "@tauri-apps/api/core";
//   const raw = await invoke<CommandResult<T>>(cmd, args); 直接返回 raw；
//   invoke 抛错（命令 panic/拒绝）时 catch 后包装为 { success:false, message:String(e) }。

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

- [ ] **Step 1: tauriApi + types + uiStore**（按上面签名，含 persist 中间件 `import { persist } from "zustand/middleware"`）。
- [ ] **Step 2: DockNav 骨架** — 容器 `fixed bottom-5 left-1/2 -translate-x-1/2 z-30 flex items-center gap-1 rounded-[18px] border border-white/60 bg-white/80 px-3 py-2 shadow-[0_8px_30px_rgb(0_0_0/0.12),0_1px_0_rgb(255_255_255/0.6)_inset] dark:border-white/10 dark:bg-[#201d28]/85`；不用 backdrop-filter（性能，设计文档 §4 明确）。每项：`<button aria-label={label}>` 内 lucide 图标 size 18；hover 显示 tooltip（用 shadcn Tooltip，delayDuration 250）。激活项渲染激活胶囊（本任务先静态 `bg-black/5 rounded-full`，域色版在 Task 5）。
- [ ] **Step 3: AppShell** — 顶条：左「⌘K 搜索」按钮（占位，点击暂无动作）+ 右侧 🔔/🌓/👤 三个图标按钮（占位）；`🌓` 点击切 `document.documentElement.classList.toggle("dark")`（最小可用深浅切换）。内容区 `<main className="pb-28">` 包 AnimatePresence 切换面板。
- [ ] **Step 4: 8 个占位面板** — 统一组件形态：左上 8px 域色角标 + 面板标题 + 「建设中」空态文案；App.tsx 按 activePanel 渲染对应面板。
- [ ] **Step 5: shadcn/ui 基座** — `npx shadcn@latest init -d`（若交互阻塞则手工建 `components.json`），添加 `npx shadcn@latest add button input card tooltip`；组件落 `frontend/src/components/ui/`。
- [ ] **Step 6: 验证** `npm run build`；`npm run dev` 起 1420 端口浏览器开 `http://localhost:1420` 目测 Dock/顶条/深浅切换（截图存 `docs/verify/m0-dock.png`）。
- [ ] **Step 7: Commit** `feat(frontend): IPC 契约 + AppShell + Dock 导航骨架（8 面板）`

---

### Task 5: Dock 动效完整版（域色胶囊 + 磁吸放大）

**Files:**
- Modify: `frontend/src/components/DockNav.tsx`

**Interfaces:**
- Consumes: Task 4 的 DOCK_ITEMS / uiStore。
- Produces: 最终交互 = 设计文档 §4（激活胶囊+指示条随 spring 滑动、80px 磁吸、reduced-motion 降级）。

- [ ] **Step 1: 激活态** — framer-motion `<motion.span layoutId="dock-pill">` 圆角胶囊 + 底部 3px 圆点指示条，`transition={{ type: "spring", stiffness: 500, damping: 34 }}`；胶囊与图标激活色 = 该项 `color`（域色），未激活图标 `text-text-2`。
- [ ] **Step 2: 磁吸** — gsap `quickTo` per-item 控制 `scale`（1→1.35）与 `y`（0→-14px）：容器 onMouseMove 计算每项中心距离，<80px 按比例插值目标值；RAF 节流；`onMouseLeave` 全部复位。`matchMedia("(prefers-reduced-motion: reduce)")` 命中则不注册磁吸（胶囊 spring 保留）。
- [ ] **Step 3: 面板切换过渡** — AppShell 中 AnimatePresence `mode="wait"`：进 `y: 8→0, opacity 0→1`（spring），退 0.04s 快出；快速连切用 `useDeferredValue(activePanel)` 只渲染最终面板。
- [ ] **Step 4: 验证** dev 起应用目测：胶囊滑动、磁吸、连切不闪烁；reduced-motion 模拟（系统设置或 emulate）无磁吸。截图 `docs/verify/m0-dock-motion.png`。
- [ ] **Step 5: Commit** `feat(frontend): Dock 域色胶囊 + gsap 磁吸 + 面板过渡`

---

### Task 6: crates/campus-auth — RSA 加密模块

**Files:**
- Create: `crates/campus-auth/{Cargo.toml, src/lib.rs, src/rsa.rs, tests/rsa_golden.rs}`

**Interfaces:**
- Produces:
```rust
// crates/campus-auth/src/rsa.rs
pub const CAS_RSA_N_HEX: &str; // 见 REPORT.md modulus（258 hex 含前导 00）
pub fn rsa_encrypt_hex(plaintext: &str) -> String;
pub fn cas_token_header(now_ms: u64) -> String; // = rsa_encrypt_hex(&format!("lyasp{}", now_ms))
```
- Cargo 依赖：`num-bigint-dig = "0.8"`、`num-traits`、`thiserror`（lib.rs 统一 `CampusAuthError`）。

**算法事实（来自 REPORT.md，已实测）：** modulus 1024 位（258 hex 首字节 00）、e=0x010001、chunkSize=126 字节、明文按 little-endian 组块（块内字节序反转后即大端整数）、块尾补 0 至 126 字节、输出 hex **不补前导零**、块间直接拼接。密码 < 126 字节（现实必然），单块。

- [ ] **Step 1: 生成 JS 参考值（golden）** — 在 `docs/cas-recon/` 下运行：
```bash
cd ../../docs/cas-recon && node -e "
const {RSAKey,setMaxDigits}=require('./rsa30.js');
setMaxDigits(131);
const k=new RSAKey();
k.setPublic('b5eeb166e069920e80bebd1fea4829d3d1f3216f2aabe79b6c47a3c18dcee5fd22c2e7ac519cab59198ece036dcf289ea8201e2a0b9ded307f8fb704136eaeb670286f5ad44e691005ba9ea5af04ada5367cd724b5a26fdb5120cc95b6431604bd219c6b7d83a6f8f24b43918ea988a76f93c333aa5a20991493d4eb1117e7b1','10001');
console.log(k.encryptedString('lyasp1726500000000'));"
```
把输出 hex 固化为测试常量（若 rsa30.js 导出方式不同，读该文件头部导出说明适配；rsa30.js 是线上原样提取）。同时生成第二参考：`rsa_encrypt_hex("Test@Password123")`。
- [ ] **Step 2: 写失败测试** `tests/rsa_golden.rs` — 断言 Rust 输出 == 两个 JS 参考 hex（明文长度 16 与 18，覆盖组块内偏移差异）。
- [ ] **Step 3: 实现 rsa.rs** — 按「算法事实」：`chunks(126)` → 块尾补零 → `reverse()` → `BigUint::from_bytes_be` → `modpow(&e, &n)` → `format!("{:x}", c)`（num-bigint-dig 的 LowerHex 天然无前导零）拼接。
- [ ] **Step 4: `cargo test -p campus-auth`** Expected: 2 golden 测试通过。
- [ ] **Step 5: Commit** `feat(campus-auth): CAS textbook RSA（golden 对拍线上 JS）`

---

### Task 7: crates/campus-auth — CAS 客户端

**Files:**
- Create: `crates/campus-auth/src/{cas.rs, error.rs}`
- Modify: `crates/campus-auth/src/lib.rs`

**Interfaces:**
- Produces（Task 9/10 消费）:

```rust
pub const CAS_BASE: &str = "https://wxcas.cwxu.edu.cn/lyuapServer";
pub const PORTAL_SERVICE: &str = "https://my.cwxu.edu.cn/shiro-cas";
pub const WEBVPN_SERVICE: &str = "https://webvpn.cwxu.edu.cn/login?cas_login=true";
pub const JWGL_SERVICE: &str = "https://jwgl.cwxu.edu.cn/sso/lyiotlogin";

pub struct CasClient { http: reqwest::Client } // cookie store 开启；UA 用真实 Chrome UA
pub struct CaptchaInfo { pub uid: String, pub png_base64: String, pub kaptcha_type: String }
pub struct CasLoginOk { pub tgt: String, pub ticket: String }
pub enum CasLoginError { WrongUserOrPwd, WrongCaptcha, UserLocked, NeedTwoVerify(String), Unknown(String), Network(String) }

impl CasClient {
  pub fn new() -> Result<Self, CampusAuthError>;
  pub async fn kaptcha(&self) -> Result<CaptchaInfo, CampusAuthError>;      // GET /kaptcha，JSON: {kaptchaType,uid,content}
  pub async fn login(&self, username: &str, password_rsa_hex: &str, captcha_uid: &str, captcha_code: &str) -> Result<CasLoginOk, CasLoginError>;
  // POST {CAS_BASE}/v1/tickets, content-type: x-www-form-urlencoded
  // headers: token = cas_token_header(当前毫秒)（用 std::time 内部取，不留参数给调用方）
  // body: username / password(密文) / service=PORTAL_SERVICE / loginType / id=captcha_uid / code=captcha_code / otpcode
  // 成功: 顶层 {tgt, ticket}；失败: 错误码映射（NOUSER→WrongUserOrPwd, CODEFALSE→WrongCaptcha, USERLOCK→UserLocked, TWOVERIFY→NeedTwoVerify, 其余→Unknown(原样码)）——全集以 REPORT.md 为准
  pub async fn sso_follow(&self, service_url: &str, ticket: &str) -> Result<reqwest::Url, CampusAuthError>;
  // GET {service}?ticket={ticket}，跟随重定向到最终落点（302 链）；cookie 已在 client store 中 → 会话建立
  pub async fn portal_cookie_summary(&self) -> Vec<(String, String)>; // 调试/保活检测用：返回 cookie 名与值长度（不返回值本身，脱敏）
}
```

- [ ] **Step 1: 写失败测试**（离线单测，`tests/cas_parse.rs`）：① 错误码 JSON → 枚举映射全覆盖（样例 JSON 内联自 REPORT.md）；② 成功响应 `{tgt,ticket}` 解析；③ 请求体含 6 字段且顺序/编码正确（用 reqwest 的 body 构造函数抽成纯函数 `build_login_body(...)` 单测）。
- [ ] **Step 2: 实现 cas.rs**；网络方法不做单测（见 Step 3 集成测试）。
- [ ] **Step 3: 集成测试 `tests/cas_live.rs`（`#[ignore]` 标注）** — 读 `CAMPUS_HUB_CREDS` 指向的凭据文件（首行 `账号:xxx`、次行 `密码:xxx` 格式，解析函数写在测试内）：kaptcha → 调 `crate::captcha::solve`（Task 8 完成前先手动占位答案，Task 8 后打开自动识别）→ login → sso_follow(PORTAL_SERVICE) → 断言最终 URL 为门户域名且 cookie 含 `customsid`。此测试只在主智能体验收时 `cargo test -p campus-auth -- --ignored` 运行。
- [ ] **Step 4: `cargo test -p campus-auth`** Expected: 离线测试全过。
- [ ] **Step 5: Commit** `feat(campus-auth): CAS 客户端（kaptcha/tickets/SSO 回跳/错误码映射）`

---

### Task 8: 算术验证码识别（模板匹配）

**Files:**
- Create: `crates/campus-auth/src/captcha.rs`、`crates/campus-auth/templates/kaptcha-templates.json`
- Create: `scripts/captcha-collect.mjs`（一次性采集脚本，进 git 供复训）
- Modify: `crates/campus-auth/Cargo.toml`（`image = { version = "0.25", default-features = false, features = ["png"] }`）

**Interfaces:**
- Produces:
```rust
pub struct KaptchaTemplates; // include_str! JSON 加载
impl KaptchaTemplates {
  pub fn load() -> Self;
}
/// 输入 PNG 字节 → 输出算术答案（如 "63"）。返回 None = 无法识别（上层刷新验证码重试）。
pub fn solve(png_bytes: &[u8], t: &KaptchaTemplates) -> Option<String>;
```

**实现规格：**
1. `image::load_from_memory` → `to_luminance8()`；阈值二值化（Otsu，无外部依赖手写 ~15 行：直方图 + 类间方差取峰）。
2. 垂直投影切分：按列统计前景像素，找连续空列切字符；题面为「两一位数 + 运算符(+/-/×) + = ?」→ 只需前 3 个非空片段（两个操作数、一个运算符）；切出的前 3 片若数量对不上（粘连/断裂）→ 返回 None。
3. 每片段缩放至 24×24（最近邻）→ 与模板（JSON: `{"char":"7","grid":[0,1,...]}` 576 项 0/1）逐位欧氏距离，取最小距离且 < 阈值 90（低于则 None）；第二近与第一近平局（<8%）也判 None。
4. 识别串 `d1 op d2` → 求值：`+`/`-`（可为负，返回字符串含负号）/`×`→乘法。
- [ ] **Step 1: 采集脚本 `scripts/captcha-collect.mjs`** — 复用 `docs/cas-recon/cas.js` 的会话逻辑：GET kaptcha N=30 次，PNG 存 `scripts/captcha-samples/NN.png`，同时把 `{uid}` 记进 `samples.json`（**不要**请求 login，避免错误尝试）。node 运行需能访问学校 CAS（校园网/VPN 环境）。
- [ ] **Step 2: 标注** — 主智能体派多模态分身 Read 30 张图，产出 `scripts/captcha-samples/labels.json`（`{"01.png":"8+3", ...}` 算式，不含答案）；主智能体抽查 5 张核对。
- [ ] **Step 3: 模板生成** — 写 `scripts/captcha-build-templates.mjs`（纯 node，无依赖：PNG 解码用 image crate？不——脚本端用 node 内置？**node 无内置 PNG 解码**；改为 Rust 侧写 `#[test] #[ignore] build_templates()`：读 samples 目录 + labels.json → 二值化切分 → 以每字符最清晰的一个样本为模板 → 写 `templates/kaptcha-templates.json`）。主智能体验收时跑一次生成。
- [ ] **Step 4: 失败测试先行** `crates/campus-auth/tests/captcha_solve.rs`（`#[ignore]`，依赖 samples 目录）——30 张样本识别正确率 ≥ 24/30（80%）才算过；阈值在测试里断言。
- [ ] **Step 5: 实现 captcha.rs** 按「实现规格」。
- [ ] **Step 6: 跑测试** `cargo test -p campus-auth -- --ignored captcha_solve`。未达 80% → 调阈值/切分参数，仍不达 → 在 `ponytail:` 注释记录正确率并保留手动兜底路径（登录流程已设计：3 次失败转人工输入），不阻塞主线。
- [ ] **Step 7: Commit** `feat(campus-auth): 算术验证码模板匹配识别（样本+模板+测试）`

---

### Task 9: src-tauri 骨架 + 会话状态 + DPAPI 账号存储

**Files:**
- Create: `tauri-app/src-tauri/{Cargo.toml, build.rs, tauri.conf.json, capabilities/default.json, icons/*}`
- Create: `tauri-app/src-tauri/src/{main.rs, lib.rs, commands/mod.rs, commands/auth.rs, infra/mod.rs, infra/state.rs, account/{mod.rs, crypto.rs, store.rs}}`
- Modify: 根 `Cargo.toml`（members 追加 `tauri-app/src-tauri`）

**Interfaces:**
- Consumes: `campus_auth::{CasClient, rsa}`（Task 6/7）。
- Produces:
  - `infra/state.rs`: `AppState(Mutex<Option<CasSession>>)`，`CasSession { client: CasClient, username: String }`；`commands::mod` 注册表 `lib.rs::run()` 里 `invoke_handler![commands::auth::get_captcha, commands::auth::login, commands::auth::check_session, commands::auth::logout]`（本任务先注册空实现，Task 10 填肉）。
  - `account::crypto`: `dpapi_protect(plain: &str) -> Result<String>` / `dpapi_unprotect(b64: &str) -> Result<String>`（Windows CryptProtectData，base64 存取；**对齐 CampusLogin `src-tauri/src/account/` 现成实现，抄模式**）。
  - `account::store`: 账号库 `%APPDATA%/campushub/accounts.json`：`{ accounts: [{ username, password_b64(密文), last_login, display_name? }] }`；`save_account/load_accounts/remove_account`；文件 0644 常规，密码永不落明文。
  - `tauri.conf.json`: identifier `com.cwxu.campushub`、`frontendDist: "../frontend/dist"`、`devUrl: "http://localhost:1420"`、`beforeDevCommand: "npm run dev"`（cwd tauri-app/frontend，按 Tauri 2 schema 写 `build` 段）；窗口 1280×860 min 1080×720、标题「锡院助手」。icons 从 `../Wxxy-CampusLogin/tauri-app/src-tauri/icons/` 拷贝全套占位（后续换品牌图标）。
- [ ] **Step 1: 参照 CampusLogin 写 src-tauri 全套**（Cargo.toml 依赖对齐：tauri 2、serde、tokio、reqwest rustls+cookies、base64、dirs、`windows = { version = "0.5x", features = ["Win32_Security_Cryptography", ...] }` 或直接抄参考项目 DPAPI 所用 crate）。
- [ ] **Step 2: DPAPI 单测** `src-tauri/src/account/crypto.rs` 内 `#[cfg(test)]`：protect→unprotect roundtrip 相等；密文不含明文子串。
- [ ] **Step 3: store 单测**：写临时目录 JSON → load 往返；密码字段为 base64 且非明文。
- [ ] **Step 4: `cargo test -p campus-hub`**（src-tauri crate 名 `campus-hub`，lib 名 `campus_hub_lib`）+ `cargo check -p campus-hub`。Expected: 过。
- [ ] **Step 5: Commit** `feat(tauri): src-tauri 骨架 + AppState + DPAPI 账号存储`

---

### Task 10: Tauri 命令接线（登录全流程）

**Files:**
- Modify: `tauri-app/src-tauri/src/commands/auth.rs`、`tauri-app/src-tauri/src/infra/state.rs`
- Modify: 根 `Cargo.toml`（members 追加 `crates/campus-auth`）

**Interfaces:**
- Produces（前端 tauriApi 对接面，与 Task 4 签名对齐）:

```ts
// 4 条命令（invoke 名 → 参数 → data 类型）
get_captcha()            → { uid: string; pngBase64: string }
login(account: { username: string; password: string })  // 前端只传明文密码一次（内存中），Rust 内部全托管
  → 成功 data: { username: string; displayName: string }
  → 三次自动识别+重试均失败时: success=false, message="CAPTCHA_MANUAL", data: { uid, pngBase64 }（前端切手动模式带验证码输入框）
login_manual(account: { username; password; captchaUid; captchaCode }) → 同 login
check_session()          → { loggedIn: boolean }
logout()                 → null
```
Rust login 内部流程：`kaptcha → captcha::solve → login；CasLoginError::WrongCaptcha → 重取重试 ≤3 次 → solve 失败 3 次 → 返回 CAPTCHA_MANUAL`。成功后 `sso_follow(PORTAL_SERVICE)` 建门户会话 + `store::save_account`（DPAPI）+ 写 AppState。`loginType` 常量按 REPORT.md（账号密码方式对应值）。日志宏一律脱敏。

- [ ] **Step 1: 实现命令 + state**；错误映射：`CasLoginError::WrongUserOrPwd → message "账号或密码错误"`、`UserLocked → "账号已锁定"`、其余原样码。
- [ ] **Step 2: `cargo check -p campus-hub`** 过。
- [ ] **Step 3: Commit** `feat(tauri): CAS 登录命令接线（自动验证码 + 手动兜底）`

---

### Task 11: 登录页 UI + 今日页骨架 + 端到端验证

**Files:**
- Create: `frontend/src/panels/LoginPanel.tsx`
- Modify: `frontend/src/App.tsx`（无会话时渲染 LoginPanel，`check_session` 决定）、`frontend/src/panels/TodayPanel.tsx`（占位→骨架）

**Interfaces:**
- Consumes: Task 10 的 4 条命令（经 tauriApi）、Task 3 token。

**登录页规格：** 居中卡片（`bg-surface` 圆角 10px 1px 边线）；顶部「锡院助手」+ 锡院紫 logo 块；表单：学号 input、密码 input（type=password）、登录主按钮 `bg-brand text-white`；状态机 `idle → logging-in（按钮 loading + 「正在识别验证码…」）→ success（跳 today）/ manual（CAPTCHA_MANUAL：展示验证码图片 <img src=data:image/png;base64,…> + 答案 input + 重试按钮）/ error（message 红字提示）`；底部「多账号」入口占位（M1 已有 store，v1 UI 列出已存账号一键重登：`load_accounts` 后续命令，本任务只做入口占位不实现列表）。**绝不记住密码到 localStorage。**

**今日页骨架规格：** 问候行（按时段 早安/午安/晚上好 + 学号显示名）+ 钱包三卡占位（卡片带域绿角标、数值 `--`）+ 下一节课横幅占位（无数据整行隐藏——现在即无数据，验证隐藏逻辑）+ 快捷动作 5 按钮占位（查电费/卡片充值/网络报修/办事大厅/全部应用）。

- [ ] **Step 1: LoginPanel + App 路由逻辑**（`check_session` on mount；本地 state `loggedIn` 即可，无需路由库）。
- [ ] **Step 2: TodayPanel 骨架**。
- [ ] **Step 3: 前端验证** `npm run build` 零错误。
- [ ] **Step 4: 端到端验证（主智能体执行）** — `npm run tauri dev` 起 Windows 窗口：真实账号登录 → 今日页问候出现 → 重启应用 `check_session` 保持 → 登出回登录页。验证码自动识别路径用 live 测试先行验证（`cargo test -p campus-auth -- --ignored`，CAMPUS_HUB_CREDS 指向凭据文件）。失败则按错误修。
- [ ] **Step 5: Commit** `feat(frontend): 登录页 + 今日页骨架`

---

### Task 12: CodeWiki 初始化 + 收尾

- [ ] **Step 1:** `cw init` + `cw index` + `cw meta update`；手写 `.codewiki/_architecture.md`（协议单点+平台外壳、CommandResult 契约、crates 边界、Dock 导航）。
- [ ] **Step 2:** `CHANGELOG.md` 追加本轮条目；PLAN.md 勾选 M0/M1 完成项。
- [ ] **Step 3:** 全量回归：`cargo test --workspace`（含 campus-schedule 12 项）+ `npm run build`。
- [ ] **Step 4:** 主智能体合并 `feat/sess-b27a282c-m0-m1` → master（本地 ff），删除分支。

---

## 附：任务依赖与派单编排

```
Task 1 → Track A(2→3→4→5) ─┐
       → Track B(6→7→8)   ─┼→ Task 9 → Task 10 → Task 11 → Task 12
                            ┘（9 依赖 A4 的 IPC 契约冻结 + B6/B7 的 crate；8 可与 9 并行）
```
并发 ≤2：批1 = A(2-5) ∥ B(6-7)；批2 = B(8 模板与识别) ∥ C(9-10)；批3 = 11；12 主智能体收尾。审查：计划派 deepseek-flash 交叉评审后再开工。
