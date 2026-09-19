---
title: 一卡通统计参数实测与安全键盘机制
type: learning
source_files:
  - crates/campus-synjones/src/ecard_stats.rs
  - crates/campus-synjones/src/ecard_ops.rs
  - crates/campus-synjones/tests/ecard_features_probe_live.rs
  - tauri-app/src-tauri/src/commands/ecard.rs
tags:
  - ecard
  - statistics
  - secure-keyboard
  - live-probe
  - m4.5
---

# 一卡通统计参数实测与安全键盘机制

本篇更正项目早前一条**已被 live 探针推翻的错误结论**，并记录安全键盘（写操作预备件）的协议与方案。协议上下文见 [[modules/campus-synjones|慧新E校协议核心]]，页面侧见 [[modules/ecard-panel|一卡通页]]。

## 一、旧结论为何错：参数形态不对 → 误判「无解」

`.codewiki/modules/campus-synjones.md`（§三 一卡通节）与前端注释里原记着：

> 「今日/本月消费」**无解**：`statistics/turnover/sum/user` 需五参、多种参数组合实测全部返回空 `data`；`statistics/turnover/count` 给的是**全时段**总额且无视日期参数。故 UI 显示「暂不可用」，不要用 count 冒充。

**2026-09-19 live 探针**（`crates/campus-synjones/tests/ecard_features_probe_live.rs`）实测推翻：

- 旧探针的参数**名字/组合不对**：`count` 实际吃 `timeFrom`/`timeTo`（无参 533160 分 / 2026 全年 241540 分 / 2020-01 为 0——三组互不相同，参数显然生效）；`sum/user` 的正确参数是 `dateStr`+`dateType`+`statisticsDateStr`+`type` 四个，且**值域是 `(month|year)` 与 `(day|month)`**，旧尝试大概率在这些枚举值之外。「接口无解」实为「参数没猜对」——教训：对枚举型参数应从官方前端抓包抄值域，不要盲试自由组合。

**实测正确参数表**（全部 `GET`、`/berserker-search/` 前缀 ⇒ 信封一律 `Envelope::Search`、金额一律**分**）：

| 端点 | 参数 | 返回 |
|---|---|---|
| `statistics/turnover/count`（收支汇总） | `timeFrom` / `timeTo`（**实测生效**） | `data.expenses` / `data.income`（分） |
| `statistics/turnover/sum/user`（日期序列） | `dateStr`（`2026-09` / `2026`）+ `dateType`（`month`/`year`）+ `statisticsDateStr`（`day`/`month`）+ `type`（1 收入/2 支出） | `data` 为 `{"日期": 金额分}` map（如 `{"2026-09-03": 1800.0}`；年视图键为 `"2026-03"`） |
| `statistics/turnover`（分类聚合） | `type` + `timeFrom` + `timeTo` | `data[]`：`{amount(分), typeId, turnoverType, nameEn}` |
| `search/turnoverType`（分类字典） | 无 | 5 项：1 消费 / 2 充值 / 3 退款 / 4 扫码付 / 5 补贴 |

解析约定（`ecard_stats.rs`）：`sum/user` 的 map 在后端转**按 key 升序的数组**且**零值保留**（`parse_series`，`ecard_stats.rs:99`）——升序是为了前端免排序、零值保留是因为「当天花了 0」也是序列的合法点，丢零会让折线断段。

## 二、安全键盘（`ecard_ops.rs`）：官方协议与本项目方案

官方 `berserker-secure` 键盘的协议形态：

- 取键盘 `GET /berserker-secure/keyboard?type=Number|Standard`（信封 `Berserker`），返回 **`numberKeyboard` 乱序键位串 + 键盘图片 + `uuid`**。乱序串与 uuid 绑定：服务端知道该 uuid 下每个**显示位置**对应的真实字符。
- 提交密码的官方格式：**`pwd = "1$1$" + 键位下标序列 + "$1$" + uuid`**——提交的是**点击位置的下标序列**而不是字符本身（与 `/charge` 充值密码盘的「位置编码」同一思想，见 [[modules/campus-synjones|慧新E校协议核心]] §五安全红线）。

**本项目选用的方案：前端只传位置下标，后端拼装**。后端取键盘后把 `uuid → 键位映射` 存**进程级缓存**（`ecard_ops.rs`：TTL 300 秒、至多 8 把、随机 `padId`、`take_pad` **取走即删**，`:109-158`），只给前端回**打乱的展示键位**与 `padId`；将来写操作（挂失/改密/转账）时前端只上报用户点的**位置下标**，由后端按下标从映射还原成官方 `pwd` 格式提交。收益与 `/charge` 侧同款：客户端全程**不接触、不落盘、不打日志**真实键盘映射（不落盘不入日志是模块头注红线 1，`ecard_ops.rs:17`），前端也拿不到可还原的密钥材料。

本批只实现「取键盘 + 缓存」；`take_pad` 的消费方（写操作命令）在下一批接入——**取走即删**的语义意味着同把键盘不能重复提交，写命令需按「取键盘 → 一次提交」的单次生命周期设计。

## 三、安全观察：学校侧把 `privateKey` 下发到前端

`GET /berserker-app/frontInfo?type=pc` 的 `getFrontConfig` **JSON 字符串里含学校侧下发的 `privateKey`**（PEM 形态）。这是官方前端要用的私钥被直接塞进了配置串——对自研客户端意味着：该串**绝不能整串透传、落盘或打日志**，解析必须走**键白名单**（`commands/ecard.rs::parse_client_config`，白名单之外一个键都不读）。同串的 `getEcardConfig`/`getFrontConfig` 其余键（`recharge=401`、`scan=407`、`passwordRule` 等）已提炼进 `EcardClientConfig` 白名单。

附带实测事实：`frontInfo?type=pc` 与 `?type=app` **响应完全相同**（本校口径可通用，取 `pc`）；本校 `getEcardConfig = {freezeRecharge:0, showSno:1, type:1, showLost:1, manageFee:1, msCardFlag:0}`（`type=1` ⇒ 主余额口径 = 电子账户）；`getAllApps` 19 项里没有 `bind-campus-card` ⇒ 多卡绑定入口不显示。
