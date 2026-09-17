// 验证码样本采集（8a · 一次性工具，供模板训练与评测）
// 用法: node scripts/captcha-collect.mjs [数量，默认100]
// 仅 GET /kaptcha，绝不请求 login（避免 CAS 连续错误计数）。
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const CAS = 'https://wxcas.cwxu.edu.cn/lyuapServer';
const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36';
const OUT = path.join(path.dirname(fileURLToPath(import.meta.url)), 'captcha-samples');
const N = parseInt(process.argv[2] || '100', 10);

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const index = [];
  for (let i = 1; i <= N; i++) {
    const name = String(i).padStart(3, '0');
    try {
      const res = await fetch(`${CAS}/kaptcha`, { headers: { 'User-Agent': UA } });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const j = await res.json();
      const b64 = String(j.content || '').split(',')[1];
      if (!b64 || !j.uid) throw new Error('响应缺 content/uid');
      fs.writeFileSync(path.join(OUT, `${name}.png`), Buffer.from(b64, 'base64'));
      index.push({ file: `${name}.png`, uid: j.uid });
      process.stdout.write(`[${i}/${N}] ok\r\n`);
    } catch (e) {
      process.stdout.write(`[${i}/${N}] FAIL ${e.message}\r\n`);
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  fs.writeFileSync(path.join(OUT, 'samples.json'), JSON.stringify(index, null, 2));
  console.log(`完成：成功 ${index.length}/${N}，索引 samples.json`);
})();
