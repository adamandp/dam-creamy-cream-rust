#!/usr/bin/env node
// Compare PR3 externally exposed HTTP behavior against the pinned Nest app.
import { createHmac } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';

const bases=[process.env.PARITY_NEST_URL??'http://127.0.0.1:3101',
             process.env.PARITY_RUST_URL??'http://127.0.0.1:3102'];
const secret=process.env.ACCESS_JWT_SECRET;
if (!secret) throw new Error('ACCESS_JWT_SECRET is required');
const id={
  user:'22222222-2222-4222-8222-222222222222',
  role:'11111111-1111-4111-8111-111111111111',
  product:'55555555-5555-4555-8555-555555555555',
  shipment:'15151515-1515-4515-8515-151515151515',
};
const b64=v=>Buffer.from(JSON.stringify(v)).toString('base64url');
const now=Math.floor(Date.now()/1000);
const unsigned=`${b64({alg:'HS256',typ:'JWT'})}.${b64({sub:id.user,role:id.role,iat:now,exp:now+900})}`;
const token=`${unsigned}.${createHmac('sha256',secret).update(unsigned).digest('base64url')}`;

async function invoke(base,method,path,body,authenticated=true) {
  const response=await fetch(`${base}${path}`,{
    method,headers:{...(authenticated?{Authorization:`Bearer ${token}`,
      Cookie:`refreshToken=${token}`} : {}),
      ...(body===undefined?{}:{'Content-Type':'application/json'})},
    ...(body===undefined?{}:{body:JSON.stringify(body)}),signal:AbortSignal.timeout(15000),
  });
  const text=await response.text();
  let data;
  try {data=text?JSON.parse(text):null;} catch {data=text;}
  return {status:response.status,data};
}

const comparable=(v,skipTimestamp=false)=> {
  if (Array.isArray(v)) return v.map(x=>comparable(x,skipTimestamp));
  if (v&&typeof v==='object') return Object.fromEntries(Object.entries(v)
    .filter(([key])=>!skipTimestamp||key!=='updatedAt')
    .sort(([a],[b])=>a.localeCompare(b)).map(([k,val])=>[k,comparable(val,skipTimestamp)]));
  return v;
};

let passed=0,failed=0;
async function compare(label,method,path,body,skipTimestamp=false,authenticated=true) {
  const [nest,rust]=await Promise.all(bases.map(base=>invoke(base,method,path,body,authenticated)));
  if (nest.status===rust.status && isDeepStrictEqual(comparable(nest.data,skipTimestamp),comparable(rust.data,skipTimestamp))) {
    console.log(`PASS PR3 ${label} (${nest.status})`);passed++;
  } else {
    console.error(`FAIL PR3 ${label}: nest ${JSON.stringify(nest)} rust ${JSON.stringify(rust)}`);failed++;
  }
}

await compare('cart initial','GET','/carts/user?page=1&limit=10');
await compare('cart missing session','GET','/carts/user?page=1&limit=10',undefined,false,false);
await compare('cart add','POST','/carts/user/add',{productId:id.product,quantity:2});
await compare('cart increment readback','GET','/carts/user?page=1&limit=10');
await compare('cart remove','POST','/carts/user/remove',{productId:id.product,quantity:3});
await compare('cart decrement readback','GET','/carts/user?page=1&limit=10');
await compare('cart missing product','POST','/carts/user/add',
  {productId:'aaaaaaaa-1111-4111-8111-111111111111',quantity:1});
await compare('cart quantity below minimum','POST','/carts/user/add',{productId:id.product,quantity:0});
await compare('cart quantity above maximum','POST','/carts/user/remove',{productId:id.product,quantity:101});
await compare('cart missing quantity','POST','/carts/user/add',{productId:id.product});
await compare('cart missing product ID','POST','/carts/user/add',{quantity:1});
await compare('shipments list','GET','/shipments?page=1&limit=10');
await compare('shipments detail','GET',`/shipments/${id.shipment}`);
await compare('shipments status update','PATCH',`/shipments/${id.shipment}/status`,{status:'SHIPPED'});
await compare('shipments anonymous status update','PATCH',`/shipments/${id.shipment}/status`,{status:'DELIVERED'},false,false);
await compare('shipments invalid status','PATCH',`/shipments/${id.shipment}/status`,{status:'PACKING'});
await compare('shipments updated detail','GET',`/shipments/${id.shipment}`,undefined,true);
await compare('RajaOngkir successful upstream quote','POST','/shipments/cost',{origin:1,destination:2,weight:1000});
await compare('RajaOngkir rejects unknown field','POST','/shipments/cost',
  {origin:1,destination:2,weight:1000,unrecognized:true});
await compare('RajaOngkir missing origin','POST','/shipments/cost',{destination:2,weight:1000});
for (const [label,method,path,body] of [
  ['payments scaffold list','GET','/payments'],
  ['payments scaffold detail','GET','/payments/123'],
  ['payments JS hexadecimal number','GET','/payments/0x10'],
  ['payments JS infinity number','GET','/payments/Infinity'],
  ['payments scaffold create','POST','/payments',{}],
  ['payments scaffold update','PATCH','/payments/123',{}],
  ['payments scaffold delete','DELETE','/payments/123'],
  ['orders route absent','GET','/orders'],
]) await compare(label,method,path,body);

console.log(`PR3 commerce parity: ${passed} matched, ${failed} mismatched`);
if (failed) process.exitCode=1;
