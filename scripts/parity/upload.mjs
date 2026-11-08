#!/usr/bin/env node
// Real HTTP API -> presigned MinIO PUT -> confirm -> download smoke test.
// Only targets the disposable Rust database and MinIO bucket configured by CI.
import { createHmac } from 'node:crypto';
import { deflateSync } from 'node:zlib';
import { strict as assert } from 'node:assert';

const apiBase = process.env.PARITY_RUST_URL ?? 'http://127.0.0.1:3102';
const productId = '55555555-5555-4555-8555-555555555555';
const userId = '22222222-2222-4222-8222-222222222222';
const roleId = '11111111-1111-4111-8111-111111111111';
const now = Math.floor(Date.now() / 1000);
const unsigned = [
  Buffer.from(JSON.stringify({ alg: 'HS256', typ: 'JWT' })).toString('base64url'),
  Buffer.from(JSON.stringify({ sub: userId, role: roleId, iat: now, exp: now + 900 })).toString('base64url'),
].join('.');
const signature = createHmac('sha256', process.env.ACCESS_JWT_SECRET ?? '')
  .update(unsigned).digest('base64url');
const authorization = `Bearer ${unsigned}.${signature}`;

function chunk(name, content) {
  const tag = Buffer.from(name);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(content.length);
  const data = Buffer.concat([tag, content]);
  let crc = 0xffffffff;
  for (const byte of data) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
  }
  const checksum = Buffer.alloc(4);
  checksum.writeUInt32BE((crc ^ 0xffffffff) >>> 0);
  return Buffer.concat([length, data, checksum]);
}

// Generate an exact 1x1 RGBA PNG using only standard Node.js modules.
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(1, 0);
ihdr.writeUInt32BE(1, 4);
ihdr[8] = 8; // 8-bit color
ihdr[9] = 6; // RGBA
const png = Buffer.concat([
  Buffer.from('89504e470d0a1a0a', 'hex'),
  chunk('IHDR', ihdr),
  chunk('IDAT', deflateSync(Buffer.from([0, 255, 0, 0, 255]))),
  chunk('IEND', Buffer.alloc(0)),
]);

async function api(method, path, body) {
  const response = await fetch(new URL(path, apiBase), {
    method,
    headers: { Authorization: authorization, 'Content-Type': 'application/json' },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    signal: AbortSignal.timeout(15000),
  });
  const data = await response.json();
  assert.ok(response.ok, `${method} ${path} => ${response.status} ${JSON.stringify(data)}`);
  return { status: response.status, data };
}

const presigned = await api('POST', `/products/${productId}/image/presign`, {
  fileName: 'test.png', fileSize: png.length, mimeType: 'image/png',
});
assert.equal(presigned.status, 201);
const { uploadUrl, key, headers } = presigned.data.data;
assert.match(key, new RegExp(`^pending/products/${productId}/`));
assert.equal(headers['Content-Type'], 'image/png');

const put = await fetch(uploadUrl, {
  method: 'PUT', headers, body: png, signal: AbortSignal.timeout(15000),
});
assert.ok(put.ok, `presigned PUT => ${put.status} ${await put.text()}`);
console.log(`PASS presigned direct PUT to isolated MinIO (${put.status})`);

const confirmed = await api('POST', `/products/${productId}/image/confirm`, { key });
assert.equal(confirmed.status, 201);
console.log('PASS product image confirm and immutable object finalization (201)');

const detail = await api('GET', `/product/${productId}`);
const publicImageUrl = detail.data.data.imageUrl;
assert.ok(publicImageUrl.includes(`/media/products/${productId}/`));
const download = await fetch(publicImageUrl, { signal: AbortSignal.timeout(15000) });
assert.ok(download.ok, `image download => ${download.status}`);
assert.deepEqual(Buffer.from(await download.arrayBuffer()), png);
console.log('PASS public media URL -> presigned GET -> identical PNG bytes');

const repeated = await api('POST', `/products/${productId}/image/confirm`, { key });
assert.equal(repeated.status, 201);
console.log('PASS confirm retry is idempotent (201)');
