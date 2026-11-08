#!/usr/bin/env node
// Temporary diagnostic characterization: collect exact Nest/Axum failure
// envelopes before converting them into durable error-contract assertions.
import { createHmac } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';

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
  ['health external ping', '/health'],
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
  ['category invalid name', '/categories', 'POST', {name:'Cr3am',description:null,isActive:true}],
  ['category empty name', '/categories', 'POST', {name:'',description:null,isActive:true}],
  ['category wrong boolean', '/categories', 'POST', {name:'Cream',description:null,isActive:'true'}],
  ['product invalid price', '/products', 'POST', {categoryId:'44444444-4444-4444-8444-444444444444',name:'Cream',description:null,price:0,isActive:true}],
  ['product unknown fields', '/products', 'POST', {categoryId:'44444444-4444-4444-8444-444444444444',name:'Cream',description:null,price:10000,isActive:true,extra:true}],
  ['discount missing nullable', '/discounts', 'POST', {name:'Test',discountType:'FIXED',maxApplied:1,maxRedeemed:10,redeemedCount:0,description:null,productId:null,quantity:null,startDate:'2026-11-08',endDate:'2026-12-08'}],
  ['voucher missing nullable', '/vouchers', 'POST', {code:'TEST',name:'Test Voucher',description:null,type:'FIXED',group:'CHECKOUT',productId:null,quantity:null,minimumPurchase:1000,isActive:true,startDate:'2026-11-08',endDate:'2026-12-08',maxRedeemed:10,redeemedCount:0}],
  ['review invalid rating', '/reviews', 'POST', {userId:'22222222-2222-4222-8222-222222222222',productId:'55555555-5555-4555-8555-555555555555',rating:6}],
  ['address invalid phone', '/addresses', 'POST', {userId:'22222222-2222-4222-8222-222222222222',recipientName:'Parity',recipientPhoneNumber:'abc',province:'Banten',city:'Tangerang',subdistrict:'Serpong',village:'Lengkong',postalCode:'15310',address:'Cream Street',isPrimary:false}],
  ['address unknown field', '/addresses', 'POST', {userId:'22222222-2222-4222-8222-222222222222',recipientName:'Parity',recipientPhoneNumber:'081234567890',province:'Banten',city:'Tangerang',subdistrict:'Serpong',village:'Lengkong',postalCode:'15310',address:'Cream Street',isPrimary:false,unexpected:42}],
  ['inventory empty PATCH', '/inventory/66666666-6666-4666-8666-666666666666', 'PATCH', {}],
];

// Legacy Nest controllers have no UUID pipe on some routes, invalid requests
// bubble up a Prisma 500 whose stack includes CI-specific absolute paths.
// The reviews controller also omits the decorators on two handler parameters.
// Report these historical issues independently rather than mask mismatches in
// the stable 400/404 API contract with status-only assertions.
const legacyExceptions = new Set([
  'review list legacy decorators',
  'review by me legacy decorators',
  'user voucher by user route order',
  'category bad UUID',
  'inventory bad UUID',
]);

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

let passed = 0;
let failed = 0;
let legacy = 0;
for (const [label,path,method,body] of cases) {
  const [nest,rust] = await Promise.all([invoke(nestBase,path,method,body),invoke(rustBase,path,method,body)]);
  const same = nest.status === rust.status && isDeepStrictEqual(nest.data,rust.data);
  if (legacyExceptions.has(label)) {
    console.log('LEGACY_DIFFERENCE '+JSON.stringify({label,nest,rust}));
    legacy++;
  } else if (same) {
    console.log(`PASS error contract: ${label} (${nest.status})`);
    passed++;
  } else {
    console.error('FAIL error contract '+JSON.stringify({label,nest,rust}));
    failed++;
  }
}
console.log(`Error response parity: ${passed} matched, ${failed} mismatched, ${legacy} legacy exceptions`);
if (failed) process.exitCode = 1;
