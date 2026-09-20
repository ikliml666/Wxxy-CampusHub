---
title: 安全键盘伪字符映射协议（键面必须渲染图片）
type: learning
source_files:
  - crates/campus-synjones/src/ecard_ops.rs
  - crates/campus-synjones/src/ecard.rs
  - tauri-app/frontend/src/components/ecard/SecureKeypad.tsx
  - tauri-app/src-tauri/src/commands/ecard.rs
tags:
  - ecard
  - synjones
  - keyboard
  - security
---

# 安全键盘伪字符映射协议（键面必须渲染图片）

2026-09-20 官方逆向 + 应用内端到端实锤。**旧实现把 `keys` 字符当键面文字渲染是根因性
错误**——用户「密码一直错误」的真相不是密码错，是键面显示的根本不是数字。

## 协议真相

`GET /berserker-secure/keyboard?type=Number&order=1` 返回：

- `numberKeyboard`（keys）：**伪字符数组**（实测样本 `PeEpv7_iR\`、`Bi3xtq-:l{`）——
  与用户想输的数字**无关**；
- `numberKeyboardImage`：**每键一张 PNG**（Number 10 张），图片画的才是用户要按的
  数字（乱序排列）；
- 服务端按 uuid 批次维护「数字 ↔ 伪字符」映射：用户点图片数字（位置 i）→ 提交位置
  序列 → 后端按位置取 `keys[i]`（伪字符）拼 `pwd = 1$1$<伪字符>$1$<uuid>` → 服务端
  按批次映射还原出数字校验。

## 端到端实证（应用内，Number 键盘 + positions 链）

密码（证件后六位）校验通过（retcode=0，`ecard_check_pwd` 返回 ok）。由此钉死四条推论：

1. 官方成功抓包里的「6 位混合明文」= 用户点证件后六位产生的伪字符序列（此前误判为
   percent-encoding 干扰）；
2. 同一明文配**新 uuid** 必失败（重放 60005）——明文与批次绑定；
3. **系统键盘明文直输模式在协议上不可行**（脱离批次的明文服务端还原必失败）——UI
   入口已删（`ecard_check_pwd_plain`/`ecard_unlost_plain` 命令与 `plain_pwd`/
   `fresh_keyboard_uuid` 一并移除）；
4. **键面绝不能渲染 keys 字符**——必须渲染 `images`（官方图片九宫格）。91 键 standard
   盘的图片无法在前端分区（前端不知道每张图画什么字符），查询密码场景统一走 Number。

## 旧结论推翻记录

[[learnings/official-ecard-packet-capture-parity|官方抓包对照]] 第七节「该校查询密码
实际值并非证件后六位」**是错的**：当时穷举把证件后六位明文直接拼 pwd 提交，协议上必然
60005，与密码真值无关。「官方 Number 键盘输不出固定数字密码」同样不成立——官方键面
是图片数字九宫格（用户截图即证据），完全输得出。

## 官方「查看卡号」三步交互

checkPwd 只是**显示闸门**：完整银行卡号随 `getCampusCards` 的 `bankacc` 字段早已下发
（官方存 sessionStorage、详情页读取，无需第三跳）。校验通过后 `van-popup`（bottom）
弹出：标题「查看卡号」、中间 `bankNumber` computed（`isShow=false` 只给前 4 位）+
通栏按钮「查看卡号」→ 点击 `showNumber` → 显全号（4 位一组）。应用已同款复刻
（`fetch_bank_number` 回读 + `RevealCardNoSection` 三步弹窗）。

## 相关

- [[learnings/official-ecard-packet-capture-parity|官方抓包对照（第七节已重写）]]
- [[modules/ecard-panel|一卡通页]]
- [[learnings/ecard-write-protocol-json-body|写操作协议]]
