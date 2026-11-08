# PR3/5 — Commerce and Checkout Parity

Reference: `adamandp/dam-creamy-cream-be@7a3407b` (NestJS). Base: Rust PR2 `feat/pr2-catalog-promotions`.

## HTTP surface

| NestJS endpoint | Rust counterpart | Notes |
| --- | --- | --- |
| `POST /carts/user/add` | same | Authenticated user, quantity increment in transaction |
| `POST /carts/user/remove` | same | Decrement or delete at/below requested quantity |
| `GET /carts/user` | same | Paginated product/category/discount projection |
| `GET /shipments` | same | Paginated list, 404 when empty |
| `GET /shipments/:id` | same | Prisma-compatible output fields |
| `PATCH /shipments/:id/status` | same | Source Nest controller has no authorization guard; same public status update |
| `POST /shipments/cost` | same | RajaOngkir POST as URL-encoded form, same courier set; legacy Nest type-only DTO import leaves HTTP body validation inactive |
| `GET/POST/PATCH/DELETE /payments` | same | Nest's existing **scaffold string responses**, not invented billing logic |
| `/orders` | no public handler | The Nest controller declares **zero routes**; Rust preserves 404 rather than silently exposing checkout |

## Internal services

- `orders::find_all` ports the Nest CQRS query (also not publicly routed).
- `orders::checkout::execute` ports the CQRS command sequence: user, address, product, cart, voucher, user voucher, discount, inventory, cart item, product-discount validation; quantity/counter updates; item and voucher arithmetic; order, order items, free items, voucher links, payment and shipment creation.
- Everything is performed inside **one SeaORM transaction**, preserving rollback and row locking. A failing late order-voucher insert must roll back reserved inventory, cart decrement, discount redemption, voucher use, and partial order rows.
- SeaORM maps the pre-existing quoted PostgreSQL column `order_items."finalPrice"` exactly. The Prisma schema does **not** map this field to `final_price`.
- Shipping voucher is applied to shipping cost, checkout voucher to discounted item total, first matching voucher of each group. Payment method and `method_type` match the Nest create step.

## Intentional differences from a literal port

1. The Nest checkout discount lock uses raw SQL against table `discount` (singular), but Prisma's actual table is `discounts` (plural). Rust locks the **real table** so the intended operation can complete. No existing PostgreSQL schema is changed.
2. Nest checkout can over-reserve inventory or exceed discount caps if a request repeats a product/discount on multiple lines. Rust accumulates the lines before reservation, capacity checks, and updates, preventing lost updates/overselling while keeping the same calculation for ordinary requests.
3. Nest `CurrentUser` reads a refresh-token cookie without verifying its signature. Rust supports **both** a verified, stored `refreshToken` session cookie (for existing clients) and a verified Bearer access JWT. Forged or revoked cookies remain unauthorized instead of cloning Nest's unsigned-decode vulnerability. The Nest shipments status endpoint has no guard; Rust matches that existing route contract.
4. DTO error envelopes for untested malformed bodies and unusual JS number/date coercions can differ. The PR3 HTTP differential explicitly checks cart quantity bounds and missing fields, shipment status enum, and the cost handler's legacy acceptance of missing/extra provider keys.
5. The Nest Payment service consists of literal scaffold messages, **not real payment operations**. Rust keeps those stubs and does not manufacture external payment behavior.

## Verification

- Rust integration test runs on isolated `creamy_cream_test` schema, exercising a late-stage transaction rollback followed by success, plus two competing checkouts on independent PostgreSQL connections where only one may reserve the remaining stock.
- The checkout integration fixture also exercises a legitimate **cookie-only cart request**, forged cookie rejection, and revoked-session rejection against the actual `user_tokens` table.
- Unit tests verify input schema, quantity math and voucher math.
- HTTP differential script compares cart, shipping (including anonymous status update and a complete success/failure RajaOngkir HTTP roundtrip to an isolated provider mock), payments and absent orders routes against pinned NestJS on two separately migrated PostgreSQL databases.
- `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --all-targets --locked` run in CI.

No legacy image/object migration and no data migrations; MinIO remains the presigned direct-upload storage from PR1/PR2.
