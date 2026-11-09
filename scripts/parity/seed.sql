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
('55555555-5555-4555-8555-555555555555', '44444444-4444-4444-8444-444444444444', 'Vanilla Cream', NULL, 'Creamy vanilla', 15000, true, '2026-01-01', '2026-01-01'),
-- Existing product without product_information makes the 404 deterministic.
('eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee', '44444444-4444-4444-8444-444444444444', 'Unlisted Cream', NULL, 'No optional information yet', 15000, false, '2026-01-01', '2026-01-01');

INSERT INTO product_information (id, product_id, highlights, serving_suggestions, product_details, why_choose, created_at, updated_at) VALUES
('cccccccc-cccc-4ccc-8ccc-cccccccccccc', '55555555-5555-4555-8555-555555555555', '["Fresh"]', '["Chill"]', '{"size":"1 L","storage":"Freezer","shelfLife":"One year"}', 'Because it is creamy', '2026-01-01', '2026-01-01');

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

-- PR3 commerce fixtures. Both databases are disposable and receive the same rows.
INSERT INTO carts (id, user_id, created_at, updated_at) VALUES
('12121212-1212-4212-8212-121212121212', '22222222-2222-4222-8222-222222222222', '2026-01-01', '2026-01-01');

INSERT INTO cart_items (id, cart_id, product_id, quantity, created_at, updated_at) VALUES
('13131313-1313-4313-8313-131313131313', '12121212-1212-4212-8212-121212121212', '55555555-5555-4555-8555-555555555555', 4, '2026-01-01', '2026-01-01');

INSERT INTO orders (id, user_id, address_id, voucher_amount, discount_amount, total_price, final_price, status, created_at, updated_at) VALUES
('14141414-1414-4414-8414-141414141414', '22222222-2222-4222-8222-222222222222', '33333333-3333-4333-8333-333333333333', 0, 0, 15000, 16000, 'PENDING', '2026-01-01', '2026-01-01');

INSERT INTO shipments (id, order_id, courier, service, cost, estimated, delivered_at, description, tracking_number, status, created_at, updated_at) VALUES
('15151515-1515-4515-8515-151515151515', '14141414-1414-4414-8414-141414141414', 'jne', 'REG', 1000, '2026-01-05', NULL, NULL, NULL, 'PENDING', '2026-01-01', '2026-01-01');

-- PR4 supporting features, shared by the Nest and Axum databases.
INSERT INTO product_analytics (id, user_id, product_id, created_at) VALUES
('16161616-1616-4616-8616-161616161616', '22222222-2222-4222-8222-222222222222', '55555555-5555-4555-8555-555555555555', '2026-01-01');

-- Extra active category and product fixtures exercise successful classic/related recommendations.
INSERT INTO category (id, name, description, is_active, created_at, updated_at) VALUES
('19191919-1919-4919-8919-191919191919', 'Classic Favorites', 'Classic product category', true, '2026-01-01', '2026-01-01');

INSERT INTO products (id, category_id, name, image_url, description, price, is_active, created_at, updated_at) VALUES
('20202020-2020-4020-8020-202020202020', '19191919-1919-4919-8919-191919191919', 'Classic Vanilla', NULL, 'Vanilla', 10000, true, '2026-01-01', '2026-01-01'),
('21212121-2121-4121-8121-212121212121', '19191919-1919-4919-8919-191919191919', 'Classic Strawberry', NULL, 'Strawberry', 20000, true, '2026-01-01', '2026-01-01');

INSERT INTO reviews (id, user_id, product_id, rating, comment, created_at, updated_at) VALUES
('22222222-3333-4333-8333-222222222222', '22222222-2222-4222-8222-222222222222', '20202020-2020-4020-8020-202020202020', 4, 'Classic vanilla', '2026-01-01', '2026-01-01'),
('22222222-4444-4444-8444-222222222222', '22222222-2222-4222-8222-222222222222', '21212121-2121-4121-8121-212121212121', 3, 'Classic strawberry', '2026-01-01', '2026-01-01');

INSERT INTO discounts (id, name, discount_type, value, max_applied, max_redeemed, redeemed_count, description, start_date, end_date, created_at, updated_at) VALUES
('23232323-2323-4323-8323-232323232323', 'Classic Percent', 'PERCENTAGE', 10, 10, 100, 0, 'Discount percent', '2026-01-01', '2027-01-01', '2026-01-01', '2026-01-01');

INSERT INTO product_discounts (id, product_id, discount_id, is_active, created_at, updated_at) VALUES
('24242424-2424-4424-8424-242424242424', '21212121-2121-4121-8121-212121212121', '23232323-2323-4323-8323-232323232323', true, '2026-01-01', '2026-01-01');

INSERT INTO notifications (id, title, message, created_at, updated_at) VALUES
('17171717-1717-4717-8717-171717171717', 'Creamy Update', 'New cream flavors are available', '2026-01-01', '2026-01-01');

INSERT INTO user_notifications (id, user_id, notification_id, is_read, created_at, updated_at) VALUES
('18181818-1818-4818-8818-181818181818', '22222222-2222-4222-8222-222222222222', '17171717-1717-4717-8717-171717171717', false, '2026-01-01', '2026-01-01');
