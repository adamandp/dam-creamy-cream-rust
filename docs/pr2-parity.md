# PR 2 — NestJS → Axum behavioral parity baseline

Reference backend: `adamandp/dam-creamy-cream-be` at `7a3407b` (`master`).
Target: the stacked Rust PR #2, based on Rust PR #1. The NestJS business rules
are the reference, **including known bugs**. A security or upload change is
not considered equivalent unless called out below.

## Scope

| NestJS module | Axum module | Scope of comparison |
| --- | --- | --- |
| `categories` | `categories.rs` | CRUD, cached list, DTO names, pagination |
| `products` | `products.rs` | CRUD, public list and catalog, image URL |
| `product` | `product.rs` | Product detail, review average, sales and formatted information |
| `inventory` | `inventory.rs` | List, by ID/product and patch semantics |
| `reviews` | `reviews.rs` | CRUD, user/product-specific lists and pagination |
| `discounts` | `discounts.rs` | CRUD, DTO rules and legacy response labels |
| `product-discounts` | `product_discounts.rs` | Create, list, by ID/product and deactivate |
| `vouchers` | `vouchers.rs` | CRUD, DTO rules and soft deactivation |
| `user-vouchers` | `user_vouchers.rs` | Create, list, joined by-user list and mark USED |

## Legacy behaviors deliberately preserved

- `ProductDiscountsService.findAll()` takes its `totalItems` from **users**,
  not product discounts. A product-discount list can therefore be empty while
  pagination indicates existing items, and its 404 condition follows user count.
- `ReviewsService.findByUser()` and `findByProduct()` count **all reviews** for
  `totalItems` although their result rows are filtered.
- `CategoriesService.findAll()` caches under `categories:{page}:{limit}` for
  one hour and **does not invalidate** the cached list when categories change.
- The original InventoryService has **no constraint** that stock must be
  nonnegative or that reserved stock must be at most total stock on PATCH.
- Promotion DTOs validate types/format but do **not** reject negative numeric
  quantities, redemption counters, or reversed date intervals. Voucher claim
  dates also have no ordering rule.
- `baseName` DTOs accept ASCII letters and whitespace only (not punctuation).
- Standard collection endpoints do not request an explicit SQL sort. Product
  catalog retains NestJS's explicit `createdAt DESC` or price sort.
- Nest's `validateUpdate` rejects a PATCH when **any** submitted field equals
  its prior value; the Rust catalog PATCH handlers now share the same error
  message convention through `require_changed_update`.
- `DiscountsService.findAll()` intentionally uses the legacy `Users` error
  and `User` success message, even though it returns discounts.

## Explicit exceptions to 1:1

1. Product multipart image upload is replaced by direct-to-MinIO presign and
   confirm, per the agreed upload contract. No legacy object migration.
2. Authorization checks added in PR #1 and PR #2 remain in place for private
   and write endpoints; public routes and product listing remain public.
3. Rust returns typed/structured 400/404 errors for several malformed inputs,
   while Nest can fail earlier in the DTO pipe or Prisma. Byte-for-byte error
   envelope parity is **not yet established**.
4. Nest `ReviewsController` omits parameter decorators on `findAll` and
   `findByUser`. The Rust handlers preserve the intended list behavior rather
   than intentionally reproducing a missing-decorator runtime failure.
5. The PR #1 address write paths deliberately enforce a single-primary
   invariant and reject some ownership transfers that the Nest update path
   permits. This is a prior security/data-integrity change, **not** 1:1 logic.
6. Zod's date coercion accepts a wider variety of JavaScript date inputs than
   the Rust ISO/date-only parser (notably numeric timestamps); those formats
   are not yet contract-equivalent.

## Verification / limits

- `src/pr2_tests.rs` exercises an isolated PostgreSQL schema when
  `TEST_DATABASE_URL` is configured. Additional checks characterize legacy
  pagination, name validation, and inventory patch behavior.
- CI builds the fixed NestJS reference SHA, starts both backends against
  **separate migrated PostgreSQL databases seeded with identical fixtures**,
  runs independent Redis databases, and compares real HTTP statuses, response
  JSON, PATCH responses and persisted updates using
  `scripts/parity/compare.mjs`. Any unexpected mismatch fails CI.
- The differential matrix covers the original 28 GET scenarios, 16 additional
  PATCH/readback/cache-staleness cases, plus catalog query variants and
  negative/positive Zod DTO cases (unknown keys, required-nullable fields,
  strict objects, coercion and date-only values), plus normalized DB
  readbacks after create, hard delete, and soft-deactivation operations.
  It intentionally excludes
  legacy broken controller decorators, different authorization rules, and
  the agreed upload change.
- CI also boots an isolated MinIO instance and executes the storage integration
  test with a **real presigned HTTP PUT**, verify, immutable copy, GET and
  cleanup; it fails if `TEST_MINIO_ENDPOINT` is unavailable.
- After differential checks, `scripts/parity/upload.mjs` exercises the full
  authenticated Rust API flow: presign, browser CORS preflight, PUT into MinIO,
  confirm, public media GET and idempotent confirm retry. The separate MinIO
  deployment must explicitly allow the frontend origin via its own CORS setup
  (for example `MINIO_API_CORS_ALLOW_ORIGIN`).
- Running both test sets requires PostgreSQL, Redis, MinIO, Node.js, and Rust.
  Differential testing of the entire route/DTO edge-case matrix and
  all browser-specific CORS and deployment conditions is still **not complete**;
  the current matrix is a reproducible parity gate, not 100% certification.
