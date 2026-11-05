-- The server's tables live in its own schema. Every row belongs to one tenant, every
-- table has a primary key, and its comment states its retention.
CREATE TABLE usd.item (
    tenant uuid NOT NULL,
    item_id uuid NOT NULL,
    name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 128),
    PRIMARY KEY (tenant, item_id)
);

COMMENT ON TABLE usd.item IS
    'Retention: for the life of the tenant. The server deletes no item.';
