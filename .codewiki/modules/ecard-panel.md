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
  - tauri-app/frontend/src/components/ecard/SecureKeypad.tsx
  - tauri-app/frontend/src/components/ecard/EcardCardOpsView.tsx
  - tauri-app/frontend/src/components/ecard/EcardTransferView.tsx
  - tauri-app/frontend/src/components/ecard/EcardBankView.tsx
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

`panels/EcardsPanel.tsx` 是容器：当前子页放 **uiStore 的 `ecardView`**（持久化、非法值兜底 `"home"`）——今日页快捷动作「查电费」「卡片充值」设 `panel:"ecard"` + `ecardView:"power"/"recharge"` 即可**直达子页**（`TodayPanel.tsx:109-110,441`）。`EcardView = "home" | "balance" | "bill" | "stats" | "recharge" | "power" | "cardops" | "transfer" | "bank"`（`shared/types.ts:943`，M4.5 批 4 追加后三值；`uiStore.ts:31-33` 的 `ECARD_VIEWS` 同步）。

- **首页** `EcardHome.tsx`：顶部**主余额带**（主余额口径 = 电子账户，由 `config.balanceShowsElectronic` 决定）+ 功能宫格。
- **余额** `EcardBalanceView.tsx`：`get_ecard_overview` 的 `cards[]`（`getCampusCards` 全卡列表）逐卡渲染。
- **账单** `EcardBillView.tsx`：`get_ecard_transactions` 分页流水 + `get_ecard_types` 分类字典筛选。
- **统计** `EcardStatsView.tsx`：`get_ecard_stats_summary`（今日/区间收支合计）+ `get_ecard_stats_series`（日期序列，`MiniLine.tsx` 手绘 SVG 折线）+ `get_ecard_stats_assort`（分类占比）。
- **充值** `EcardRechargeView.tsx`：一卡通充值（片区 401），复用 [[modules/campus-synjones|慧新E校协议核心]] §五 的 `RechargeFlow` 支付链路，建单带 `noContext: true`（见下「命令面」）。
- **电费** `EcardPowerView.tsx`：**原 PowerPanel 电费内容整体搬入**（级联查询/常用房间/结果卡/三张数据卡均未动），页面壳换成子页形态；原文章细节见 [[modules/electricity-panel|电费页]]（其 `source_files` 已同步指向本文件）。
- **卡务操作** `EcardCardOpsView.tsx`（M4.5 批 4 新增）：挂失·解挂（挂失分区按 `config.showLost` 门控）/ 修改密码三段式（旧密 + 新密 + 确认新密，**三把键盘各领各的**）/ 短信找回密码 / 免密与限额设置 / 圈存转账标识。
- **转账** `EcardTransferView.tsx`（批 4 新增）：卡账户 ⇄ 电子账户互转，金额前端校验**不得超转出账户余额**。
- **银行卡** `EcardBankView.tsx`（批 4 新增）：绑定/解绑银行卡（短信验证码流程）、查看绑定卡号（走**查询密码校验**）；宫格入口按 `config.enabledApps` 含 `yinhangka`/`bind-bank-card` 门控。
- **安全键盘** `SecureKeypad.tsx`（批 4 新增，通用组件）：调 `get_ecard_secure_keyboard` 拿 `keys` 渲染，**只记录用户点击的位置下标序列**（明文不进前端），点满 6 位自动回调提交；占位符只显示已输位数，`aria-label` 只写「第 N 键」——**绝不把键位字符回显到无障碍文本**。

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

## 写操作命令清单（`commands/ecard.rs`，2026-09-19 批 4 新增 15 条；⚠️ **全部未 live 验证**——写路径红线：开发/点验阶段绝不真发写请求，只能由用户在真机上显式触发，与 `submit_pay` 同口径，见 [[modules/campus-synjones|慧新E校协议核心]] §五「未验证项」）

| 命令 | 关键参数 | 说明 | 位置 |
|---|---|---|---|
| `ecard_lost` | `account?` | 挂失（**免密**）；前端二次确认后才发 | `ecard.rs:470` |
| `ecard_unlost` | `account?`, `padId?/positions?` | 解挂；本校无 `unlockFlag` ⇒ 服务端默认需密码，`optional_pad` 要求密码参数**要么都给要么都不给**（`ecard.rs:455`） | `ecard.rs:489` |
| `ecard_check_pwd` | `account?`, `padId/positions` | 校验查询密码，返回 `{ok, bankCardNo}`；本校 `bankCardNo` 恒 null（前端如实提示「学校未返回卡号」） | `ecard.rs:514` |
| `ecard_modify_pwd` | `account?`, 三组 `padId/positions` | 改密三段式（`oldpw`/`newpw`/`renewpw` 各自消耗一把键盘） | `ecard.rs:536` |
| `ecard_send_find_pwd_code` | `account?` | 发验证码，回 `data.account` 作后续会话 id | `ecard.rs:566` |
| `ecard_find_pwd` | `account?`, `padId/positions`×2, `vercode`, `id` | 凭短信验证码设新密（免旧密） | `ecard.rs:585` |
| `ecard_set_limits` | `account?`, `acctype`, 日/免密/单笔限额（**元**） | 后端 `yuan_to_fen_str` ×100 转分（`ecard_ops.rs:292`） | `ecard.rs:614` |
| `ecard_set_autotrans` | `account?`, `flag`, 金额（元）, `limite?` | 圈存转账标识（`autotransFlag/Amt/Limite`） | `ecard.rs:646` |
| `ecard_transfer` | `srcAccount`, `dstAccount`, `srcAcctype`, 金额（元） | 卡间转账；**唯一必须由前端回传账户原号的写命令**（账户来自 `get_ecard_transfer_accounts`） | `ecard.rs:668` |
| `ecard_send_bind_bank_code` | `account?` | 绑定银行卡-发验证码 | `ecard.rs:695` |
| `ecard_bind_bank` | `account?`, 银行卡号, `vercode` | 建立银行卡绑定关系 | `ecard.rs:716` |
| `ecard_cancel_bank` | `account?` | 解绑银行卡 | `ecard.rs:741` |
| `ecard_send_bind_user_code` | `account?` | 绑定用户-发验证码 | `ecard.rs:760` |
| `ecard_bind_user` | `account?`, 姓名, 证件号, `vercode` | 绑定用户身份 | `ecard.rs:779` |
| `ecard_unbind_user` | `account?` | 解绑用户 | `ecard.rs:803` |

**账号来源（批 4 关键设计）**：除 `ecard_transfer` 外，这 15 条命令的 `account` 参数都是 `Option<String>`——因为**卡号原号不暴露给前端**（`CardDetail` 只有 `account_masked`，见下「脱敏策略」），缺省时由后端 helper `resolve_account`（`ecard.rs:440`）调 crate 层新增的 `ecard::current_account`（`ecard.rs:392`，内部走 `getCampusCards` 取本人当前卡）解析。这与「电费房间上下文串由后端合成」（`third_party_for_room`）是同一取舍：**凡是前端拿不到/不该拿的数据，由后端在命令边界现解析**。

**密码协议链路（安全键盘消费）**：前端 `SecureKeypad.tsx` 只提交 `padId`（本进程随机 id，真实 uuid 不出后端）+ 用户点击的**位置下标序列** → 后端 `ecard_ops::assemble_pwd`（`ecard_ops.rs:284`）经 `take_pad`（取走即删）取回键盘映射，纯函数 `build_pwd`（`ecard_ops.rs:263`）按下标翻译成字符拼 `pwd = "1$1$" + 明文 + "$1$" + keyboardUuid`（`pwdType:"1"`），**明文只在后端内存中出现、拼完即弃**——与电费充值 `passwordMap` 同构的红线（[[learnings/ecard-stats-params-and-secure-keyboard|一卡通统计参数实测与安全键盘]]、[[modules/campus-synjones|慧新E校协议核心]] §五安全红线）。

**双层成功判定**：写操作不能只看信封 `code==200`。`require_retcode_ok`（`ecard_ops.rs:302`）要求**axios 层 `code==200` 且业务层 `data.retcode=="0"`**，失败取 `data.errmsg`、回落顶层 `msg` 组装中文错误。既有单测 `retcode_double_layer_judgement` 钉住该口径（`ecard_ops.rs:750`）。

**前端一处已知妥协**：改密时「新密码 === 确认新密码」的本地比对依赖**两把键盘的布局指纹**（两次取键盘若乱序布局不同则本地比不出，明文不进前端所致）——布局不一致时交服务端判定，前端不为此还原明文。**多卡绑定不做**：本校 `getAllApps` 无 `bind-campus-card`（见上「门控与配置」）。

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
