---
title: 慧新E校 charge 侧金额是元、一卡通侧是分；待支付单「恒 500」旧结论已推翻
type: learning
source_files:
  - crates/campus-synjones/src/turnover.rs
  - crates/campus-synjones/src/ecard.rs
  - crates/campus-synjones/src/charge.rs
  - crates/campus-synjones/tests/m4_history_probe_live.rs
tags:
  - synjones
  - electricity
  - ecard
  - unit
  - recon
  - m4
---

# 慧新E校 charge 侧金额是元、一卡通侧是分；待支付单「恒 500」旧结论已推翻

2026-09-19 M4 探针（`crates/campus-synjones/tests/m4_history_probe_live.rs`，5 轮内网实测）连续踩到两个跨模块的坑，都会让后来者照抄错东西，故单独记一篇。

## 一、单位：`/charge/*` 是**元**，一卡通侧是**分**

同一时刻的对照样本最清楚：一笔 1 元电费缴费在 `/charge/turnover/app_account` 里是 `TRANAMT = 1`，而**同一次扣款**在一卡通流水 `/berserker-search/search/personal/turnover` 里是 `tranamt = 100`。

| 侧 | 字段 | 单位 | 换算 |
|---|---|---|---|
| `/charge/*` | `TRANAMT` / `tranamt` / `accountTotal` / `pieAccountList[].tranamt` | **元** | 直接用，**不要 `/100`** |
| 一卡通（`ecard.rs`） | `tranamt` / `cardBalance` / `elec_accamt` / `db_balance` / `unsettle_amount` | **分** | 经 `ecard::yuan()` |

坑的诱因：官方 PC 页（`charge-pc` bundle）对 `TRANAMT` **确实做了 `/100`**，照抄就得到 0.01 元。官方 App 版才与实测一致。所以「照抄官方前端」在这条上会翻车——**以实测对照为准**。

连带影响：跨源聚合（如首页钱包卡把一卡通余额与电费余额并排显示）时，两者口径不同，**绝不可混用同一个换算函数**。`turnover.rs` 的 `num_of` 刻意不做任何隐式缩放，并在模块头注与单测里把这个断言钉死（`parse_bills_reads_count_and_yuan_amount` 断言 `amount_yuan == Some(1.0)`）。

## 二、`/charge/order/personal_data?status=0`「任何形态恒 500」——**错在头组，不在端点**

旧结论（M3.1 记录，并写进了 [[modules/campus-synjones|模块文档]] 与 CHANGELOG）是「该端点任何形态恒 500，官方 App 同一调用亦然 ⇒ 学校侧没有可用的待支付订单列表接口」，由此导致「进充值前检查遗留订单」这条防线被判定无法实现。

M4 探针逐项加头定位到真实门槛：**App 拦截器的头组必须齐**——`synAccessSource=app` 同时出现在 query **与**同名请求头（即 `client.rs` 的 `with_headers` 行为）。只带来源头、不带 query 一份时，同参数复跑照样 500；补齐后 `code=200` + `orderList`（实测拿到当时那笔遗留的 1 元待支付单）。

教训（比这一条本身更值钱）：

1. **「恒 500」这种结论必须附上当时的完整头组与参数**——不然它会在 wiki 里活成事实，让后续任务把可行的方案判死；本项目已有一次「防线被误判为做不到」的实际代价（遗留订单只能人工处理）。
2. **同一端点在不同头组下行为完全不同**时，优先怀疑「官方前端有我们没复刻的拦截器行为」，而不是先怀疑端点参数。
3. 该端点的 `feeitemid` 顶层字段**恒为 0**（占位），真实片区 id 在 `feeitemlist[0].feeitemid`（缺失时回落 `orderDetailList[0]`）——只看顶层字段会以为「没带片区」。

## 三、学校侧**没有**电费每日余额

同一轮探针逐月/逐日验证：`turnover` 的 `balance_amount` **恒 null**、`mouthAccount` 只回本月合计（`createdate` 参数被忽略）、`threeExpen_account` 恒空。能拿到余额数字的路径只有级联末级那句自由文本（`charge::balance_from_text`），**且那个数字没有历史**⇒ 日序列只能客户端自采，见 [[decisions/electricity-daily-snapshot-and-merge|电费日快照与多端合并决策]]。

## 相关

[[modules/campus-synjones|慧新E校协议核心]]、[[learnings/history-dedupe-not-by-adjacent-sort|历史去重不能靠排序后看相邻]]。
