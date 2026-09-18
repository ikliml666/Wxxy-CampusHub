---
title: 会议代理端点与静默降级可观测化
type: learning
source_files:
  - crates/campus-portal/src/client.rs
  - crates/campus-portal/src/parse.rs
tags:
  - meeting
  - schedule
  - degradation
  - observability
  - url-encoding
---

# 会议代理端点与静默降级可观测化

M2 遗留项接入校级会议卡（`api/uppexcard/ext/dynamicData/10.1.90.34/ZCHY`，门户信封）时踩的两个坑与一条流程教训。接口事实与最终形态见设计文档附录 H 与计划文档 §1.2。

## 1. DJZ 参数按教学周次过滤，无周次前缀恒为 0 条

会议卡响应 `{meta:{success},data:[{HYMC,ZCR,CBDW,DD,RQ,SJ,ZJ,CXRY,NF,ZC,…}]}`，但 `DJZ`（标题）参数决定取哪一周的数据。2026-09-18 实测四组对照：

| DJZ | 结果 |
|---|---|
| `第二周会议日程安排表` | 6 条 |
| `第一周会议日程安排表` | 3 条 |
| `第九周会议日程安排表` | 0 条 |
| `会议日程安排表`（无周次前缀） | 0 条 |

**标题必须由教学周次构造**：`meeting_query_title(week)` = `第<中文数字>周会议日程安排表`，周次由区间起点与学期开学日推算（`teaching_week_of`，开学当周 = 第 1 周），不能写死、不能用自然周序号（教学周从开学日起算）。中文数字 `week_to_chinese` 覆盖 1–99（0 与越界返回 None 走降级）。**风险形态**：「构造错周次」与「请求没打通」都表现为 0 条——排查时必须区分。

## 2. 中文 query 交给 url 层自动编码即可，不用手写 percent-encode

`DJZ=第二周…` 直接拼进 URL 字符串交给 `reqwest::Client::get`，`Url::parse` 对 query 中非 ASCII 自动 UTF-8 percent-encode，产出与官方浏览器请求**逐字节一致**（`%E7%AC%AC%E4%BA%8C…`）。离线验证方式：`Url::parse(format!(…))` 后打印/断言 query 形态，不需要额外编码依赖。

## 3. 教训：降级路径必须可观测，否则「静默空」与「真 0 条」不可区分

初版实现把会议链路所有失败（学期信息失败 / 周次推算失败 / 请求失败 / 解析失败 / 0 条）一律吞成空 Vec「不影响课表日程」——降级承诺本身正确，但**真机出现「会议 0 条且无错误态」时无法定位是哪一环**（6 个分支全部静默）。事后补救：用真机捕获样本离线证明解析层与 URL 编码层正确（真实响应体解析出 6 条），把真因范围收窄到网络响应/运行时输入；同时给每个失败点加一行 `[meeting-diag]` stderr 打点。真机复验时链路一次跑通（`响应 2269 字节 → 解析成功 6 条 → 区间保留 6 条`），功能修复；早前那次「静默 0 条」的具体真因未留下证据——这是把诊断做在事后的代价。

**沉淀的打点纪律**（client.rs `query_meetings_for_range` / `query_meeting_events`）：

- **只在降级/失败时输出**，成功路径零输出（正常刷新不产生噪音）；
- 每个失败点一行：`[meeting-diag]` + 环节 + 周次 + HTTP 状态/错误类别；
- **敏感纪律**：不打 JWT/cookie/响应体全文——错误消息只含 URL（`DJZ` 为周次会议标题，公开信息；无凭据参数）、HTTP 状态与服务端 message；serde 错误类别信息不含 body 内容；
- 外层与内层不重复打点（请求/解析失败在内层打，外层 `Err(_)` 静默降级）。

## 4. 顺带的网关形态事实

该代理端点对**未登录**请求返回 `302 → Location: http://my.cwxu.edu.cn`（首页）——reqwest 默认跟随重定向后拿到 200 HTML，表现为 `parse_meeting_events` 的 serde 错误「expected value at line 1 column 1」。诊断输出据此可区分「会话被网关甩回」（HTML → 解析失败）与「信封失败」（`meta.success=false`，服务端 message）。同形态也佐证了应用域名的 WebVPN 包装验证困境（未登录一律回落，见设计文档附录 H 第 4 节）。
