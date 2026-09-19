---
title: 一卡通页（前端 + 命令面，M4.5）
type: module
source_files:
  - tauri-app/frontend/src/panels/EcardsPanel.tsx
  - tauri-app/frontend/src/components/ecard/EcardHome.tsx
  - tauri-app/frontend/src/components/ecard/EcardBalanceView.tsx
  - tauri-app/frontend/src/components/ecard/EcardBillView.tsx
  - tauri-app/frontend/src/components/ecard/EcardStatsView.tsx
  - tauri-app/frontend/src/components/ecard/EcardPowerView.tsx
  - tauri-app/frontend/src/components/ecard/EcardRechargeView.tsx
  - tauri-app/frontend/src/components/ecard/MiniLine.tsx
  - tauri-app/frontend/src/shared/types.ts
  - tauri-app/src-tauri/src/commands/ecard.rs
  - tauri-app/src-tauri/src/commands/synjones.rs
  - tauri-app/src-tauri/src/commands/electricity.rs
  - crates/campus-synjones/src/ecard.rs
  - crates/campus-synjones/src/ecard_stats.rs
  - crates/campus-synjones/src/ecard_ops.rs
  - crates/campus-synjones/src/recharge.rs
  - crates/campus-synjones/tests/ecard_features_probe_live.rs
tags:
  - frontend
  - ecard
  - tauri
  - statistics
  - m4.5
---

# 一卡通页（M4.5：原「钱包」+「电费」合并）

合并动机与取舍见 [[decisions/ecard-panel-merge|一卡通面板合并决策]]；协议事实与「统计无解」旧结论的更正见 [[modules/campus-synjones|慧新E校协议核心]] 与 [[learnings/ecard-stats-params-and-secure-keyboard|一卡通统计参数实测与安全键盘]]。壳层的 PanelId 9→8 与 persist v3 迁移见 [[modules/frontend-shell|前端外壳]]。

## 页面结构：宫格首页 ⇄ 子页

`panels/EcardsPanel.tsx` 是容器：当前子页放 **uiStore 的 `ecardView`**（持久化、非法值兜底 `"home"`）——今日页快捷动作「查电费」「卡片充值」设 `panel:"ecard"` + `ecardView:"power"/"recharge"` 即可**直达子页**（`TodayPanel.tsx:109-110,441`）。`EcardView = "home" | "balance" | "bill" | "stats" | "recharge" | "power"`（`shared/types.ts:943`）。

- **首页** `EcardHome.tsx`：顶部**主余额带**（主余额口径 = 电子账户，由 `config.balanceShowsElectronic` 决定）+ 功能宫格。
- **余额** `EcardBalanceView.tsx`：`get_ecard_overview` 的 `cards[]`（`getCampusCards` 全卡列表）逐卡渲染。
- **账单** `EcardBillView.tsx`：`get_ecard_transactions` 分页流水 + `get_ecard_types` 分类字典筛选。
- **统计** `EcardStatsView.tsx`：`get_ecard_stats_summary`（今日/区间收支合计）+ `get_ecard_stats_series`（日期序列，`MiniLine.tsx` 手绘 SVG 折线）+ `get_ecard_stats_assort`（分类占比）。
- **充值** `EcardRechargeView.tsx`：一卡通充值（片区 401），复用 [[modules/campus-synjones|慧新E校协议核心]] §五 的 `RechargeFlow` 支付链路，建单带 `noContext: true`（见下「命令面」）。
- **电费** `EcardPowerView.tsx`：**原 PowerPanel 电费内容整体搬入**（级联查询/常用房间/结果卡/三张数据卡均未动），页面壳换成子页形态；原文章细节见 [[modules/electricity-panel|电费页]]（其 `source_files` 已同步指向本文件）。

## 命令面清单（`commands/ecard.rs`，2026-09-19 新增 7 条，全部**只读**）

| 命令 | data 形态 | 位置 |
|---|---|---|
| `get_ecard_overview` | `EcardOverview{ cards: CardDetail[], config: EcardClientConfig }` | `ecard.rs:154` |
| `get_ecard_types` | `EcardTurnoverType[]`（`search/turnoverType` 字典，实测 5 项：1 消费/2 充值/3 退款/4 扫码付/5 补贴） | `ecard.rs:205` |
| `get_ecard_stats_summary` | `EcardStatsSummary{ expensesYuan, incomeYuan }` | `ecard.rs:222` |
| `get_ecard_stats_series` | `EcardStatsPoint[]`（后端已转**按 key 升序**数组、零值保留） | `ecard.rs:243` |
| `get_ecard_stats_assort` | `EcardStatsAssortItem[]`（`{amount, typeId, turnoverType}`） | `ecard.rs:268` |
| `get_ecard_transfer_accounts` | `EcardTransferAccount[]`（`queryCardByTransfer`；⚠️ 学校侧 `data` 是**数组**，解析按数组取） | `ecard.rs:348` |
| `get_ecard_secure_keyboard` | `EcardSecurePad{ padId, keys[] }`（**写操作的预备件**，见 learning） | `ecard.rs:376` |

流水查询在 `commands/synjones.rs::get_ecard_transactions`（`:153`）**原地扩参**：`account?/page/size?/type?/typeId?/info?/orderId?`，映射到协议层 `TurnoverFilter`（`ecard.rs:391`；`build_turnover_params` 保证**未传的可选参数不进 query**）。

**写类（挂失/转账/改密等）本批未做**：安全键盘只到「取键盘 + 进程级缓存」（`ecard_ops.rs`），写操作在下一批。

## 门控与配置（`EcardClientConfig`）

`get_ecard_overview` 顺带解析 `GET /berserker-app/frontInfo?type=pc` 的 `getEcardConfig`/`getFrontConfig` 两个 **JSON 字符串**（`parse_client_config`，`commands/ecard.rs:108`）：

- **只取白名单键**（`balanceShowsElectronic/showSno/showLost/freezeRecharge/manageFee/rechargeFeeitemId/scanFeeitemId/passwordRule` + `enabledApps`）——`getFrontConfig` 串里**含学校侧下发的 `privateKey`**，白名单之外的一个键都不读、绝不落盘/透传（模块头注红线，`commands/ecard.rs:16-17`）。
- 宫格入口按 config 门控：`config.showLost` 控制挂失入口、`enabledApps`（`getAllApps` 中 `status==1` 的 `appCode` 白名单）控制银行卡等入口（`EcardsPanel.tsx:44` 注释）。本校实测 `getAllApps` 19 项里**没有 `bind-campus-card`** ⇒ 多卡绑定入口不显示。
- 主余额口径：`getEcardConfig.type != "2"` ⇒ 电子账户（本校 `type=1`）。
- `frontInfo` 实测 `type=pc` 与 `type=app` **响应完全相同**，按官方 PC 页口径取 `pc`（`commands/ecard.rs:170` 注释）。
- `getAllApps` 拉取失败 → 空白名单（宫格按「无入口」降级），**不阻塞余额展示**（`commands/ecard.rs:182`）。

## 单位口径（一卡通侧 vs `/charge` 侧）

一卡通侧（`berserker-*`/`berserker-search`）一切金额字段是**分**，由协议层 `ecard::yuan()`（`ecard.rs:132`）统一换算成元下发前端（`StatsSummary.expensesYuan` 等）；`/charge` 侧（电费账单/订单）是**元**——跨源聚合时两者口径不可混用，完整红线见 [[modules/campus-synjones|慧新E校协议核心]] §三 与 [[learnings/synjones-charge-yuan-vs-fen-and-pending-orders|元/分口径]]。

## 脱敏策略（契约 §2.5）

- 卡号只给 `account_masked`，**不含原号**（`CardDetail`，`ecard.rs:237-244`）：够长（≥8 位）取前 5 + `****` + 后 2；**短卡号**（本校实测 5 位，如 `42940`）取**首位** + `****` + 末 2（`mask_account`，`ecard.rs:172-185`）——旧实现短号一律给 `****`，界面等于什么都没显示，2026-09-19 修订。
- `AccInfo`（脱敏卡视图，`ecard.rs:205`）同样全金额分→元、不透出原号。
- 完整卡号 `account` 只存在于 IPC 数据内供前端查流水分页，**不入日志**（`commands/synjones.rs:23` 模块头注）。
