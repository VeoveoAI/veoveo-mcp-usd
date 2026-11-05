-- The first page of a tenant's items in key order, with one row of lookahead.
SELECT item_id AS "item_id: ItemId", name
FROM usd.item
WHERE tenant = $1
ORDER BY item_id
LIMIT $2
