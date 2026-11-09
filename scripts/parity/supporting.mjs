#!/usr/bin/env node
// Real HTTP differential: PR4. Only touches two disposable CI databases.
import { createHmac } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';

const nestBase = process.env.PARITY_NEST_URL ?? 'http://127.0.0.1:3101';
const rustBase = process.env.PARITY_RUST_URL ?? 'http://127.0.0.1:3102';
const secret = process.env.ACCESS_JWT_SECRET;
if (!secret) throw new Error('ACCESS_JWT_SECRET is required');
const userId = '22222222-2222-4222-8222-222222222222';
const productId = '55555555-5555-4555-8555-555555555555';
const notificationId = '17171717-1717-4717-8717-171717171717';
const userNotificationId = '18181818-1818-4818-8818-181818181818';
const payload = { sub: userId, role: '11111111-1111-4111-8111-111111111111', iat: Math.floor(Date.now()/1000), exp: Math.floor(Date.now()/1000)+900 };
const unsigned = `${Buffer.from('{"alg":"HS256","typ":"JWT"}').toString('base64url')}.${Buffer.from(JSON.stringify(payload)).toString('base64url')}`;
const token = `${unsigned}.${createHmac('sha256',secret).update(unsigned).digest('base64url')}`;

function normalize(value, ignoreUpdatedAt=false) {
  if (Array.isArray(value)) return value.map(v=>normalize(v,ignoreUpdatedAt));
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value)
    .filter(([key])=>!ignoreUpdatedAt || key!=='updatedAt')
    .map(([key,v])=>[key,normalize(v,ignoreUpdatedAt)]).sort(([a],[b])=>a.localeCompare(b)));
  return value;
}
async function invoke(base,path,method='GET',body) {
  const res=await fetch(new URL(path,base), { method, headers:{Authorization:`Bearer ${token}`,...(body===undefined?{}:{'Content-Type':'application/json'})},
    ...(body===undefined?{}:{body:JSON.stringify(body)}),signal:AbortSignal.timeout(15000)});
  const raw=await res.text();
  let data;try{data=JSON.parse(raw)}catch{data=raw}
  return {status:res.status,data};
}
let passed=0,failed=0;
async function compare(label,path,method='GET',body,ignoreUpdatedAt=false) {
  const [nest,rust]=await Promise.all([invoke(nestBase,path,method,body),invoke(rustBase,path,method,body)]);
  if(nest.status===rust.status && isDeepStrictEqual(normalize(nest.data,ignoreUpdatedAt),normalize(rust.data,ignoreUpdatedAt))) {
    console.log(`PASS ${label} (${nest.status})`);passed++;
  } else {
    console.error(`FAIL ${label}: Nest ${nest.status} ${JSON.stringify(nest.data)} | Rust ${rust.status} ${JSON.stringify(rust.data)}`);failed++;
  }
}

await compare('analytics product','/product-analytics/product/'+productId+'?page=1&limit=10');
await compare('analytics count','/product-analytics/product/'+productId+'/count');
await compare('analytics user','/product-analytics/user?page=1&limit=10');
await compare('recommendation categories','/recommendations/categories');
await compare('recommendation offers','/recommendations/offers');
await compare('recommendation related','/recommendations/related/'+productId);
await compare('notifications list','/notifications?page=1&limit=10');
await compare('notifications detail','/notifications/'+notificationId);
await compare('user notifications list','/user-notifications?page=1&limit=10');
await compare('user notifications detail','/user-notifications/'+userNotificationId);
await compare('user notifications count','/user-notifications/count/me');
await compare('notification user associations','/user-notifications/notification/'+notificationId+'?page=1&limit=10');
await compare('notification user associations count','/user-notifications/notification/'+notificationId+'/count');
await compare('analytics create','/product-analytics','POST',{userId,productId});
await compare('notification update','/notifications/'+notificationId,'PATCH',{title:'Creamy News'});
await compare('notification updated detail','/notifications/'+notificationId,'GET',undefined,true);
await compare('user notification mark read','/user-notifications/'+userNotificationId,'PATCH');
await compare('user notification updated detail','/user-notifications/'+userNotificationId,'GET',undefined,true);
console.log(`PR4 supporting differential: ${passed} matched / ${failed} mismatched`);
if (failed) process.exitCode=1;
