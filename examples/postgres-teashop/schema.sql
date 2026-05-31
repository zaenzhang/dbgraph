CREATE TABLE customers (
  id BIGSERIAL PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE products (
  id BIGSERIAL PRIMARY KEY,
  sku TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  price_cents INTEGER NOT NULL CHECK (price_cents >= 0)
);

CREATE TABLE orders (
  id BIGSERIAL PRIMARY KEY,
  customer_id BIGINT NOT NULL REFERENCES customers(id),
  status TEXT NOT NULL DEFAULT 'pending',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE order_items (
  id BIGSERIAL PRIMARY KEY,
  order_id BIGINT NOT NULL REFERENCES orders(id),
  product_id BIGINT NOT NULL REFERENCES products(id),
  quantity INTEGER NOT NULL CHECK (quantity > 0)
);

CREATE TABLE payments (
  id BIGSERIAL PRIMARY KEY,
  order_id BIGINT NOT NULL REFERENCES orders(id),
  amount_cents INTEGER NOT NULL CHECK (amount_cents >= 0),
  provider_token TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'authorized'
);

CREATE INDEX idx_orders_customer_id ON orders(customer_id);
CREATE INDEX idx_order_items_order_id ON order_items(order_id);
CREATE INDEX idx_payments_order_id ON payments(order_id);

CREATE VIEW active_orders AS
SELECT o.id, o.customer_id, o.status, o.created_at
FROM orders o
WHERE o.status IN ('pending', 'paid');

COMMENT ON COLUMN customers.email IS 'customer email address';
COMMENT ON COLUMN payments.provider_token IS 'payment provider token';

INSERT INTO customers (email, display_name) VALUES
  ('alice@example.test', 'Alice'),
  ('bob@example.test', 'Bob');

INSERT INTO products (sku, name, price_cents) VALUES
  ('TEA-001', 'Green Tea', 1299),
  ('TEA-002', 'Black Tea', 1499);

INSERT INTO orders (customer_id, status, created_at) VALUES
  (1, 'pending', now() - interval '3 days'),
  (1, 'paid', now() - interval '2 days'),
  (2, 'cancelled', now() - interval '1 day');

INSERT INTO order_items (order_id, product_id, quantity) VALUES
  (1, 1, 2),
  (2, 2, 1),
  (3, 1, 1);

INSERT INTO payments (order_id, amount_cents, provider_token, status) VALUES
  (2, 1499, 'tok_live_sensitive_123', 'captured');
