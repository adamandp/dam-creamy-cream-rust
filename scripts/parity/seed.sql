-- Deterministic, disposable data for an HTTP-level comparison of NestJS and Axum.
-- Apply only to freshly migrated CI databases; never to a user database.
INSERT INTO roles (id, name, created_at, updated_at) VALUES
('11111111-1111-4111-8111-111111111111', 'super-admin', '2026-01-01', '2026-01-01');

INSERT INTO users (id, role_id, username, email, password, full_name, is_active, created_at, updated_at) VALUES
('22222222-2222-4222-8222-222222222222', '11111111-1111-4111-8111-111111111111', 'ParityAdmin', 'parity@example.invalid', 'unused-test-password', 'Parity Admin', true, '2026-01-01', '2026-01-01');

INSERT INTO addresses (id, user_id, recipient_name, recipient_phone_number, province, city, subdistrict, village, postal_code, address, is_primary, created_at, updated_at) VALUES
('33333333-3333-4333-8333-333333333333', '22222222-2222-4222-8222-222222222222', 'Parity Admin', '081234567890', 'Jawa Barat', 'Bandung', 'Coblong', 'Dago', '40135', 'Jalan Test Satu', true, '2026-01-01', '2026-01-01');

INSERT INTO category (id, name, description, is_active, created_at, updated_at) VALUES
('44444444-4444-4444-8444-444444444444', 'Cream Cake', 'Dairy cakes', true, '2026-01-01', '2026-01-01');

INSERT INTO products (id, category_id, name, image_url, description, price, is_active, created_at, updated_at) VALUES
('55555555-5555-4555-8555-555555555555', '44444444-4444-4444-8444-444444444444', 'Vanilla Cream', NULL, 'Creamy vanilla', 15000, true, '2026-01-01', '2026-01-01');

INSERT INTO inventory (id, product_id, quantity, reserved_quantity, is_active, created_at, updated_at) VALUES
('66666666-6666-4666-8666-666666666666', '55555555-5555-4555-8555-555555555555', 12, 2, true, '2026-01-01', '2026-01-01');

INSERT INTO reviews (id, user_id, product_id, rating, comment, created_at, updated_at) VALUES
('77777777-7777-4777-8777-777777777777', '22222222-2222-4222-8222-222222222222', '55555555-5555-4555-8555-555555555555', 5, 'Good cake', '2026-01-01', '2026-01-01');

INSERT INTO discounts (id, name, discount_type, value, max_applied, max_redeemed, redeemed_count, description, start_date, end_date, created_at, updated_at) VALUES
('88888888-8888-4888-8888-888888888888', 'Cream Discount', 'FIXED', 1000, 10, 100, 0, 'A test discount', '2026-01-01', '2027-01-01', '2026-01-01', '2026-01-01');

INSERT INTO product_discounts (id, product_id, discount_id, is_active, created_at, updated_at) VALUES
('99999999-9999-4999-8999-999999999999', '55555555-5555-4555-8555-555555555555', '88888888-8888-4888-8888-888888888888', true, '2026-01-01', '2026-01-01');

INSERT INTO vouchers (id, code, name, description, type, "group", value, minimum_purchase, is_active, start_date, end_date, max_redeemed, redeemed_count, created_at, updated_at) VALUES
('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'CREAMTEST', 'Cream Voucher', 'A test voucher', 'FIXED', 'CHECKOUT', 2000, 10000, true, '2026-01-01', '2027-01-01', 100, 0, '2026-01-01', '2026-01-01');

INSERT INTO user_vouchers (id, user_id, voucher_id, status, claimed_at, expired_at, created_at, updated_at) VALUES
('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', '22222222-2222-4222-8222-222222222222', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'CLAIMED', '2026-01-01', '2027-01-01', '2026-01-01', '2026-01-01');
