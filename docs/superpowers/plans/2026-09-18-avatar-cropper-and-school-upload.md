# 计划：头像裁切器（1:1 拖拽缩放）+ 上传回学校

> 2026-09-18（会话 sess-c2ed9b35）· 分支 `feat/sess-c2ed9b35-avatar-cropper-upload`
> 用户拍板：做 ①react-easy-crop 裁切器 + ②上传回学校；并要求「**200KB 限制下尽量保留高分辨率**」「**增加只改本机头像不上传**」「**放宽本机头像上限**」

## 一、门户上传接口（2026-09-18 实机侦察，含金标）

抓法：登录门户 → 装 XHR 钩子 → 用当前头像生成 1:1 测试图注入官方上传弹窗 → 记录请求/响应（中途用备份头像还原，未留痕）。

- **接口**：`POST https://my.cwxu.edu.cn/api/authc/users/portraitChange`
- **Body**：`{"displayPhoto":"data:image/jpeg;base64,<…>"}` —— 整图内联为 data URL，**无独立附件上传步骤**
- **必需请求头**（除 HttpOnly 会话 Cookie 外的全部）：
  | 头 | 值来源 |
  |---|---|
  | `Authorization` | `POST /tryLoginUserInfo` 响应 `data.tokenId`（JWT，**无 Bearer 前缀**，231 字符；本轮已比对确认） |
  | `loginUserId` / `loginUserName` | 同上 `data.userId`（学号） |
  | `loginUserOrgId` | 同上 `data.orgId`（学生为 `-1`） |
  | `csrfTimestamp` | `Date.now()`（毫秒） |
  | `csrfToken` | **`md5("timestamp=<ts>,key=<GATEWAY_KEY>")`**（小写 hex；网关常量取自门户前端 bundle，明文只写在 `crates/campus-auth/src/cas.rs` 的 `GATEWAY_KEY` 常量里，文档一律不回抄） |
  | `Content-Type` | `application/json` |
  | `X-Requested-With` | `XMLHttpRequest` |
  | `Accept` | `application/json, text/plain, */*` |
- **响应**：HTTP 200 + `{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":true}`
- **金标向量（单测用）**：`md5("timestamp=1789705884524,key=<GATEWAY_KEY>") == "f523769fc014de2a561b4a81a0cf4c7d"`（在 `cas.rs::tests::csrf_token_golden_vector` 中以源码常量重算校验）
- **重要结论**：服务端**原样存储**上传图（实测上传 300×300/7KB 后 `getLoginInfo.headPortrait` 即为该图，长度 9443 ≈ 上传长度）→ **在 200KB 内拉高分辨率是有实际意义的**；官方客户端限制为 `size/1024 <= 200`（KB）、格式 png/jpg、建议 1:1
- 上传成功后门户刷新头像的路径是 `getDefaultSetting.userFace`；我们沿用已验证的 `getLoginInfo.headPortrait`

## 二、冻结契约

**IPC（camelCase，一律 `CommandResult`）**
- 新增 `upload_official_avatar(image_data_url: string) -> CommandResult<AvatarData>`
  - `image_data_url` = **完整 data URL**（`data:image/jpeg;base64,…` 或 png）
  - 无会话 → `err("请先登录")`；数据 URL 非法 → err；**裸 base64 长度 > 200KB → `err("学校头像上限 200KB，请调小尺寸或质量")`**；服务端 `meta.success=false` → err(meta.message 原文)
  - 成功后：重新拉取官方头像落盘（`officialBase64`）并返回最新 `AvatarData`
- 变更 `set_avatar`：本机上限 **512KB → 2MB**（base64 长度），错误文案改「本机头像过大（上限 2MB）」
- `AvatarData` 形状不变：`{ imageBase64: string|null, source: "local"|"official"|null }`

**前端（`AvatarDialog` 重做）**
- 裁切：`react-easy-crop`（aspect 固定 1:1；滚轮 + 滑杆缩放 1–3×；拖拽取景；圆形/方形预览切换；「重置」）；舞台 280×280
- 两条保存路径，各自显示尺寸与体积：
  - **仅保存到本机**：最长边 = `min(1024, 裁切边长)`（不放大），有透明通道出 PNG、否则 JPEG q0.92 → `set_avatar`（裸 base64）
  - **保存并上传学校**：在 **≤200KB** 约束下取「分辨率最高的组合」——尺寸阶梯 `[1024, 896, 768, 640, 512, 448, 384, 320]` × 质量阶梯 `[0.95, 0.9, 0.85, 0.8, 0.75, 0.7, 0.65, 0.6]`，取能塞进 200KB 的最大尺寸 → 完整 data URL → `upload_official_avatar`；**未登录时按钮禁用**并提示「登录后可上传到学校」
  - 上传成功同时把该图（高分辨率版本）存本机，界面显示「已上传学校 · 768×768 · 187KB」
- 保留：拖拽/点击选图、`同步学校头像`、`移除本机头像`、错误内联（不 alert）、512KB 旧守卫改为新上限

## 三、任务分解（2 分身并行，文件不重叠）

### C1 · 协议与后端
- `crates/campus-auth/Cargo.toml`：加 `md-5`（RustCrypto，极小）
- `crates/campus-auth/src/cas.rs`：`PortalProfile` 增 `user_id` / `org_id` / `token_id`（解析 `tryLoginUserInfo` 的 `data.userId/orgId/tokenId`）；新增纯函数 `csrf_token(ts_ms: u128) -> String`（**金标向量单测**）与 `portal_change_portrait(&self, data_url: &str) -> Result<(), CampusAuthError>`（构造 §一 全部请求头，解析 `meta.success`）
- `tauri-app/src-tauri/src/commands/profile.rs`：`set_avatar` 上限改 2MB（含文案与单测）；新增 `upload_official_avatar`（200KB 守卫 + 调 `portal_change_portrait` + 成功后 `portal_login_info` 刷新官方头像落盘）
- `tauri-app/src-tauri/src/lib.rs`：注册新命令
- 验收：`cargo test --workspace` 全绿（新增 csrf 金标、上限、非法 data URL、无会话路径单测）

### C2 · 前端裁切器
- `tauri-app/frontend/package.json`：加 `react-easy-crop`
- `tauri-app/frontend/src/components/AvatarDialog.tsx`：按 §二 重做（裁切 UI + 两条保存路径 + 尺寸搜索 + 状态/错误内联）
- `tauri-app/frontend/src/stores/authStore.ts`：若需新增动作（如 `uploadOfficialAvatar(dataUrl)`）按契约最小改动
- 验收：`npx tsc --noEmit` 0 错误 + `npx vite build` 通过

### C3 · 主智能体真机验证（收尾）
1. `tauri dev` → 裁切器交互（拖拽/缩放/圆方预览）截图
2. 「仅保存到本机」→ 顶栏头像变化 + `profile.json` 体积符合预期
3. 「保存并上传学校」→ 成功后重开门户核对 `getLoginInfo.headPortrait` 已变（尺寸/长度与上传一致）
4. 文档（设计文档附录 C 补上传链路）、CHANGELOG、CodeWiki（learnings 记 csrf/鉴权头）、提交与本地 ff 合并

## 四、风险

| 风险 | 处置 |
|---|---|
| 服务端对 csrfToken/JWT 校验更严（如时限） | 每次请求现算 `csrfTimestamp`+`csrfToken`，JWT 每次登录/取资料时刷新 |
| 上传后服务端做压缩导致与本地不一致 | 上传成功后**重新拉取**官方头像落盘，界面以服务端结果为准 |
| `md-5` 是一把新依赖 | 仅 RustCrypto 纯算法小 crate，无网络/系统依赖；若坚持零依赖可改为自实现 MD5（本轮不采用） |
| react-easy-crop 对 React 19 的 peer 警告 | 只影响安装提示；如遇 peer 冲突按 npm 提示处理并在报告中说明 |
