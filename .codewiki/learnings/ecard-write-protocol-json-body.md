---
title: 一卡通写操作协议实测（JSON body / 双层判定 / 三套金额单位）与转账结论
type: learning
source_files:
  - crates/campus-synjones/src/client.rs
  - crates/campus-synjones/src/ecard_ops.rs
  - tauri-app/frontend/src/components/ecard/EcardCardOpsView.tsx
tags:
  - synjones
  - ecard
  - write-ops
  - protocol
---

# 一卡通写操作协议实测

2026-09-20 真机验证（校园网恢复后，用户授权限额与 0.01 元转账）得到的一手结论。批 3 的 15 个写端点曾在交付时全部返回 `code=400 业务异常`，根因不在参数名而在**请求体编码**。

## 一、写操作必须用 JSON body（最关键）

官方前端用的是自己的 axios 实例（默认 JSON body）。我们用 form-urlencoded 提交**同样的参数**会得到 **`code=400 业务异常`** —— 服务端只按 JSON 解析，form 形态连业务层都没进，所以错误码没有信息量。

修复：`SynjonesClient::post_json`（值全字符串）与 `post_json_vals`（值可为 JSON number），`synAccessSource` **合并进 JSON body**，同名头照旧携带。切到 JSON 后错误码立刻变成业务级（`code=60006`、`retcode` 等），排障从「猜」变成「读错误码」。

推论：**看到 `code=400 业务异常` 这种无信息量的错误码，先怀疑报文形态（编码/字段类型），再怀疑参数值。** 官方 bundle 里 `axios.create(...)` 的默认 `Content-Type` 是最该先核对的一件事。

## 二、双层成功判定与 acctype 拆分

- `/ykt/tsm/*` 与 `/accountuser/*` 的成败要看**两层**：axios 层 `code==200` **且** 业务层 `data.retcode=="0"`；失败取 `data.errmsg`，回落顶层 `msg`（`require_retcode_ok`）。
- **`acctype` 必须按 `-` 拆分**：官方是 `acctype.split("-")[0]` / `[1]` 分成 `account` 与 `acctype` 两个字段传。整串原样传会报 **`code=60006 电子账户信息不存在`**。形如 `42940-000` 的值在该校是「卡号-子账户」的复合串。

## 三、金额单位有三套口径（同一批写操作内部都不统一）

| 场景 | 单位 | 官方形态 |
|---|---|---|
| 一卡通读类（`ecard.rs` 流水/统计/卡信息） | **分** | `db_balance` / `elec_accamt` 等原值 |
| `/charge/*`（一卡通充值、电费） | **元** | 该体系自带口径 |
| 写操作·限额与圈存（`daycostlimit`/`nonpwdlimit`/`singlelimit`/`autotransAmt`/`autotransLimite`） | **分** | 官方 `100*x` |
| 写操作·卡间转账 `tranamt` | **元，不乘 100** | 官方 `tranamt: this.amountValue.number` 原值直传 |

转账若按分处理会把金额**放大 100 倍**。判断依据只能是逐端点读官方 bundle，不能靠同一个模块名类推。

## 四、学校侧不回显限额 ⇒ 前端必须本地回写

限额写入**成功**后重取卡信息接口，`dayCostLimitYuan` / `nonpwdLimitYuan` / `singleLimitYuan` **仍是旧值**。这与官方前端把新值写进 `sessionStorage` 再本地渲染的行为一致 —— **不是服务端写入失败，是读回缺失**。

后果：界面若在保存成功后 `onChanged()` 重取概览，就会把用户刚填的值刷回旧值，**看起来像没生效**。修复（`EcardCardOpsView` 限额区）：保存成功后把本次提交值记进 `saved` 状态，展示时 `saved` 优先于学校侧值，并标注「（本次提交值）」+ 文案说明「学校系统不回显限额」。`saved` 随组件卸载消失，不污染学校侧视图。

**通用教训：写成功的判定只能靠写接口的返回，不能靠「回读一致」。** 遇到写成功但读不回，先查官方前端是否也靠本地回写（`sessionStorage`/`localStorage`/Vuex 状态）—— 若是，照做即可，不必怀疑自己的写入。

## 五、卡间转账：参数逐字一致仍被服务端拒绝

`POST /berserker-app/ykt/tsm/cardTransfer`，三次真机尝试（0.01 元字符串 / 0.01 元 JSON number / 1 元 JSON number）**全部**返回 `code=400 操作失败`，且**余额与流水零变化**。

已排除的客户端原因：

- 端点路径：与官方 `acctypeTransferNew` 定义逐字一致。
- 请求形态：JSON body（同批限额已用同一通道通过）。
- 字段名与取值来源：`dstCardAccount`/`srcCardAccount` 取账户的 `account`、`src_acctype`/`dst_acctype` 取 `payacc`，与官方 `acctypeTransfer()` 逐字一致。
- 金额下限：1 元也失败。
- 账户字段缺失：两个账户（`CARD`/`ACCOUNT`）的 `account` 与 `payacc` 均有值，`canTransferOut` 均为 `1`。
- **缺少密码**：官方转账页该区间**不含任何** `pwd`/`password`/`checkPwd`（正则确认命中为空）。

结论：疑**该校未开通此功能，或存在 bundle 不可见的前置条件**（如操作时间窗、渠道限制）。代码保留官方形态并在 `ecard_ops::transfer` 注释标注；**不要**为了让请求「看起来能过」而擅自增删字段。

## 六、CDP 真机点验的两个坑（验证手段本身）

1. **`element.click()` 在某些自定义 Button 上不触发 React 处理器**，改用 `el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, view: window }))` 可靠。
2. **消息文案不一定渲染在 `<p>` 里**：用 `document.querySelectorAll('p')` 抓反馈会漏掉，应读**父容器 `textContent`**，并且**不要 `slice` 截断**——前几次「点击没反应」的误判正是 `slice(0,8)` 把消息截在了外面。
3. 写操作点验要在**同一次表达式内**完成「写值 → 校验值落住 → 点击 → 读反馈」，并带守卫：写入没落住就直接中止，不发请求。

## 相关

- [[modules/campus-synjones|慧新E校协议核心]] — 信封、`synAccessSource` 双份携带、token 单活
- [[learnings/ecard-stats-params-and-secure-keyboard|统计参数修正与安全键盘]] — 读类统计与密码键盘
- [[modules/ecard-panel|一卡通页]] — 前端结构与子页
