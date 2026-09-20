// Offline data preparation, not a decoder test. Node22.23.2 / Google Brotli1.1.0.
const fs = require('node:fs');
const z = require('node:zlib');
const path = require('node:path');
const data = fs.readFileSync(path.join(__dirname,'vectors.bin'));
let offset = 0;
function num() { const n=data.readUInt32LE(offset); offset+=4; return n; }
function bytes() { const n=num(); const b=data.subarray(offset,offset+n); offset+=n; return b; }
const count = num();
for (let i=0;i<count;i++) {
 const name=bytes().toString(); bytes(); const encoded=bytes();
 if (name==='hello-fixed-gzip') fs.writeFileSync(path.join(__dirname,'gzip-then-br.bin'),z.brotliCompressSync(encoded));
}
