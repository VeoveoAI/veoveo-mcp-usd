//! The item store against a fresh PostgreSQL database migrated as the server's own
//! login. Requires the PostgreSQL 18 server programs on `PATH`.
use std::time::Duration;

use veoveo_fixtures::store::TestDb;
use veoveo_hub_sdk::definition::{CollectionRequest, NoFilters};
use veoveo_mcp_usd::{
    contract::{ItemId, ItemName, ItemView, Items},
    migrator, server,
    store::{ItemStore, ItemsError},
};
use veoveo_store::deterministic_tenant_id;
use veoveo_types::{PageSize, PagingError};

#[tokio::test]
async fn items_page_in_key_order_and_cursors_stay_with_their_tenant() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let db = TestDb::new().await;
        db.migrate_server(&server(), migrator()).await;
        let items = ItemStore::new(db.server_store(&server()).await);
        let tenant = deterministic_tenant_id("acme").unwrap();
        let other = deterministic_tenant_id("globex").unwrap();
        let mut created = Vec::new();
        for name in ["first", "second", "third"] {
            let name = ItemName::parse(name).unwrap();
            created.push(items.create(tenant, &name).await.unwrap());
        }
        let elsewhere = items
            .create(other, &ItemName::parse("elsewhere").unwrap())
            .await
            .unwrap();

        // Three items page as two and one, and the last page has no cursor.
        let selection = CollectionRequest::<Items>::new(NoFilters {}, PageSize::new(2).unwrap());
        let first = items.page(tenant, &selection).await.unwrap();
        let expected: Vec<ItemView> = created.iter().cloned().map(Into::into).collect();
        assert_eq!(first.items(), &expected[..2]);
        let next = first.next_cursor().expect("a third item follows").clone();
        let second = items
            .page(tenant, &selection.continuing(next.clone()))
            .await
            .unwrap();
        assert_eq!(second.items(), &expected[2..]);
        assert!(second.next_cursor().is_none());

        // Tenant and effective limit are contextual admission, not cursor authority.
        let continued = CollectionRequest::<Items>::new(NoFilters {}, PageSize::new(2).unwrap())
            .continuing(next.clone());
        assert!(matches!(
            items.page(other, &continued).await,
            Err(ItemsError::Paging(PagingError::Cursor(_)))
        ));
        let changed = CollectionRequest::<Items>::new(NoFilters {}, PageSize::new(1).unwrap())
            .continuing(next);
        assert!(matches!(
            items.page(tenant, &changed).await,
            Err(ItemsError::Paging(PagingError::Cursor(_)))
        ));

        // An exact read sees only the tenant's own item.
        assert_eq!(
            items.item(tenant, created[0].item_id).await.unwrap(),
            Some(created[0].clone())
        );
        assert_eq!(items.item(tenant, elsewhere.item_id).await.unwrap(), None);
        assert_eq!(items.item(tenant, ItemId::new()).await.unwrap(), None);
    })
    .await
    .expect("the item store test exceeded 60 seconds");
}
