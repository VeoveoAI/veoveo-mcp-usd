//! A tenant's items in the `usd` schema. Each statement is a `.sql` file under
//! `items/`, run through sqlx's checked `query_file*!` macros. The listing pages by key
//! with core's `PageSize` and a `KeysetCursor` bound to the tenant, and it has a
//! first-page and a next-page statement so the key bound stays an index range.
use crate::contract::{ItemId, ItemName, ItemView, Items, UsdResource};
use veoveo_hub_sdk::definition::CollectionRequest;
use veoveo_store::{PlatformStore, TenantId};
use veoveo_types::{LiveKeyset, Page, Pager, PagingError};

/// One item of a tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub item_id: ItemId,
    pub name: ItemName,
}

/// One item as the database returns it, before its name is admitted again.
struct ItemRow {
    item_id: ItemId,
    name: String,
}

impl TryFrom<ItemRow> for Item {
    type Error = ItemsError;
    fn try_from(row: ItemRow) -> Result<Self, Self::Error> {
        Ok(Self {
            item_id: row.item_id,
            name: ItemName::parse(row.name).map_err(|_| ItemsError::Corrupt("item name"))?,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ItemsError {
    #[error("the cursor belongs to another listing or tenant")]
    Paging(#[from] PagingError),
    #[error("a stored {0} fails admission")]
    Corrupt(&'static str),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// The stored identity of a tenant, as the identity schema derives it.
pub fn stored_tenant(
    tenant: &veoveo_types::TenantId,
) -> Result<TenantId, veoveo_store::StoreError> {
    veoveo_store::deterministic_tenant_id(tenant.as_str())
}

/// The server's items in its own schema.
#[derive(Clone)]
pub struct ItemStore {
    store: PlatformStore,
}

impl ItemStore {
    pub fn new(store: PlatformStore) -> Self {
        Self { store }
    }

    /// Record an item for `tenant`.
    pub async fn create(&self, tenant: TenantId, name: &ItemName) -> Result<Item, ItemsError> {
        sqlx::query_file_as!(
            ItemRow,
            "src/store/items/insert_item.sql",
            tenant as TenantId,
            ItemId::new() as ItemId,
            name.as_str(),
        )
        .fetch_one(self.store.pool())
        .await?
        .try_into()
    }

    /// `tenant`'s item `item_id`, or `None` when the tenant has no such item.
    pub async fn item(
        &self,
        tenant: TenantId,
        item_id: ItemId,
    ) -> Result<Option<Item>, ItemsError> {
        sqlx::query_file_as!(
            ItemRow,
            "src/store/items/item.sql",
            tenant as TenantId,
            item_id as ItemId,
        )
        .fetch_optional(self.store.pool())
        .await?
        .map(Item::try_from)
        .transpose()
    }

    /// One page of `tenant`'s items in key order, after `cursor` when the caller returns
    /// the previous page's. A cursor that another tenant's listing issued is refused.
    pub async fn page(
        &self,
        tenant: TenantId,
        request: &CollectionRequest<Items>,
    ) -> Result<Page<Items>, ItemsError> {
        let pager = Pager::<Items>::new(
            tenant,
            *request.filters(),
            request.limit(),
            LiveKeyset,
            request.cursor(),
        )?;
        let rows = match pager.after() {
            None => {
                sqlx::query_file_as!(
                    ItemRow,
                    "src/store/items/item_page_first.sql",
                    tenant as TenantId,
                    pager.fetch_limit()
                )
                .fetch_all(self.store.pool())
                .await?
            }
            Some(after) => {
                sqlx::query_file_as!(
                    ItemRow,
                    "src/store/items/item_page_next.sql",
                    tenant as TenantId,
                    *after as ItemId,
                    pager.fetch_limit()
                )
                .fetch_all(self.store.pool())
                .await?
            }
        };
        let items = rows
            .into_iter()
            .map(|row| Item::try_from(row).map(ItemView::from))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(pager.finish(items)?)
    }
}

impl From<Item> for ItemView {
    fn from(item: Item) -> Self {
        Self {
            item_uri: UsdResource::Item(item.item_id),
            item_id: item.item_id,
            name: item.name,
        }
    }
}
