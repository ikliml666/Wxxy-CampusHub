---
title: 一卡通面板合并决策（钱包 + 电费 → ecard）
type: decision
source_files:
  - tauri-app/frontend/src/panels/EcardsPanel.tsx
  - tauri-app/frontend/src/stores/uiStore.ts
  - tauri-app/frontend/src/components/DockNav.tsx
  - tauri-app/frontend/src/components/AppShell.tsx
  - tauri-app/frontend/src/panels/TodayPanel.tsx
  - tauri-app/frontend/src/shared/types.ts
tags:
  - decision
  - ecard
  - navigation
  - ui
  - m4.5
---

# 一卡通面板合并决策（M4.5，2026-09-19）

## 结论

原「钱包」（WalletPanel）与「电费」（PowerPanel）两个面板合并为一个 **`ecard`（一卡通）面板**：宫格首页 + 子页（`home/balance/bill/stats/recharge/power`，批 14 起增至八个子页 + `paycode`/`profile`，见文末「后续演进」）。`PanelId` 9 项 → 8 项，Dock 9 项 → 8 项。原两个面板文件删除，电费内容整体迁入 `components/ecard/EcardPowerView.tsx`。

## 为什么合并

1. **用户裁决**：2026-09-19 用户拍板合并（三处入口实为一个体系，拆开是历史包袱）。
2. **一卡通官方系统本身就含电费入口**：官方一卡通 App/PC 内的缴费片区含宿舍电费——「电费桃1-李8 = feeitemid 448 / 李9-李11 = 449 / 梅1-梅3 = 450」（与 `/charge` 电费三片区同源），官方视角它们就是一卡通的子功能。
3. **一卡通充值就是 `/charge` 的 401 片区**：`getFrontConfig.recharge = "401"`，充值走的是同一套 `/charge` 建单/支付链路（401 无级联、`getThirdData(401)` 恒 500 ⇒ 建单不带 `third_party`，见 [[modules/campus-synjones|慧新E校协议核心]] §三）。**三者（余额/流水/统计、电费缴费、充值）本是同一引擎**：同一慧新E校 token、同一 `/charge` 体系、同一客户端配置源（`frontInfo`）。按官方系统的信息架构归组比按自研历史归组正确。

## 导航 9 → 8 与 persist v3 迁移的取舍

- **Dock 8 项**：`ecard` 沿用 `Wallet` 图标与 `--color-wallet` 域色（用户心智 continuity：入口还在原来的位置，只是名字与内容升级为「一卡通」），label「一卡通」。9→8 同时缓解了 Dock 总宽压力。
- **persist `version: 1 → 3`（2026-09-20 后至 v4）**（`uiStore.ts:102`）：v1→v2 是 M2.5 加 timetable 时的版本；本次 v3 的 migrate 做**显式映射**——旧持久化的 `activePanel` 为 `"wallet"` 或 `"power"` 时**直接迁到 `"ecard"`**（`uiStore.ts:112`），而不是依赖「非法值兜底回 today」。理由：兜底会把老用户踢回今日页、白丢一次点击，且语义上「我上次看的余额/电费」就是今天的「一卡通」，显式映射保得住用户位置。
- **子页状态也持久化**：`ecardView` 进 persist（非法值兜底 `"home"`，`uiStore.ts:119-120`），使今日页快捷动作「查电费」「卡片充值」能**直达子页**（`TodayPanel.tsx:109-110` 设 `ecardView: "power"/"recharge"`）；三处同步点（`PanelId` 类型 + DockNav `DOCK_ITEMS` + AppShell `PANEL_MAP`）与 M2.5 的注释约定保持一致。
- **`recharge_create` 显式开关**：随 401 无级联片区的接入，`recharge_create` 的 `path` 改 `Option` 并新增 `no_context: Option<bool>`——**不用「忘传 path」隐式表达无上下文**，避免调用方漏传被静默当成无级联建单（红线见 `commands/electricity.rs:26-27` 注释）。

页面结构、命令面与门控细节见 [[modules/ecard-panel|一卡通页]]；壳层同步见 [[modules/frontend-shell|前端外壳]]。

## 后续演进（2026-09-20 核对）

- **批 14 一卡通扩容**：子页 6 → 8（+`paycode` 付款码、+`profile` 个人中心），决策本身不受影响——两页都是一卡通体系的子功能，进 `ecard` 面板符合同一归组逻辑。
- **M5 通知中心**：`PanelId` 追加第 9 项 `notifications`，Dock 回到 9 项；persist `version` 3 → 4（`uiStore.ts:100` 迁移注释：旧 8 值仍全部合法原样保留）。本决策的「9→8」数字已成历史，**合并本身与迁移方法（显式映射、子页持久化）仍是现行语义**；当前面板全集见 [[modules/frontend-shell|前端外壳]]。
