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
  ['roles list', '/roles?page=1&limit=10'],
  ['roles detail', `/roles/${ids.role}`],
  ['roles by name', '/roles/name/super-admin'],
  ['users list', '/users?page=1&limit=10'],
  ['users detail', `/users/${ids.user}`],
  ['addresses list', '/addresses?page=1&limit=10'],
  ['addresses detail', `/addresses/${ids.address}`],

  // PR2: catalog and promotions, including relationships and paging.
  ['categories list', '/categories?page=1&limit=10'],
  ['categories detail', `/categories/${ids.category}`],
  ['products list', '/products?page=1&limit=10'],
  ['products catalog', '/products/catalog?page=1&limit=10'],
  ['product detail', `/product/${ids.product}`],
  ['product formatted information', `/product/information/${ids.product}`],
  ['inventory list', '/inventory?page=1&limit=10'],
  ['inventory detail', `/inventory/${ids.inventory}`],
  ['inventory by product', `/inventory/product/${ids.product}`],
  ['review detail', `/reviews/${ids.review}`],
  ['reviews by product', `/reviews/product/${ids.product}?page=1&limit=10`],
  ['discounts list', '/discounts?page=1&limit=10'],
  ['discounts detail', `/discounts/${ids.discount}`],
  ['product discounts list', '/product-discounts?page=1&limit=10'],
  ['product discounts detail', `/product-discounts/${ids.productDiscount}`],
  ['product discounts by product', `/product-discounts/product/${ids.product}`],
  ['vouchers list', '/vouchers?page=1&limit=10'],
  ['vouchers detail', `/vouchers/${ids.voucher}`],
  ['user vouchers list', '/user-vouchers?page=1&limit=10'],
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

for (const [label, path] of [
  ['products catalog default sorting', '/products/catalog?sort=default'],
  ['products catalog price ascending', '/products/catalog?sort=asc'],
  ['products catalog price descending', '/products/catalog?sort=desc'],
  ['products catalog search', '/products/catalog?q=Cream'],
  ['products catalog category', `/products/catalog?cat=${ids.category}`],
  ['products catalog minimum price', '/products/catalog?min=1000'],
  ['products catalog maximum price', '/products/catalog?max=50000'],
  ['products catalog min/max range', '/products/catalog?min=1000&max=50000'],
  ['products catalog second page', '/products/catalog?page=2&limit=1'],
  ['users profile', '/users/profile'],
  ['roles second page', '/roles?page=2&limit=1'],
  ['products second page', '/products?page=2&limit=1'],
  ['categories second page', '/categories?page=2&limit=1'],
]) await compare(label, path);

// Deterministic PATCH scenarios verify both mutations and their DB effects.
await compare('inventory quantity update', `/inventory/${ids.inventory}`, 'PATCH', { quantity: 20 });
await compare('inventory updated detail', `/inventory/${ids.inventory}`, 'GET', undefined, { ignoreUpdatedAt: true });
await compare('category description update', `/categories/${ids.category}`, 'PATCH', { description: 'Updated cream category' });
await compare('category updated detail', `/categories/${ids.category}`, 'GET', undefined, { ignoreUpdatedAt: true });
await compare('category list stays cached after update', '/categories?page=1&limit=10');
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

// Phase 2: characterization of Nest's Zod DTO boundaries. Errors only
// compare HTTP status, because the two frameworks use different envelopes.
// Successful creates check the response envelope and status; IDs/timestamps
// vary by database, so we verify follow-up state independently later.
const expectError = { statusOnly: true, expectError: true };
const statusOnly = { statusOnly: true };
await compare('products catalog unknown search', '/products/catalog?q=nomatch123', 'GET', undefined, expectError);
await compare('catalog strict query rejects unknown keys', '/products/catalog?unexpected=x', 'GET', undefined, expectError);
const discountCreate = {
  name: 'Cream Sale', discountType: 'FIXED', value: null,
  maxApplied: 1, maxRedeemed: 10, redeemedCount: 0,
  description: null, productId: null, quantity: null,
  startDate: '2026-11-08T00:00:00.000Z', endDate: '2026-12-08T00:00:00.000Z',
};
const voucherCreate = {
  code: 'CREAM-PARITY', name: 'Cream Voucher Parity', description: null,
  type: 'FIXED', group: 'CHECKOUT', value: null, productId: null,
  quantity: null, minimumPurchase: 1000, isActive: true,
  startDate: '2026-11-08T00:00:00.000Z', endDate: '2026-12-08T00:00:00.000Z',
  maxRedeemed: 10, redeemedCount: 0,
};
const addressCreate = {
  userId: ids.user, recipientName: 'Parity', recipientPhoneNumber: '081234567890',
  province: 'Banten', city: 'Tangerang', subdistrict: 'Serpong',
  village: 'Lengkong', postalCode: '15310', address: 'Cream Street', isPrimary: 'false',
};
await compare('category missing required nullable description', '/categories', 'POST', { name: 'Cream Category', isActive: true }, expectError);
await compare('category unknown field stripped', '/categories', 'POST', { name: 'Cream New', description: null, isActive: true, discardedField: 123 }, statusOnly);
await compare('category description null allowed', '/categories', 'POST', { name: 'Cream Nullable', description: null, isActive: false }, statusOnly);
await compare('category PATCH strips unknown keys', `/categories/${ids.category}`, 'PATCH', { name: 'Cream Catalogue', ignored: 123 });
await compare('category detail after unknown-key PATCH', `/categories/${ids.category}`, 'GET', undefined, { ignoreUpdatedAt: true });
await compare('product missing required nullable description', '/products', 'POST', { categoryId: ids.category, name: 'Cream Product', price: 12000, isActive: true }, expectError);
await compare('product strictly rejects unknown fields', '/products', 'POST', { categoryId: ids.category, name: 'Cream Product', description: null, price: 12000, isActive: true, extra: true }, expectError);
for (const field of ['value', 'description', 'productId', 'quantity']) {
  const invalid = { ...discountCreate };
  delete invalid[field];
  await compare(`discount missing nullable ${field}`, '/discounts', 'POST', invalid, expectError);
}
await compare('discount strips unknown and accepts explicit nulls', '/discounts', 'POST', { ...discountCreate, ignored: 'okay' }, statusOnly);
await compare('discount accepts date-only strings like Zod', '/discounts', 'POST', {
  ...discountCreate, name: 'Cream Date Sale', startDate: '2026-11-08', endDate: '2026-12-08',
}, statusOnly);
await compare('discount PATCH strips unknown keys', `/discounts/${ids.discount}`, 'PATCH', { description: 'Cream discount extra', ignored: 7 });
await compare('discount after unknown-key PATCH', `/discounts/${ids.discount}`, 'GET', undefined, { ignoreUpdatedAt: true });
for (const field of ['value', 'description', 'productId', 'quantity']) {
  const invalid = { ...voucherCreate };
  delete invalid[field];
  await compare(`voucher missing nullable ${field}`, '/vouchers', 'POST', invalid, expectError);
}
await compare('voucher strips unknown and accepts explicit nulls', '/vouchers', 'POST', { ...voucherCreate, ignored: 'okay' }, statusOnly);
await compare('voucher PATCH strips unknown keys', `/vouchers/${ids.voucher}`, 'PATCH', { description: 'Cream voucher extra', ignored: 7 });
await compare('voucher after unknown-key PATCH', `/vouchers/${ids.voucher}`, 'GET', undefined, { ignoreUpdatedAt: true });
await compare('review strips unknown fields', '/reviews', 'POST', { userId: ids.user, productId: ids.product, rating: 4, comment: null, unknown: 'ignored' }, statusOnly);
await compare('review invalid rating', '/reviews', 'POST', { userId: ids.user, productId: ids.product, rating: 6 }, expectError);
await compare('product-discount missing discountId', '/product-discounts', 'POST', { productId: ids.product }, expectError);
await compare('user-voucher strips unknown fields', '/user-vouchers', 'POST', { userId: ids.user, voucherId: ids.voucher, claimedAt: '2026-11-08T00:00:00.000Z', expiredAt: '2026-12-08T00:00:00.000Z', ignored: true }, statusOnly);
await compare('address coerce boolean string', '/addresses', 'POST', addressCreate, statusOnly);
await compare('address reject invalid phone', '/addresses', 'POST', { ...addressCreate, recipientPhoneNumber: 'abc' }, expectError);
await compare('address strict extra fields', '/addresses', 'POST', { ...addressCreate, unexpected: 42 }, expectError);
await compare('user fullName accepts punctuation', `/users/${ids.user}`, 'PATCH', { fullName: 'Parity Admin, Jr.' });
await compare('user fullName after PATCH', `/users/${ids.user}`, 'GET', undefined, { ignoreUpdatedAt: true });

// Writes that generate IDs on both databases cannot be compared by raw IDs.
// Fetch them back from the persisted collections and assert every business
// field matches, ignoring only IDs and write-generated timestamps.
async function compareCreatedRows(label, path, selected) {
  const [nest, rust] = await Promise.all([invoke(nestBase, path), invoke(rustBase, path)]);
  const project = (result) => (Array.isArray(result.data?.data) ? result.data.data : [])
    .filter(selected)
    .map(({ id, createdAt, updatedAt, ...fields }) => comparable(fields))
    .sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
  const nRows = project(nest);
  const rRows = project(rust);
  if (nest.status === 200 && rust.status === 200 && nRows.length > 0 && isDeepStrictEqual(nRows, rRows)) {
    console.log(`PASS ${label} (${nRows.length} persisted records)`);
    passed++;
  } else {
    console.error(`FAIL ${label}: Nest ${nest.status} ${JSON.stringify(nRows)}, Rust ${rust.status} ${JSON.stringify(rRows)}`);
    failed++;
  }
}
await compareCreatedRows('category creates persist identical fields', '/categories?page=1&limit=100', (row) => ['Cream New', 'Cream Nullable'].includes(row.name));
await compareCreatedRows('discount creates persist identical fields', '/discounts?page=1&limit=100', (row) => ['Cream Sale', 'Cream Date Sale'].includes(row.name));
await compareCreatedRows('voucher create persists identical fields', '/vouchers?page=1&limit=100', (row) => row.code === 'CREAM-PARITY');
await compareCreatedRows('review create persists identical fields', `/reviews/product/${ids.product}?page=1&limit=100`, (row) => row.comment === null && row.rate === 4);
await compareCreatedRows('user voucher create persists identical fields', '/user-vouchers?page=1&limit=100', (row) => row.expiredAt === '2026-12-08T00:00:00.000Z');
await compareCreatedRows('address boolean coercion persists identical fields', '/addresses?page=1&limit=100', (row) => row.recipientName === 'Parity');

async function compareDeleteCreated(label, collection, resourcePath, matching, soft = false) {
  const [nestList, rustList] = await Promise.all([invoke(nestBase, collection), invoke(rustBase, collection)]);
  const nestId = nestList.data?.data?.find(matching)?.id;
  const rustId = rustList.data?.data?.find(matching)?.id;
  if (!nestId || !rustId) {
    console.error(`FAIL ${label}: created record missing in one database`);
    failed++;
    return;
  }
  const [nest, rust] = await Promise.all([
    invoke(nestBase, `${resourcePath}/${nestId}`, 'DELETE'),
    invoke(rustBase, `${resourcePath}/${rustId}`, 'DELETE'),
  ]);
  const [nestAfter, rustAfter] = await Promise.all([
    invoke(nestBase, `${resourcePath}/${nestId}`),
    invoke(rustBase, `${resourcePath}/${rustId}`),
  ]);
  const afterMatches = soft
    ? nestAfter.status === 200 && rustAfter.status === 200 &&
      nestAfter.data?.data?.isActive === false && rustAfter.data?.data?.isActive === false
    : nestAfter.status === rustAfter.status && nestAfter.status >= 400;
  if (nest.status === 200 && rust.status === 200 && isDeepStrictEqual(nest.data, rust.data) && afterMatches) {
    console.log(`PASS ${label} (delete and readback)`);
    passed++;
  } else {
    console.error(`FAIL ${label}: Nest ${nest.status} ${JSON.stringify(nest.data)}, Rust ${rust.status} ${JSON.stringify(rust.data)}; readback ${nestAfter.status}/${rustAfter.status}`);
    failed++;
  }
}
await compareDeleteCreated('category hard delete', '/categories?page=1&limit=100', '/categories', (row) => row.name === 'Cream New');
await compareDeleteCreated('discount hard delete', '/discounts?page=1&limit=100', '/discounts', (row) => row.name === 'Cream Date Sale');
await compareDeleteCreated('voucher soft delete', '/vouchers?page=1&limit=100', '/vouchers', (row) => row.code === 'CREAM-PARITY', true);
await compareDeleteCreated('review hard delete', `/reviews/product/${ids.product}?page=1&limit=100`, '/reviews', (row) => row.comment === null && row.rate === 4);
await compare('product discount soft delete', `/product-discounts/${ids.productDiscount}`, 'DELETE');
await compare('product discount after soft delete', `/product-discounts/${ids.productDiscount}`, 'GET', undefined, { ignoreUpdatedAt: true });

console.log(`\nNestJS ↔ Axum differential: ${passed} matched, ${failed} mismatched, ${passed + failed} scenarios.`);
if (failed) process.exitCode = 1;
