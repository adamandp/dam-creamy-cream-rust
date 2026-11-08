#!/usr/bin/env node
// HTTP differential contract check. Run against two isolated databases seeded
// with scripts/parity/seed.sql, never against production instances.
import { createHmac } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';

const nestBase = process.env.PARITY_NEST_URL ?? 'http://127.0.0.1:3101';
const rustBase = process.env.PARITY_RUST_URL ?? 'http://127.0.0.1:3102';
const secret = process.env.ACCESS_JWT_SECRET;
if (!secret) throw new Error('ACCESS_JWT_SECRET is required');

const ids = {
  role: '11111111-1111-4111-8111-111111111111',
  user: '22222222-2222-4222-8222-222222222222',
  address: '33333333-3333-4333-8333-333333333333',
  category: '44444444-4444-4444-8444-444444444444',
  product: '55555555-5555-4555-8555-555555555555',
  inventory: '66666666-6666-4666-8666-666666666666',
  review: '77777777-7777-4777-8777-777777777777',
  discount: '88888888-8888-4888-8888-888888888888',
  productDiscount: '99999999-9999-4999-8999-999999999999',
  voucher: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
  userVoucher: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
};

function base64url(input) {
  return Buffer.from(input).toString('base64url');
}

const now = Math.floor(Date.now() / 1000);
const payload = base64url(JSON.stringify({ sub: ids.user, role: ids.role, iat: now, exp: now + 900 }));
const unsigned = `${base64url('{"alg":"HS256","typ":"JWT"}')}.${payload}`;
const token = `${unsigned}.${createHmac('sha256', secret).update(unsigned).digest('base64url')}`;

const cases = [
  // PR1: roles, users and addresses. Each endpoint gets the identical JWT.
  ['roles list', '/roles/?page=1&limit=10'],
  ['roles detail', `/roles/${ids.role}`],
  ['roles by name', '/roles/name/super-admin'],
  ['users list', '/users/?page=1&limit=10'],
  ['users detail', `/users/${ids.user}`],
  ['addresses list', '/addresses/?page=1&limit=10'],
  ['addresses detail', `/addresses/${ids.address}`],

  // PR2: catalog and promotions, including relationships and paging.
  ['categories list', '/categories/?page=1&limit=10'],
  ['categories detail', `/categories/${ids.category}`],
  ['products list', '/products/?page=1&limit=10'],
  ['products catalog', '/products/catalog?page=1&limit=10'],
  ['product detail', `/product/${ids.product}`],
  ['product formatted information', `/product/information/${ids.product}`],
  ['inventory list', '/inventory/?page=1&limit=10'],
  ['inventory detail', `/inventory/${ids.inventory}`],
  ['inventory by product', `/inventory/product/${ids.product}`],
  ['review detail', `/reviews/${ids.review}`],
  ['reviews by product', `/reviews/product/${ids.product}?page=1&limit=10`],
  ['discounts list', '/discounts/?page=1&limit=10'],
  ['discounts detail', `/discounts/${ids.discount}`],
  ['product discounts list', '/product-discounts/?page=1&limit=10'],
  ['product discounts detail', `/product-discounts/${ids.productDiscount}`],
  ['product discounts by product', `/product-discounts/product/${ids.product}`],
  ['vouchers list', '/vouchers/?page=1&limit=10'],
  ['vouchers detail', `/vouchers/${ids.voucher}`],
  ['user vouchers list', '/user-vouchers/?page=1&limit=10'],
  ['user vouchers detail', `/user-vouchers/${ids.userVoucher}`],
  ['user vouchers by voucher', `/user-vouchers/voucher/${ids.voucher}`],
];

// Nest and Axum create independent updatedAt timestamps on mutations; compare
// the actual changed field and the remaining response without that single field.
function comparable(value, ignoreUpdatedAt = false) {
  if (Array.isArray(value)) return value.map((item) => comparable(item, ignoreUpdatedAt));
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value)
        .filter(([key]) => !(ignoreUpdatedAt && key === 'updatedAt'))
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([key, child]) => [key, comparable(child, ignoreUpdatedAt)]),
    );
  }
  return value;
}

async function invoke(base, path, method = 'GET', body) {
  const response = await fetch(new URL(path, base), {
    method,
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: 'application/json',
      ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    signal: AbortSignal.timeout(15000),
  });
  const text = await response.text();
  let data;
  try { data = text ? JSON.parse(text) : null; } catch { data = text; }
  return { status: response.status, data };
}

let passed = 0;
let failed = 0;
async function compare(label, path, method = 'GET', body, options = {}) {
  const [nest, rust] = await Promise.all([
    invoke(nestBase, path, method, body),
    invoke(rustBase, path, method, body),
  ]);
  const statusMatches = nest.status === rust.status;
  const bodyMatches = options.statusOnly || isDeepStrictEqual(
    comparable(nest.data, options.ignoreUpdatedAt),
    comparable(rust.data, options.ignoreUpdatedAt),
  );
  if (statusMatches && bodyMatches && nest.status < 400) {
    console.log(`PASS ${label} (${nest.status})`);
    passed++;
  } else if (statusMatches && bodyMatches && options.expectError && nest.status >= 400) {
    console.log(`PASS ${label} (expected ${nest.status})`);
    passed++;
  } else {
    console.error(`FAIL ${label}: Nest ${nest.status}, Rust ${rust.status}`);
    console.error('  Nest:', JSON.stringify(nest.data));
    console.error('  Rust:', JSON.stringify(rust.data));
    failed++;
  }
}

for (const [label, path] of cases) await compare(label, path);

// Deterministic PATCH scenarios verify both mutations and their DB effects.
await compare('inventory quantity update', `/inventory/${ids.inventory}`, 'PATCH', { quantity: 20 });
await compare('inventory updated detail', `/inventory/${ids.inventory}`, 'GET', undefined, { ignoreUpdatedAt: true });
await compare('category description update', `/categories/${ids.category}`, 'PATCH', { description: 'Updated cream category' });
await compare('category updated detail', `/categories/${ids.category}`, 'GET', undefined, { ignoreUpdatedAt: true });
await compare('category list stays cached after update', '/categories/?page=1&limit=10');
// Both backends reject unchanged PATCH values; error payload envelopes differ.
await compare('category unchanged PATCH', `/categories/${ids.category}`, 'PATCH', { name: 'Cream Cake' }, { statusOnly: true, expectError: true });
await compare('product description update', `/products/${ids.product}`, 'PATCH', { description: 'Creamy vanilla updated' });
await compare('product detail after update', `/product/${ids.product}`);
await compare('review comment update', `/reviews/${ids.review}`, 'PATCH', { comment: 'Loved it' });
await compare('review detail after update', `/reviews/${ids.review}`, 'GET', undefined, { ignoreUpdatedAt: true });
await compare('discount name update', `/discounts/${ids.discount}`, 'PATCH', { name: 'Cream Discount Updated' });
await compare('discount detail after update', `/discounts/${ids.discount}`, 'GET', undefined, { ignoreUpdatedAt: true });
await compare('voucher name update', `/vouchers/${ids.voucher}`, 'PATCH', { name: 'Cream Voucher Updated' });
await compare('voucher detail after update', `/vouchers/${ids.voucher}`, 'GET', undefined, { ignoreUpdatedAt: true });
await compare('user voucher mark used', `/user-vouchers/${ids.userVoucher}`, 'PATCH', {});
await compare('user voucher after used', `/user-vouchers/${ids.userVoucher}`, 'GET', undefined, { ignoreUpdatedAt: true });

console.log(`\nNestJS ↔ Axum differential: ${passed} matched, ${failed} mismatched, ${cases.length + 16} scenarios.`);
if (failed) process.exitCode = 1;
