---
title: 门户头像上传协议与裁切器踩坑
type: learning
source_files:
  - crates/campus-auth/src/cas.rs
  - tauri-app/src-tauri/src/commands/profile.rs
  - tauri-app/frontend/src/components/AvatarDialog.tsx
tags:
  - csrf
  - md5
  - avatar
  - upload
  - react-easy-crop
  - react-19
---

# 门户头像上传协议与裁切器踩坑

2026-09-18 实机取证 + 真机验证打通：登录会话 → JWT/ids → csrf → `portraitChange` 上传 → 回读 `headPortrait`，整条链路真实生效（上传后 `getLoginInfo.headPortrait` 由 PNG 28657 字节变为 JPEG 4852 字节，`officialFetchedAt` 刷新）。协议实现见 [[modules/campus-auth|CAS 协议核心]]，接线见 [[modules/campus-hub-tauri|接线层]]。

## 1. portraitChange 鉴权头方案（实测，缺一不可）

端点 `POST {门户}/api/authc/users/portraitChange`，body 为 `{"displayPhoto": "<完整 data URL>"}`（`cas.rs:316-364`）。除会话 Cookie 外，以下请求头**全部必需**（缺一个即失败）：

| 请求头 | 取值 | 来源 |
|---|---|---|
| `Authorization` | 网关 JWT，**不带 Bearer 前缀** | `tryLoginUserInfo` 响应的 `data.tokenId`（`PortalProfile.token_id`，`cas.rs:70`） |
| `loginUserId` / `loginUserName` | 学号（两处同值，门户前端同款） | `data.userId`（`cas.rs:66`） |
| `loginUserOrgId` | 组织 id，学生实测为 `"-1"`，缺失同值兜底 | `data.orgId`（`cas.rs:68`） |
| `csrfTimestamp` | 当前毫秒时间戳 | 本机时钟现取 |
| `csrfToken` | 见下节现算 | `csrf_token()`（`cas.rs:434-437`） |
| `X-Requested-With` | `XMLHttpRequest` | 门户前端同款 |

JWT 与 ids 每次**现取**（每次调用先 `portal_user_profile()` 拿最新 `PortalProfile`，`cas.rs:326-336`）——不缓存、不落盘，随会话失效自然过期。

## 2. csrf 派生：md5 拼串，不是接口

`csrfToken = md5("timestamp=<毫秒>,key=<GATEWAY_KEY>")` 的小写十六进制（`csrf_token`，`cas.rs:434-437`；`to_hex` 手写 3 行 `cas.rs:440-444`，不为摘要编码引 hex crate）。密钥是门户 app bundle 内的常量 `GATEWAY_KEY`（固化于 `cas.rs:26`）——**明文不入 wiki、不入文档**，源码注释可查。金标向量固化在测试里（`cas.rs:682-684`），防止算法串格式漂移（逗号、等号、无空格，顺序 timestamp 在前）。

## 3. 服务端原样存储：体积守卫只能本端做

`portraitChange` 对 `displayPhoto` **原样存储、不做任何压缩或转码**（实测 123×123 PNG 上传后回读同尺寸 JPEG 字节数即上传内容）。因此：

- 200KB 上限（官方客户端 `size/1024<=200` 同款口径）必须在客户端执行：Rust 侧 `validate_official_data_url`（`profile.rs:108-122`，剥 data URL 前缀后裸 base64 ≤200KB），前端 `findSchoolImage` 阶梯压缩保证产出达标。
- 上传什么就回显什么——前端「保存并上传学校」路径把**同一份**高分辨率图同时上传学校与存本机，两端头像一致；官方头像 `officialBase64` 以服务端回读为准（`profile.rs:249-258`）。
- 200KB 内尽量保留分辨率是有意义的：服务端不会替你压，小图上传就是永久小图。

## 4. 前端两条路径的体积阶梯（AvatarDialog）

`react-easy-crop@^6.2.3` 裁切（**必须** `import Cropper, { type Area } from "react-easy-crop"` 并 `import "react-easy-crop/react-easy-crop.css"`，否则裁切器无样式，`AvatarDialog.tsx:1-3`）：

- **本机路径** `renderLocal`（`AvatarDialog.tsx:77-95`）：边长 `min(1024, 裁切边长)` 不放大；有 alpha 出 PNG、否则 JPEG q0.92；PNG 超 2MB 回退白底 JPEG。
- **学校路径** `findSchoolImage`（`AvatarDialog.tsx:97-114`）：尺寸阶梯 `SIZE_LADDER=[1024,896,…,320]` 外层 × 质量阶梯 `QUALITY_LADDER=[0.95…0.6]` 内层，取第一个 ≤200KB 的组合（保留最大尺寸）；源分辨率不足时 clamp 不放大。白底只在 JPEG 路径铺（`drawCrop`，`AvatarDialog.tsx:27`）——防透明区转 JPEG 发黑。
- 裁切变化防抖 250ms 真实重编码出体积预估（`AvatarDialog.tsx:191-215`），保存时用同一函数重算——预估与落盘必然一致（真机实测：界面预估 139 KB ≈ profile.json 落盘 141385 字节）。
- 裁切分辨率以原图像素为准：`croppedAreaPixels` 是**原图**像素坐标，1200×900 源图 zoom=1 时方形裁切上限就是 900×900，不会被子里的显示尺寸吃掉。

## 5. Hook 顺序坑：useCallback 写在提前 return 之后 = 打开弹窗即白屏

`AvatarDialog` 用 `if (!open) return null;` 控制挂载（`AvatarDialog.tsx:219`）。`onCropComplete` 的 `useCallback` 曾写在**这行之后**——`open` 从 false 变 true 时组件内 Hook 数量增加，React 19 的 Hook 顺序校验直接**卸载整个根节点**，表现为打开头像弹窗即整页白屏、无报错栈可追。修法：所有 Hook（含 `useCallback`）一律放在提前 return **之前**（`AvatarDialog.tsx:216-217`），组件内已有注释明示此约束。教训：`if (cond) return null` 的弹层组件里新增 Hook 时，位置必须在 return 之前——React 只按首次渲染的 Hook 序表对账，顺序错位不是警告而是卸载。

## 6. 已验证（真机，2026-09-18）

- 本机路径：`%APPDATA%/campushub/profile.json` 的 `localBase64` = 141385 字节 PNG 900×900，与界面预估一致，界面右上与首页头像同步更新。
- 学校路径：上传后 `getLoginInfo.headPortrait` 由 PNG 123×123 / 28657 字节变为 JPEG 123×123 / 4852 字节，`officialFetchedAt` 刷新。
- `cargo test -p campus-auth` 全绿：14 个 lib 测试（csrf 金标 2 + portrait 响应解析 3 + 既有解析 9），3 个 ignored 为需真机凭据的 live 测试。
