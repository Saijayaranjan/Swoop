use osprey_domain::device::{AuditEntry, Device, Scope};
use osprey_domain::*;
use osprey_store::{Store, StoreError};

fn device(name: &str) -> Device {
    Device {
        id: DeviceId::new(),
        name: name.into(),
        kind: "phone".into(),
        scopes: vec![Scope::Read, Scope::Add],
        created_at: Millis(10),
        last_seen_at: None,
        last_ip: None,
        expires_at: Some(Millis(1_000_000)),
        revoked: false,
    }
}

#[tokio::test]
async fn devices_and_token_lookup() {
    let store = Store::open_in_memory().unwrap();
    let mut a = device("iPhone");
    let b = device("Tablet");
    store
        .upsert_device(&a, Some("hash-a".into()))
        .await
        .unwrap();
    store
        .upsert_device(&b, Some("hash-b".into()))
        .await
        .unwrap();
    assert_eq!(
        store.list_devices().await.unwrap(),
        vec![a.clone(), b.clone()]
    );

    assert_eq!(
        store.find_device_by_token_hash("hash-a").await.unwrap(),
        Some(a.clone())
    );
    assert!(store
        .find_device_by_token_hash("nope")
        .await
        .unwrap()
        .is_none());

    // a duplicate token hash is refused by the unique index
    let c = device("Clone");
    assert!(matches!(
        store.upsert_device(&c, Some("hash-a".into())).await,
        Err(StoreError::Sqlite(_))
    ));

    // rename without a token keeps the hash
    a.name = "My iPhone".into();
    a.scopes = vec![Scope::Admin];
    store.upsert_device(&a, None).await.unwrap();
    let found = store
        .find_device_by_token_hash("hash-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.name, "My iPhone");
    assert_eq!(found.scopes, vec![Scope::Admin]);

    // token rotation
    store
        .upsert_device(&a, Some("hash-a2".into()))
        .await
        .unwrap();
    assert!(store
        .find_device_by_token_hash("hash-a")
        .await
        .unwrap()
        .is_none());
    assert!(store
        .find_device_by_token_hash("hash-a2")
        .await
        .unwrap()
        .is_some());

    store
        .touch_device(&a.id, "10.0.0.5", Millis(777))
        .await
        .unwrap();
    let touched = store
        .find_device_by_token_hash("hash-a2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(touched.last_seen_at, Some(Millis(777)));
    assert_eq!(touched.last_ip.as_deref(), Some("10.0.0.5"));

    // revoked devices are still returned so the caller can audit
    a.revoked = true;
    store.upsert_device(&a, None).await.unwrap();
    assert!(
        store
            .find_device_by_token_hash("hash-a2")
            .await
            .unwrap()
            .unwrap()
            .revoked
    );

    assert!(store.delete_device(&a.id).await.unwrap());
    assert!(!store.delete_device(&a.id).await.unwrap());
    assert!(store
        .find_device_by_token_hash("hash-a2")
        .await
        .unwrap()
        .is_none());
    assert_eq!(store.list_devices().await.unwrap(), vec![b]);
}

#[tokio::test]
async fn audit_log_is_capped() {
    let store = Store::open_in_memory().unwrap();
    let dev = DeviceId::new();
    for i in 0..5_010i64 {
        store
            .append_audit(&AuditEntry {
                at: Millis(i),
                device_id: if i % 2 == 0 { Some(dev.clone()) } else { None },
                ip: "127.0.0.1".into(),
                action: "auth".into(),
                target: Some(format!("t{i}")),
                success: i % 7 != 0,
                detail: None,
            })
            .await
            .unwrap();
    }
    let all = store.audit_log(0).await.unwrap();
    assert_eq!(all.len(), 5_000);
    assert_eq!(all[0].at, Millis(5_009));
    assert_eq!(all.last().unwrap().at, Millis(10));
    let top = store.audit_log(2).await.unwrap();
    assert_eq!(top.len(), 2);
    assert_eq!(top[1].target.as_deref(), Some("t5008"));
    assert_eq!(top[1].device_id, Some(dev));
}
