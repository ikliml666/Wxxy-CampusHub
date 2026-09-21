---
title: 通知中心（三源轮询 + 托盘常驻 + 前端面板）
type: module
source_files:
  - tauri-app/src-tauri/src/infra/notification.rs
  - tauri-app/src-tauri/src/commands/notification.rs
  - tauri-app/src-tauri/src/app_tray.rs
  - tauri-app/src-tauri/src/lib.rs
  - tauri-app/frontend/src/panels/NotificationsPanel.tsx
  - tauri-app/frontend/src/panels/SettingsPanel.tsx
  - tauri-app/frontend/src/components/AppShell.tsx
  - tauri-app/frontend/src/components/DockNav.tsx
  - tauri-app/frontend/src/stores/uiStore.ts
  - tauri-app/frontend/src/shared/types.ts
tags:
  - notification
  - polling
  - tray
  - frontend
  - m5
---

# 通知中心（三源轮询 + 托盘常驻 + 前端面板）

2026-09-20（M5 批 1-4）。应用内的统一通知面：门户资讯 / 待办 / 电费低余额三类检查
合入单一后台轮询，新通知进**通知中心面板**并按设置发系统通知；应用关窗后**隐藏到
托盘**继续跑轮询。数据层在 `infra/notification.rs`（纯函数 + 两份 JSON 状态文件），
轮询与命令在 `commands/notification.rs`，前端在 `panels/NotificationsPanel.tsx`。
基线与竞态的两个教训单独成文：[[learnings/notification-baseline-race|通知基线与已读竞态教训]]。

## 一、数据层（`infra/notification.rs`，纯函数可全量单测）

**两份状态文件**（损坏回退默认、不删坏文件，`load_state:161` / `load_settings:183`）：

- `notification_state.json` = `NotificationState`（`:94`）：`unread` 未读列表（上限
  `MAX_UNREAD=200`（`:49`），同 id 覆盖原位、新 id 追加尾部，`push_unread:260`）+
  `cursors` 每源已见 id 队列（上限 `CURSOR_CAP=100` 丢最旧）+ `baselined_sources`
  显式基线标志 + `lastElecAlertAt` 电费提醒节流时刻。
- `notification_settings.json` = `NotificationSettings`（`:113`，8 字段全 serde
  default 旧文件自动补默认）：默认 7 栏（`DEFAULT_INFO_COLUMNS:60`，与 campus-portal
  `KNOWN_COLUMNS` 同一份实测数据、两处需同步改）、待办/电费开关开、阈值 10 元、
  间隔 10/10/30 分钟、系统通知不静音。

**核心纯函数**：`diff_seen(cursor, already_baselined, fetched)`（`:231`）算「本轮新
id」；`elec_should_alert`（`:278`）= 余额低于阈值 **且** 24h（`DAY_SECS`）未提醒过
**且** 余额非 `None`（提不到数字绝不误报）；`validate_settings`（`:298`）与
`effective_interval_secs`（`:318`，基础间隔 × 退避倍数）。通知条目构造
`info_notification`/`todo_notification`/`elec_notification`（`:336-367`），通知 id
确定性（`info:{栏目id}:{源id}` / `todo:{源id}` / `elec:{YYYY-MM-DD}`）保证重启后
同条目不重复。

## 二、轮询主循环（`commands/notification.rs`）

`poll_tick`（`:218`，lib.rs setup spawn 单一实例，参照 `auto_sync_tick` 先例）：

- **启动即检查**（不等第一个间隔）；三类源独立执行，单源失败不影响其他源。
- **无会话静默跳过、绝不拉起登录**——通知轮询是被动观察者，不得为它触发 CAS 登录
  或验证码识别链路。
- **失败指数退避**：`settle_backoff`（`:203`）成功归 1、失败 ×2，`BACKOFF_MAX_MULT=24`
  配最小间隔 5 分钟 = 封顶 2h（`:39-41`）。
- **每 tick 热读 settings**——改设置无需重启轮询。
- **登录唤醒**（2026-09-21 补）：`POLL_KICK`（tokio `Notify::const_new()`）+ 休眠改
  `tokio::select!` 可中断；`finish_login` 成功点 `notify_one`（`auth.rs`）——否则隔夜
  死会话的失败退避（×2 起）让用户重新登录后首批通知最多等 20 分钟。唤醒同时把退避
  归一（新会话不背旧会话的退避）。
- 每源拉取后按 `diff_seen` 结果落盘 + `push_unread` + 发系统通知；电费源成功提醒后
  记 `lastElecAlertAt`。

**日志纪律**（2026-09-21 补）：lib.rs setup 里初始化 `env_logger`（默认 info，
RUST_LOG 可覆盖，dev 进 stderr）——此前全仓无 logger，`log::warn!` 全部静默丢弃，
轮询/补采失败原因完全不可见（M2 `[meeting-diag]` 教训重演）。

**系统通知红线**（`NotificationExt`，`:106`）：只在 Rust 侧发送（插件不开放前端
invoke），标题恒「锡院助手」（`NOTIFY_TITLE:43`），内容只含 kind 标签 + 通知标题，
**绝不携带 token/票据/详情正文**（锁屏预览可见面最小化）。

**命令面 4 条**（`:321-359`）：`get_notifications`（含 `count_by_kind` 分kind 未读
计数）/ `mark_notifications_read`（`remove_read:303` 从未读列表移除）/ 
`get_notification_settings` / `save_notification_settings`（落盘前 `validate_settings`）。

## 三、托盘常驻（`app_tray.rs`）

- tray-icon / image-ico features；菜单「显示主窗口 / 退出」，左键唤起主窗口（`:45` `build_tray`）。
- **关窗 = 隐藏**：lib.rs 的 `on_window_event` 拦截 main 窗口 `CloseRequested` →
  `hide()` + `prevent_close`，应用常驻跑轮询；真退出只走托盘「退出」。
- **`TRAY_READY: AtomicBool`**（`:28`）：托盘建成与否显式记录，**未建成不拦关窗**
  （走默认关闭退出）——否则窗口藏起来没有托盘可唤回，应用假死（P1 教训，
  见 [[learnings/notification-baseline-race|通知基线与已读竞态教训]]）。

## 四、前端

- **`PanelId` 第 9 项 `notifications`**（`shared/types.ts` + `uiStore.ts:100` persist
  v3→v4 迁移：旧 8 值仍全部合法原样保留）。
- **NotificationsPanel**：四态（loading/ready/empty/error，`:13-18`）+ kind 徽标
  （`KIND_META:20`——电费用 alert 红，未知 kind 不跳转兜底）+ 点击卡片跳对应面板
  （`jump:130`）。已读操作即时本地移除。
- **Bell 徽标**（AppShell）：**不轮询**——后台 poll_tick 已持续检查，前端只在挂载
  时与每次切面板时读一次 `get_notifications`（本地文件读，成本可忽略；
  `AppShell.tsx:49-52`）。>99 显示 99+。
- **SettingsPanel 通知分区**（`:99`）：三源开关（公告开关落盘语义 = `infoColumns`
  空/非空）+ 电费阈值输入（clamp 校验后落盘）+ 轮询间隔只读展示。

## 五、验证状态

CDP 真机点验过：Dock 9 项、面板空态、设置分区渲染、阈值保存落盘、poll_tick 启动即
对 7 栏建真实基线。**未验证**：WinRT 系统通知弹窗（需打包版）、托盘图标肉眼确认。

相关：[[modules/frontend-shell|前端外壳]]、[[modules/campus-hub-tauri|接线层 campus-hub-tauri]]、[[modules/campus-portal|门户业务协议核心]]（资讯/待办源）、[[modules/campus-synjones|慧新E校协议核心]]（电费源）。
