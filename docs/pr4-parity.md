# PR4 — Supporting features parity notes

Source of truth: `adamandp/dam-creamy-cream-be` (`master`, `7a3407b`). This PR is stacked on `feat/pr3-commerce-checkout`.

## Routes covered

- `product-analytics`: create, by-product list/count, current-user list.
- `recommendations`: classic, categories, offers, related product.
- `notifications`: CRUD.
- `user-notifications`: create, list, detail, current-user list/count, notification list/count, mark read.
- `health`: existing in the PR1–PR3 base; unchanged.

SeaORM entities map directly to existing PostgreSQL tables (`product_analytics`, `notifications`, `user_notifications`). There are no migrations and no Cloudinary asset migration.

## Legacy quirks deliberately preserved

- Product analytics product/user lists use the **global** analytics count for pagination, as in the Nest service.
- User-notification lookup by notification ID filters on **user ID** in Nest; this counterintuitive query remains unchanged to avoid silently changing behavior.
- Offer recommendations take five products before their application-level discount/rating sort. Discount mapping retains the original `FIXED`/`PERCENTAGE` behavior and nullable fields.
- Related recommendations rank category matches and fill remaining slots from other active products.

## Deliberate differences from legacy

- JWTs are **verified**, rather than merely decoded. A user can access their own notifications and analytics; administrative collection/mutation operations require an admin. This follows the authorization strengthening started in PR1–PR3.
- Legacy analytics DTO permits `null` UUIDs, but the Nest service substitutes the string `unkown`, which fails the PostgreSQL UUID foreign keys. Rust requires valid UUIDs instead of deliberately triggering a database error.
- Notification title length is limited to 50 characters by the actual PostgreSQL column, despite the Nest Zod schema accepting up to 100.
- Database errors are mapped through the shared typed Rust `AppError`; raw Prisma exceptions are not recreated.

## Verification

The latest PR4 GitHub Actions run (`42602650041`) passed:

- `cargo fmt --all -- --check`, `cargo clippy --all-targets --locked -- -D warnings`.
- `cargo test --all-targets --locked`: **42 passed**, including four recommendation regression tests.
- Existing HTTP contract differential: **113 matched, 0 mismatched**.
- Existing error contract differential: **38 matched, 0 mismatched** (five known Nest legacy exceptions tracked separately).
- PR3 commerce differential: **28 matched, 0 mismatched**.
- PR4 supporting-feature HTTP differential against two independent, identically migrated PostgreSQL databases: **22 matched, 0 mismatched**. The cases cover create/read/update, product analytics, recommendations (classic, offers, categories and related), count and read-state behavior. PR4 recommendations receive additional deterministic fixtures after the base suites and before the Rust-only MinIO upload changes image URLs.

This demonstrates parity on the **tested cases**, not mathematical proof that arbitrary requests, concurrent traffic or all legacy bugs will behave identically. The intentional security/API differences described above remain.
