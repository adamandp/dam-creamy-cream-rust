#!/usr/bin/env node
// Temporary diagnostic characterization: collect exact Nest/Axum failure
// envelopes before converting them into durable error-contract assertions.
import { createHmac } from 'node:crypto';

const nestBase = process.env.PARITY_NEST_URL ?? 'http://127.0.0.1:3101';
const rustBase = process.env.PARITY_RUST_URL ?? 'http://127.0.0.1:3102';
const secret = process.env.ACCESS_JWT_SECRET;
if (!secret) throw new Error('ACCESS_JWT_SECRET is required');
const enc = (value) => Buffer.from(value).toString('base64url');
const now = Math.floor(Date.now() / 1000);
const payload = enc(JSON.stringify({sub:'22222222-2222-4222-8222-222222222222',role:'11111111-1111-4111-8111-111111111111',iat:now,exp:now+900}));
const unsigned = `${enc('{"alg":"HS256","typ":"JWT"}')}.${payload}`;
const token = `${unsigned}.${createHmac('sha256',secret).update(unsigned).digest('base64url')}`;
const absent = 'ffffffff-ffff-4fff-8fff-ffffffffffff';
const cases = [
  ['health database ping', '/health/db'],
  ['review list legacy decorators', '/reviews?page=1&limit=10'],
  ['review by me legacy decorators', '/reviews/me?page=1&limit=10'],
  ['user voucher by user route order', '/user-vouchers/user'],
  ['role detail absent', `/roles/${absent}`],
  ['role name absent', '/roles/name/notthere'],
  ['user detail absent', `/users/${absent}`],
  ['address detail absent', `/addresses/${absent}`],
  ['category detail absent', `/categories/${absent}`],
  ['product detail absent', `/product/${absent}`],
  ['product information absent', `/product/information/${absent}`],
  ['inventory detail absent', `/inventory/${absent}`],
  ['inventory by product absent', `/inventory/product/${absent}`],
  ['review detail absent', `/reviews/${absent}`],
  ['reviews by product absent', `/reviews/product/${absent}?page=1&limit=10`],
  ['discount detail absent', `/discounts/${absent}`],
  ['product discount detail absent', `/product-discounts/${absent}`],
  ['voucher detail absent', `/vouchers/${absent}`],
  ['user voucher detail absent', `/user-vouchers/${absent}`],
  ['catalog empty search', '/products/catalog?q=nothing-found-here'],
  ['catalog bad sort', '/products/catalog?sort=sideways'],
  ['catalog unknown query', '/products/catalog?unrecognized=1'],
  ['role bad UUID', '/roles/not-a-uuid'],
  ['category bad UUID', '/categories/not-a-uuid'],
  ['inventory bad UUID', '/inventory/not-a-uuid'],
  ['pagination invalid', '/categories?page=0&limit=10'],
  ['pagination invalid text', '/categories?page=abc&limit=10'],
  ['category create missing description', '/categories', 'POST', {name:'Cream Extra',isActive:true}],
  ['product create missing description', '/products', 'POST', {categoryId:'44444444-4444-4444-8444-444444444444',name:'Cream',price:10000,isActive:true}],
  ['inventory empty PATCH', '/inventory/66666666-6666-4666-8666-666666666666', 'PATCH', {}],
];

async function invoke(base, path, method = 'GET', body) {
  const response = await fetch(new URL(path, base), {
    method,
    headers:{Authorization:`Bearer ${token}`,Accept:'application/json',...(body===undefined?{}:{'Content-Type':'application/json'})},
    ...(body===undefined?{}:{body:JSON.stringify(body)}),
    signal:AbortSignal.timeout(15000),
  });
  const payload = await response.text();
  let data = payload;
  try { data = JSON.parse(payload); } catch {}
  return {status:response.status,data};
}

for (const [label,path,method,body] of cases) {
  const [nest,rust] = await Promise.all([invoke(nestBase,path,method,body),invoke(rustBase,path,method,body)]);
  console.log('ERROR_SURVEY '+JSON.stringify({label,nest,rust}));
}
