// Offline fixture generation only. Node's independent Google Brotli encoder.
const fs = require('node:fs');
const z = require('node:zlib');
const crypto = require('node:crypto');
const path = require('node:path');
let state = 0x12345678;
const random = Buffer.alloc(65536);
for (let i=0;i<random.length;i++) { state ^= state << 13; state ^= state >>> 17; state ^= state << 5; random[i] = state & 255; }
const inputs = {empty: Buffer.alloc(0), hello: Buffer.from('hello brotli\n'),
 unicode: Buffer.from('Καλημέρα 世界 🌍\n'.repeat(32)), binary: Buffer.from(Array.from({length:256},(_,i)=>i)),
 bomb: Buffer.alloc(1048576, 97), random};
const manifest = {node:process.version, brotli:process.versions.brotli, files:{}};
function save(name, bytes) { fs.writeFileSync(path.join(__dirname,name),bytes); manifest.files[name]=crypto.createHash('sha256').update(bytes).digest('hex'); }
for (const [name, plain] of Object.entries(inputs)) {
 save(name+'.raw',plain);
 for (const quality of [0,5,11]) {
  const encoded = z.brotliCompressSync(plain,{params:{[z.constants.BROTLI_PARAM_QUALITY]:quality,[z.constants.BROTLI_PARAM_LGWIN]:24}});
  save(name+'-'+quality+'.br',encoded);
 }
}
save('large-window.br', z.brotliCompressSync(inputs.hello,{params:{[z.constants.BROTLI_PARAM_LARGE_WINDOW]:1,[z.constants.BROTLI_PARAM_LGWIN]:25}}));
fs.writeFileSync(path.join(__dirname,'pin.json'),JSON.stringify(manifest,null,2)+'\n');
