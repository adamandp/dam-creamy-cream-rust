# DAM Creamy Cream — Rust API

Rust/Axum + SeaORM rewrite of [`dam-creamy-cream-be`](https://github.com/adamandp/dam-creamy-cream-be), built as a feature-oriented modular monolith.

## PR 1 of 5 — foundation and identity

This first increment implements the existing NestJS routes for:

| Area | Routes |
| --- | --- |
| Auth | `POST /auth/dummy-customer-login`, `/auth/logout`, `/auth/logout-all` |
| Sessions | `POST /sessions/refresh` |
| Roles | `POST /roles`, `GET /roles`, `GET /roles/{id}`, `GET /roles/name/{name}`, `PATCH /roles/{id}`, `DELETE /roles/{id}` |
| Users | `GET /users`, `GET /users/profile`, `GET /users/{id}`, `PATCH /users/{id}`, `PUT /users/{id}/password`, `DELETE /users/{id}` |
| Addresses | `POST /addresses`, `GET /addresses`, `GET /addresses/user`, `GET /addresses/{id}`, `PATCH /addresses/{id}`, `DELETE /addresses/{id}` |

`UsersService.create()` exists as `features::users::create_internal` for inter-module use. NestJS does **not** register a `POST /users` controller endpoint, so neither does this rewrite.

### Agreed storage API change

The old multipart `PUT /users/{id}/image` is replaced with **direct MinIO upload**:

1. With `Authorization: Bearer <accessToken>`, call `POST /users/{id}/image/presign` with `{ "fileName": "avatar.webp", "fileSize": 12345, "mimeType": "image/webp" }`.
2. Upload the image bytes directly to the returned `data.uploadUrl` using HTTP `PUT`, exactly the returned `data.headers` (`Content-Type` and `Content-Length`). Do not pass the backend Bearer token to MinIO.
3. With the same Bearer token, call `POST /users/{id}/image/confirm` with `{ "key": "<data.key>" }`.
4. The backend HEAD-checks the upload and fully decodes the image as its declared type, with a 2 MiB byte limit, 4096-pixel dimension limit, and 32 MiB decoder-memory limit. It conditionally copies the staged object to a permanent key before storing the stable `imageUrl` in the original `users.image_url` column. The permanent key cannot be overwritten using the original presigned PUT URL.
5. `GET /media/users/{id}/{asset}` redirects browsers to a short-lived MinIO presigned GET URL, avoiding public buckets and expiring URLs in the database.

Uploads are limited to 2 MiB and `image/jpeg`, `image/png`, or `image/webp`. The backend attempts to delete previously confirmed images from MinIO after successful replacement; failed cleanup is logged for operator follow-up without reporting a false upload failure. Staged objects that were never confirmed use the `pending/` prefix: **configure a MinIO lifecycle rule to expire `pending/` objects**, for example after 24 hours. The MinIO instance must be **separate** from Chat With PDF and reachable by frontend browsers at `MINIO_ENDPOINT`. Configure bucket CORS to allow `PUT` with `Content-Type` from your frontend origin. No Cloudinary migration is included.

### Infrastructure

- PostgreSQL uses **the exact existing Prisma schema**; entity column mapping preserves `browserInfo` and `expiredAt` in `user_tokens`, which are **not** snake_case.
- The existing Prisma SQL migrations remain authoritative. This PR does not drop/create/change production tables or data. Apply the NestJS migrations to new development databases before running this API.
- SeaORM creates a shared PostgreSQL connection pool, and the app verifies Redis with PING and MinIO bucket availability at startup.
- JWT access tokens are HS256, 15-minute; refresh tokens HS256, 7-day, persisted in `user_tokens` and issued in an HttpOnly `refreshToken` cookie (`SameSite=Lax`, Secure in production). Refresh JWTs use a unique `jti` instead of a redundant role claim, so distinct logins cannot collide while fitting the existing `VARCHAR(255)` token column. Refresh issues access tokens using the current database role. Inactive users cannot authenticate or refresh; only the intentionally inactive legacy `DummyUser` is exempt in development. Password changes revoke all refresh sessions atomically.
- The intentionally insecure legacy dummy login is **development-only** (`NODE_ENV=production` returns 404). Production requires a configured `FRONTEND_ORIGINS`, HTTPS for public MinIO URLs, and distinct access/refresh JWT secrets.
- Session browser/OS/device metadata is parsed using Woothee. Its exact classifications can differ from the legacy `ua-parser-js` implementation, while the stored JSON field names remain the same.
- `PASSWORD` hashing continues to use bcrypt cost 10; password comparison is run in a blocking worker.
- JSON responses retain NestJS `message`, `data`, and `paging` conventions, with the same emoji response messages where possible. Invalid JSON bodies and pagination queries use the same JSON error envelope (HTTP 400) as application validation errors.

### Environment and running

Requires Rust 1.94+, PostgreSQL with the pre-existing Creamy Cream schema, Redis, and a dedicated MinIO instance.

```bash
cp .env.example .env
# Set the connection URLs and secret keys in .env
cargo run
```

Compile and verify:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Do not share MinIO access keys with the frontend; only presigned URLs are handed to browsers. Configure `PUBLIC_BASE_URL` to the public backend origin so persisted image URLs work outside localhost. Use HTTPS for MinIO in production.

### Important parity notes

1. **Explicitly approved difference:** image uploads use presigned direct PUT instead of multipart; frontend integration must change.
2. **Security correction:** NestJS `POST /sessions/refresh` signs a **refresh JWT** while naming it `accessToken`. Rust returns a real 15-minute **access JWT**, checks the refresh JWT against the database, and rejects revoked sessions. The JSON shape stays unchanged.
3. **Security correction:** `GET /users/profile` verifies the Bearer signature instead of only decoding the token; `/addresses/user` verifies the refresh cookie and its active database session instead of decoding an unverified cookie.
4. **Authorized behavior change (PR 1 follow-up):** role mutations and role assignment require `super-admin`; user and address collections require `admin` or `super-admin`, while single records and mutations require ownership or administrator privileges. Administrators can change activation status only for other non-super-admin users, and nobody can self-change role or activation status. Password changes require the account's existing password through `PUT /users/{id}/password` and revoke all refresh sessions. Address ownership transfers require administrator privileges. JWT roles are resolved from the database on each protected request, so demoted users cannot retain elevated permissions with old access tokens. Inactive accounts are blocked except for legacy `DummyUser` in development.
5. **User deletion:** deleting accounts without purchase history also removes their dependent carts/cart items, refresh tokens, addresses, reviews, vouchers, analytics, and notifications in a database transaction; users referenced by orders or order addresses return HTTP 409 to preserve financial history. No schema migration or order data destruction is needed.
6. **Address invariant:** transactional address updates, transfers, and deletes serialize on user rows and repair the primary-address invariant (exactly one primary whenever a user has an address).
7. CI uses an isolated PostgreSQL database to test authorization, account lifecycle, address transfers, primary-address updates, and concurrent creates. These tests never migrate or modify the existing application database. An additional MinIO integration test verifies real image decoding, conditional copy, and deletion when `TEST_MINIO_ENDPOINT` points to a separately running test MinIO instance with the test credentials specified in the test. The official public MinIO container images are currently not accessible to CI, so that optional test is not run automatically. Remaining catalog, promotion, commerce, and supporting endpoints are scheduled for PRs 2–4, with broader cross-module E2E testing in PR 5.

This implementation is not a new database schema, is not intended to rewrite the original NestJS repository, and does not include a migration from Cloudinary.
