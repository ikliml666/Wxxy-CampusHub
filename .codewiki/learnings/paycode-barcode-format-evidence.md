---
title: 付款码条码格式取证（CODE128 实锤，「改 ITF」猜测作废）
type: learning
source_files:
  - tauri-app/frontend/src/components/ecard/EcardPaycodeView.tsx
  - crates/campus-synjones/src/plat.rs
tags:
  - ecard
  - paycode
  - barcode
  - recon
  - evidence
---

# 付款码条码格式取证：官方显式 `format:"CODE128"`，不改（2026-09-20）

## 结论

`EcardPaycodeView` 的条码编码格式维持 **CODE128**——不是「一期缺省凑合」，而是与
官方完全同款。此前注释里的「POS 不识别时改 ITF / Code39」是**无证据猜测**，正式
作废（`EcardPaycodeView.tsx:15-18` 与 `:103` 注释均已改写为取证结论）。

## 证据链（官方 bundle 四层拆链）

1. **渲染组件**：官方 `/plat/js/chunk-2d0ccb9c.ff1379f8.js` 的 BarcodeComponent 中，
   JsBarcode 调用为**显式** `{ format: "CODE128", margin: 0, displayValue: !1,
   height: 80 }`——不是 JsBarcode 的缺省值推断（缺省 CODE128 与显式声明外观相同，
   但显式声明排除了「官方用了别的格式靠缺省回落」的可能）。
2. **数据源**：条码串来自 `GET /berserker-app/ykt/tsm/batchGetBarCodeGet` 的
   `data.barcode[]`（20 位数字串 ×10），我们与官方请求参数
   `{account, payacc, paytype}` 逐字一致（[[learnings/plat-api-same-token|plat 体系鉴权与 API 清单]]）——
   编码端数据相同，格式差异只可能出在渲染端，而渲染端已实锤。
3. **排版参数为自由度**：`height`（官方 80 / 本页 72）、`width`、margin 是视觉
   参数，不影响编码格式与扫码识别；除此之外本页与官方逐参数对齐。
4. **口径冻结**：本页 CODE128 即官方口径；后续若真遇到 POS 不识别（当前无任何
   这样的事故报告），优先怀疑扫码环境与条码清晰度，而不是先动格式——格式改动
   现在必须有新的官方证据支撑。

## 方法论沉淀

「一期先选个缺省值、出问题再改」与「先取证钉死」的成本差在**注释置信度**：前者
会留下一条永远悬着的「待验证」注释并诱导后续会话做无据改动（本条 ITF 猜测正是
这样产生的）。凡官方 bundle 可拆链的前端行为（格式、参数名、请求头形态），一次
取证换掉全部后续猜测；取证结论要连**证据位置**（chunk 文件名）一起落注释。

## 关联红线（付款码凭据）

`barcode` 数字串是动态支付凭据：不写 console.log、不进 localStorage、不进错误文案
与 aria-label；除条码图与「查看数字」主动展开外不留存（`EcardPaycodeView.tsx`
凭据红线条）。`codebarPayinfo` 的 `bandacc` 是绑定银行卡全号，相关 DTO 透出必须
脱敏（[[learnings/plat-api-same-token|plat 体系鉴权与 API 清单]]）。

相关：[[modules/campus-synjones|慧新E校协议核心]]、[[modules/ecard-panel|一卡通页]]、[[learnings/official-ecard-packet-capture-parity|官方动态抓包对照与对接修复]]。
