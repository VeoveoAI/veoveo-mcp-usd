-- Record one item of a tenant.
INSERT INTO usd.item (tenant, item_id, name)
VALUES ($1, $2, $3)
RETURNING item_id AS "item_id: ItemId", name
