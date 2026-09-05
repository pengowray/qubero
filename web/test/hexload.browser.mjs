// Full application loading profile with a valid 4 MiB PNG. IDAT_SIZE=8192
// exercises hundreds of chunks; unset uses one. Run against TEST_URL (Vite).
import { deflateSync, crc32 } from "node:zlib";
import { randomBytes } from "node:crypto";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ channel: "msedge", headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1100, height: 800 } });
  page.on("pageerror", e => console.error(e));
  await page.goto(process.env.TEST_URL || "http://127.0.0.1:17272");
  await page.evaluate(async () => {
    window.times = {};
    for (const [file, cls, names] of [
      ["hexview", "HexView", ["render", "frame", "measure", "relayout"]],
      ["hexrows", "HexRows", ["write", "heights"]],
      ["doc", "Doc", ["spans", "node", "locate", "runCells"]],
    ]) {
      const proto = (await import(`/src/${file}.ts`))[cls].prototype;
      for (const name of names) {
        const original = proto[name];
        if (!original) continue;
        proto[name] = function (...args) {
          const start = performance.now();
          try { return original.apply(this, args); }
          finally { (window.times[`${cls}.${name}`] ??= []).push(performance.now() - start); }
        };
      }
    }
    localStorage.setItem("qubero.view", "hex");
  });
  const chunk = (kind, bytes) => {
    const result = Buffer.alloc(bytes.length + 12);
    result.writeUInt32BE(bytes.length); result.write(kind, 4); bytes.copy(result, 8);
    result.writeUInt32BE(crc32(result.subarray(4, -4)), result.length - 4);
    return result;
  };
  const head = Buffer.alloc(13); head.writeUInt32BE(1024); head.writeUInt32BE(1024,4); head[8]=8; head[9]=6;
  const pixels = randomBytes(1024 * 4097);
  for (let i=0; i<1024; i++) pixels[i*4097]=0;
  const compressed = deflateSync(pixels);
  const data = [];
  const size = Number(process.env.IDAT_SIZE) || compressed.length;
  for (let i=0; i<compressed.length; i+=size) data.push(chunk("IDAT",compressed.subarray(i,i+size)));
  const png = Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]), chunk("IHDR",head), ...data, chunk("IEND",Buffer.alloc(0))]);
  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("button", {name:"Open a file",exact:true}).click();
  await (await chooser).setFiles({name:"large.png",mimeType:"image/png",buffer:png});
  await page.waitForTimeout(5000);
  if (await page.locator('.hv-hex > span[data-off="0"]').count() === 0) throw new Error("Hex grid did not load");
  console.log(await page.evaluate(() => Object.fromEntries(Object.entries(window.times).map(([k,v])=>[k,{count:v.length,total:v.reduce((a,b)=>a+b,0),max:Math.max(...v)}]))));
} finally { await browser.close(); }
