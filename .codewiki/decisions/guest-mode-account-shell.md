---
title: 决策记录 - 游客优先外壳、账号系统与头像三态
type: decision
source_files:
  - tauri-app/frontend/src/App.tsx
  - tauri-app/frontend/src/stores/authStore.ts
  - tauri-app/frontend/src/stores/uiStore.ts
  - tauri-app/frontend/src/components/AccountMenu.tsx
  - tauri-app/frontend/src/components/LoginDialog.tsx
  - tauri-app/frontend/src/components/AvatarDialog.tsx
  - tauri-app/frontend/src/components/Avatar.tsx
  - tauri-app/src-tauri/src/commands/profile.rs
tags:
  - guest-mode
  - login
  - avatar
  - decision
---

# 决策记录：游客优先外壳、账号系统与头像三态

状态：已实施（2026-09-18 外壳视觉重设计批次）。关联模块：[[modules/frontend-shell|前端外壳]]、[[modules/campus-hub-tauri|接线层]]、[[modules/campus-auth|CAS 协议核心]]。

## D1：去掉登录门禁，改为游客优先（壳恒渲染）

**问题**：旧版 `App.tsx` 是登录门卫——启动 `check_session` 未登录就渲染全屏登录页（`AppPhase = "checking" | "in" | "out"`）。代价：① 首屏必被 `check_session` 网络往返阻塞；② 未登录用户什么都看不到，产品在游客眼里是「一个登录框」；③ 登录页独占整屏后，登录入口无处安放，也不存在「以游客身份先逛逛」的路径。

**决策**：壳与面板**恒渲染**，`checkSession` 改为非阻塞（`void` 发出、不 await 渲染，`App.tsx:8-12`）；登录态收敛为 `authStore.status` 三态（unknown/guest/authed，`authStore.ts:9`），各组件自行响应两态——面板游客显示「登录后查看×××」空态 + 登录按钮，已登录未接线显示「数据接入中」。unknown 期间照常可交互，落定后自然切换，不需要任何全局 loading。

**备选与放弃理由**：保留门禁只做「先渲染骨架再决定」（仍挡住全部内容，游客模式名存实亡）；启动本地缓存登录态跳过探测（探测本来就是非阻塞的，再缓存只是增加状态同步面）。

## D2：登录收进账号系统——单弹层 + 多入口

**问题**：门禁删除后，登录 UI 往哪放？已保存账号的免密登录、账号切换、删除账号又放哪？

**决策**：登录入口收敛为一个**弹层**（`LoginDialog`，挂在 App 根部、受 `uiStore.loginDialogOpen` 控制）+ **三个入口**（右上角 AccountMenu、TodayPanel 引导条、各面板 EmptyState 的「登录」按钮），全部经 `uiStore.openLoginDialog()` 打开同一实例。账号管理（已保存账号快捷登录/切换/删除、深浅主题、退出登录）全部收进 AccountMenu 菜单，按 `status` 渲染两套内容（`AccountMenu.tsx:346-568`）。原 `panels/LoginPanel.tsx` 整文件删除，四态状态机（idle/logging-in/manual/error）与 CAPTCHA_MANUAL 流程原样迁入 LoginDialog（`LoginDialog.tsx:14,84-107`）。

**取舍——为什么不引 Radix DropdownMenu / @radix-ui dialog 依赖**：项目现有 shadcn 入库组件只有 button/card/input/tooltip 四件，弹层类组件尚未入库；账号菜单需要的能力（点外关闭、Esc、aria 属性、reduced-motion 降级）手写不过百行，且 framer-motion 已在依赖里（菜单进出动画复用同一套 spring）。引一整套 Radix 只为一处下拉/两个弹层，依赖收益不成比例；后续若接入命令面板等更复杂浮层，再统一升级 shadcn dialog/dropdown 不迟。

**取舍——为什么登录弹层不持久化开关状态**：`loginDialogOpen/avatarDialogOpen` 属一次性 UI 状态，`partialize` 排除在 persist 外（`uiStore.ts:39`）；重启后恢复一个开着的登录弹层是反直觉的。

## D3：头像三态（本地 > 官方 > 首字默认）

**问题**：学校门户有官方头像（getLoginInfo.headPortrait），用户也想要自定义头像；两者并存时展示谁？都没有时展示什么？

**决策**：三级回落——**本地 > 官方 > 首字默认**。后端 `profile.rs::current_avatar` 纯函数统一裁决（`profile.rs:73-90`），`AvatarData { imageBase64, source }` 键恒在值可 null；`clear_avatar` 只清本地、官方保留（清完回落官方展示）；`Avatar.tsx` 无 src 时渲染品牌渐变 + 姓名首字、空则「锡」占位（`Avatar.tsx:34-50`）。上传管线在前端完成（中心裁方 → 256×256 → 有透明出 PNG 否则 JPEG q0.9 → 512KB 内预检，`AvatarDialog.tsx:21-46`），后端只做体积校验与落盘，不引图像处理依赖。首次登录且本地无头像时后台静默补一次官方头像（`authStore.ts:148`），失败不打扰登录流程。

**取舍——为什么头像不走 DPAPI**：DPAPI 的威胁模型是「凭据落盘」（密码、会话 cookie），加密换来的安全性对头像无意义——头像泄漏不产生任何身份冒用面；反过来 DPAPI 落盘/读取每张头像都要加解密往返，且让 `profile.json` 与 `session.json`/`accounts.json` 的容错语义耦合（解密失败的跳过逻辑会蔓延到图片数据）。所以头像明文 base64 存 `profile.json`，DPAPI 严格只保护凭据类数据（纪律线：**非凭据不加密、凭据必加密**）。

**取舍——为什么游客不显示账号头像**：本地/官方头像都是**账号资产**，与登录态绑定才有意义；游客（哪怕 localStorage 里还留着上次登录的 username）展示账号头像会造成「已登录」的错觉。落地为硬规则：`refreshAvatar` 在 `status !== "authed"` 时直接清空头像（`authStore.ts:120-124`），logout 同步清空（`authStore.ts:186-195`）。头像与账号列表不进 localStorage（persist 只存 username/displayName，`authStore.ts:220-223`），由命令面实时拉取。

## 派生约定

- **displayName 生命周期**：登录成功后尽力取门户真实姓名（`portal_user_profile`），失败回退已存值、再回退学号且不抹旧值（save_account 是整条 upsert，`auth.rs:312-330`）；UI 展示统一 `displayName ?? username`，无「未登录」歧义时不含学号（AccountMenu 头部学号去重，`AccountMenu.tsx:352-359`）。
- **错误文案即契约**：`sync_official_avatar` 无会话返回固定文案「请先登录」（`profile.rs:23`），前端不再自判登录态，文案原文内联展示。
- **菜单内反馈不弹窗**：同步头像/免密登录/删除账号的结果全部内联在菜单行内（busy 转圈 + 文案），操作失败不关菜单便于重试（`AccountMenu.tsx:244-270,272-284`）。
