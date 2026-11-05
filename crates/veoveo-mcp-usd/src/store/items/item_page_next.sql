-- A page of a tenant's items after a cursor's key. A separate statement for the first
-- page keeps this bound an index condition in a cached generic plan.
SELECT item_id AS "item_id: ItemId", name
FROM usd.item
WHERE tenant = $1 AND item_id > $2
ORDER BY item_id
LIMIT $3
