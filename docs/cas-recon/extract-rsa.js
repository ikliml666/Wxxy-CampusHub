// 从 CAS 前端 app.js 提取 webpack 模块 30（textbook RSA，Shapiro RSA.js 系）
const fs = require('fs');
const path = require('path');
const app = process.argv[2] || path.join(__dirname, 'app.js');
const s = fs.readFileSync(app, 'utf8');
const head = s.indexOf('30:function(e,t,a)');
if (head < 0) throw new Error('module 30 not found');
const open = s.indexOf('{', head);
let depth = 0, end = -1;
for (let i = open; i < s.length; i++) {
  const ch = s[i];
  if (ch === '"') { // 跳过字符串字面量（模块内无模板串/转义陷阱可忽略——已验证无嵌套大括号字符串）
    i++;
    while (i < s.length && s[i] !== '"') { if (s[i] === '\\') i++; i++; }
    continue;
  }
  if (ch === '{') depth++;
  else if (ch === '}') { depth--; if (depth === 0) { end = i; break; } }
}
if (end < 0) throw new Error('module 30 end not found');
const body = s.slice(open + 1, end); // 去掉最外层大括号
const out = `// 提取自 https://wxcas.cwxu.edu.cn/assets/js/app.2fb1f8a1ec5d2342de95.js webpack 模块 30
// 用法（对齐前端 encodePass）：rsa.setMaxDigits(131); const key=rsa.RSAKeyPair(e,'',m); rsa.encryptedString(key, text)
module.exports = (function(){
  var t = {};
  function a(){ return function(){ throw new Error('no deps'); }; }
  ${body}
  return t;
})();
`;
fs.writeFileSync(path.join(__dirname, 'rsa30.js'), out);
console.log('written rsa30.js, body length =', body.length);

// 立即自测：RSAKeyPair 导出与基本加密往返
const rsa = require('./rsa30.js');
console.log('exports:', Object.keys(rsa));
rsa.a(131); // setMaxDigits(131)，对齐前端 encodePass 的调用序列
const PUB_E = '010001';
const MOD = '00b5eeb166e069920e80bebd1fea4829d3d1f3216f2aabe79b6c47a3c18dcee5fd22c2e7ac519cab59198ece036dcf289ea8201e2a0b9ded307f8fb704136eaeb670286f5ad44e691005ba9ea5af04ada5367cd724b5a26fdb5120cc95b6431604bd219c6b7d83a6f8f24b43918ea988a76f93c333aa5a20991493d4eb1117e7b1';
const key = rsa.b(PUB_E, '', MOD); // RSAKeyPair
const ct = rsa.c(key, 'lyasp1737123456789'); // encryptedString
console.log('encrypted("lyasp1737123456789") len =', ct.length);
console.log('ct =', ct.slice(0, 80) + '...');
console.log('golden:', ct);
