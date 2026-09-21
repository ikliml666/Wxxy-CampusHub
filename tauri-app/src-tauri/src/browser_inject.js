// 应用内浏览器注入脚本（Rust 侧 include_str! 打进二进制，不进 resources 打包配置；
// initialization_script 机制：每次文档起始执行；IIFE 防污染页面全局）。
// 副 webview 页面无 IPC capability（不能 invoke Tauri 命令），本脚本只做
// 纯页面侧治理，不与宿主通信（宿主通信走 on_navigation / on_page_load 回调）。
(function () {
  "use strict";
  if (window.__campushub_injected) return;
  window.__campushub_injected = true;
  var host = location.hostname;
  var style = document.createElement("style");
  style.id = "campushub-inject";
  // ① 表格防溢出 ⑨ 图片防横向滚动 ③ 焦点环（老页面常砍 focus-visible）
  style.textContent =
    "table{max-width:100%!important}" +
    "img{max-width:100%;height:auto!important}" +
    ":focus-visible{outline:2px solid #5b2e90!important;outline-offset:1px!important}";
  // ② 纯文本型老页居中：按 host 白名单保守启用（门户信息域）
  if (host === "my.cwxu.edu.cn") {
    style.textContent += "body{max-width:1200px;margin:0 auto!important}";
  }
  // ① 慧新E校移动端布局防糊（桌面端按桌面布局渲染，禁止收缩到移动断点）
  if (host === "10.3.100.110") {
    style.textContent += "html{min-width:1080px}";
  }
  (document.head || document.documentElement).appendChild(style);
  // ⑤ target=_blank → 当前 webview 内导航（老站弹窗满天飞的主因）
  document.addEventListener("click", function (e) {
    var a = e.target && e.target.closest ? e.target.closest("a[target='_blank']") : null;
    if (a && a.href && /^https?:/.test(a.href)) {
      e.preventDefault();
      a.removeAttribute("target");
      location.href = a.href;
    }
  }, true);
  var _open = window.open;
  window.open = function (u) { if (u) location.href = String(u); return null; };
  // ④ 弹窗治理：本批仅收集观察（console 打点），不改 alert/confirm 行为——
  //    隐藏名单灰度后再上，避免误伤校方业务弹窗（设计文档 B.4 护栏）
  var _alert = window.alert;
  window.alert = function (msg) { console.info("[campushub:alert]", msg); _alert.call(window, msg); };
  // ⑥ 下载引导本批不做：WebView2 原生下载行为若不可靠，由工具栏
  //    「在外部浏览器打开」兜底（Task 4 前端补，见设计文档 B.4）。
  console.info("[campushub] injected:", host);
})();
