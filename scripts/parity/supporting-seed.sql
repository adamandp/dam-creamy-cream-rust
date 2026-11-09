-- Additive PR4 fixtures applied only after PR1–PR3 differential tests.
-- Their presence must not alter the original suite's default ordering/pagination.
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

-- An unambiguous sales winner makes the category-preview selection deterministic.
INSERT INTO order_items (id, order_id, product_id, discount_id, discount_amount, price, "finalPrice", quantity, created_at, updated_at) VALUES
('25252525-2525-4525-8525-252525252525', '14141414-1414-4414-8414-141414141414', '21212121-2121-4121-8121-212121212121', NULL, 0, 20000, 20000, 1, '2026-01-01', '2026-01-01');
