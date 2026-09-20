---
title: 账户转账功能删除决策（官方全站无转账操作界面）
type: decision
source_files:
  - tauri-app/src-tauri/src/commands/ecard.rs
  - crates/campus-synjones/src/ecard_ops.rs
  - tauri-app/frontend/src/components/ecard/EcardHome.tsx
  - tauri-app/frontend/src/panels/EcardsPanel.tsx
  - tauri-app/frontend/src/shared/types.ts
  - tauri-app/frontend/src/stores/uiStore.ts
tags:
  - ecard
  - transfer
  - recon
  - decision
---

# 账户转账功能删除决策（2026-09-20 批 13）

## 背景

批 4 实现了「账户转账」（`EcardTransferView` + `get_ecard_transfer_accounts` +
`ecard_transfer`，卡账户 ⇄ 电子账户互转）。真机提交恒 `code=400 操作失败`（参数与
官方 bundle 逐字一致、四变体穷举亦然），当时判定「该校未开通」但保留了入口与失败
文案。用户在官方系统里也找不到「卡转系统」，要求实证后删除。

## 2026-09-20 官方全站扫描（逐按钮逐界面遍历取证）

官方 h5（`plat/shouyeUser` 首页 + 大厅 + 我的 + 各子页，登录态经 lyCas 桥恢复）：

- **首页宫格**：付款 / 卡包 / 人脸采集 / 电费×3 / 挂失·解挂 / 账单 / 银行卡 /
  卡片充值 / 全部应用——**无转账**；
- **大厅**（应用全集）：卡片管理（银行卡绑定 / 挂失·解挂 / 银行卡 / 卡片充值）+
  其他服务（人脸采集 / 电费×3）——**无转账**；
- **卡包 → 卡片设置 → 校园卡支付设置**：有「卡账户转账」区，但其中「转账标识」
  只是 ActionSheet 弹层选档位（**只允许自助转账 / 自助及自动转账**，即
  `autotransFlag` 圈存档位），配上自动转账金额/限额——是**圈存设置**，不是转账
  操作界面；
- 之前已实证：`cardTransfer` API 四变体全 400（服务端未开通）+ 官方 bundle 零调用。

## 决策

**删除整个转账功能**：官方没有这个操作界面，我们保留一个永远失败的入口只会误导
用户。删除范围：前端 tile/子页/路由值/类型/命令调用，后端 `transfer`、
`EP_CARD_TRANSFER`、`parse_transfer_accounts`、`get_ecard_transfer_accounts`、
`ecard_transfer` 及其单测；`ecard_features_probe_live.rs` 的历史探针取证保留。

**保留** `ecard_set_autotrans`（圈存标识/金额/限额）——官方「校园卡支付设置」有
对应界面，功能真实可用。

## 官方扫描副产品（功能对照，未实现项的取舍）

| 官方功能 | 我们 | 取舍 |
|---|---|---|
| 付款码（条码+二维码+下拉刷新，付款/支付设置依赖它） | 无 | **不做**：桌面端无线下扫码场景；脱机二维码/付款顺序开关随付款码一起不做 |
| 账单搜索框 | 有筛选无搜索 | 低价值，暂不做 |
| 统计「支出排行榜」 | 有月/年趋势+分类，无排行榜 | 小差异，可选做 |
| 卡片信息（开户时间/有效期/当天支付累计） | 部分展示 | 字段已有（`expDate` 等），暂不扩 UI |
| 修改银行卡（换绑） | 绑定/解绑可组合 | 不做快捷换绑 |
| plat 体系设置（个人资料/安全/设备管理/通用/校园卡解绑） | 无 | **不适用**：plat 是官方 APP 的账号会话体系，与桌面应用登录形态无关 |

**⚠️ 表中两条已被批 14 推翻（2026-09-20 更正，决策本身不变）**：①「付款码不做」——用户裁决接入，`get_ecard_paycode` 等命令与 `EcardPaycodeView` 已落地，条码格式经官方 bundle 取证与官方同款 CODE128（[[learnings/paycode-barcode-format-evidence|付款码条码格式取证]]）；②「plat 不适用」——plat API 与我们 PC 落点 token **同源直调**（[[learnings/plat-api-same-token|plat 体系鉴权与 API 清单]]），个人中心（`get_plat_*` 只读 4 条）与设备写操作（`plat_offline_device`/`plat_remove_device`）已接入。转账删除决策本身不受影响：`cardTransfer` 400 与官方全站无转账界面两条证据仍未被推翻。

## 相关

- [[learnings/ecard-write-protocol-json-body|一卡通写操作协议实测]] — cardTransfer 400 取证
- [[modules/ecard-panel|一卡通页]]
- [[decisions/ecard-panel-merge|一卡通面板合并决策]]
