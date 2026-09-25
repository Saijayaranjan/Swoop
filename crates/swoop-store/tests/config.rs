mod common;

use std::path::PathBuf;
use swoop_domain::automation::*;
use swoop_domain::category::Category;
use swoop_domain::queue::{BandwidthProfile, Queue, QueueCompletionAction};
use swoop_domain::rules::{MatchMode, Rule, RuleAction, RuleCondition};
use swoop_domain::schedule::{Condition, Recurrence, Schedule, ScheduleAction};
use swoop_domain::settings::{Settings, SETTINGS_SCHEMA_VERSION};
use swoop_domain::*;
use swoop_store::{RecipeRecord, Store, StoreError};

#[tokio::test]
async fn queues_round_trip() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.list_queues().await.unwrap().is_empty());
    let mut qs = Queue::builtin_defaults();
    qs[1].bandwidth = BandwidthProfile {
        download_limit: 1_000,
        upload_limit: 2,
        connections_per_task: 3,
    };
    qs[1].completion_action = QueueCompletionAction::RunAutomation {
        automation_id: AutomationId("a1".into()),
    };
    qs[1].directory = Some(PathBuf::from("/x"));
    qs[1].schedule_id = Some(ScheduleId("s".into()));
    for q in &qs {
        store.upsert_queue(q).await.unwrap();
    }
    assert_eq!(store.list_queues().await.unwrap(), qs);
    qs[0].name = "Renamed".into();
    qs[0].position = 99;
    store.upsert_queue(&qs[0]).await.unwrap();
    let listed = store.list_queues().await.unwrap();
    assert_eq!(listed.last().unwrap().name, "Renamed");
    assert!(store.delete_queue(&qs[0].id).await.unwrap());
    assert!(!store.delete_queue(&qs[0].id).await.unwrap());
    assert_eq!(store.list_queues().await.unwrap().len(), 4);
}

#[tokio::test]
async fn categories_round_trip() {
    let store = Store::open_in_memory().unwrap();
    let cats = Category::builtin_defaults();
    for c in cats.iter().rev() {
        store.upsert_category(c).await.unwrap();
    }
    assert_eq!(store.list_categories().await.unwrap(), cats);
    assert!(store.delete_category(&cats[0].id).await.unwrap());
    assert_eq!(store.list_categories().await.unwrap().len(), cats.len() - 1);
}

#[tokio::test]
async fn rules_round_trip_in_priority_order() {
    let store = Store::open_in_memory().unwrap();
    let mut r1 = Rule::new("pdfs");
    r1.priority = 50;
    r1.match_mode = MatchMode::Any;
    r1.conditions = vec![
        RuleCondition::Extension {
            any_of: vec!["pdf".into()],
        },
        RuleCondition::Regex {
            pattern: "^https://".into(),
        },
        RuleCondition::SizeGreaterThan { bytes: 10 },
    ];
    r1.actions = vec![
        RuleAction::SaveTo {
            directory: PathBuf::from("/docs"),
        },
        RuleAction::SetPriority {
            priority: Priority::High,
        },
        RuleAction::StopProcessing,
    ];
    r1.hit_count = 3;
    let mut r2 = Rule::new("later");
    r2.priority = 100;
    r2.enabled = false;
    store.upsert_rule(&r2).await.unwrap();
    store.upsert_rule(&r1).await.unwrap();
    assert_eq!(
        store.list_rules().await.unwrap(),
        vec![r1.clone(), r2.clone()]
    );
    assert!(store.delete_rule(&r1.id).await.unwrap());
    assert_eq!(store.list_rules().await.unwrap(), vec![r2]);
}

#[tokio::test]
async fn schedules_round_trip() {
    let store = Store::open_in_memory().unwrap();
    let mut s = Schedule::new(
        "night",
        Recurrence::Weekly {
            days: vec![0, 4],
            start: "23:00".into(),
            end: "06:00".into(),
        },
    );
    s.conditions = vec![
        Condition::OnAcPower,
        Condition::BatteryAbove { percent: 30 },
    ];
    s.on_start = vec![ScheduleAction::SetSpeedLimit {
        download: 1,
        upload: 2,
    }];
    s.on_end = vec![ScheduleAction::Notify {
        message: "done".into(),
    }];
    s.gate_attached = false;
    s.last_fired_at = Some(Millis(5));
    let a = Schedule::new("always", Recurrence::Always);
    store.upsert_schedule(&s).await.unwrap();
    store.upsert_schedule(&a).await.unwrap();
    assert_eq!(
        store.list_schedules().await.unwrap(),
        vec![a.clone(), s.clone()]
    );
    assert!(store.delete_schedule(&a.id).await.unwrap());
    assert_eq!(store.list_schedules().await.unwrap(), vec![s]);
}

fn automation() -> AutomationRule {
    let mut a = AutomationRule::new("post");
    a.events = vec![
        AutomationEvent::DownloadCompleted,
        AutomationEvent::TorrentFinished,
    ];
    a.conditions = vec![RuleCondition::Kind {
        equals: "http".into(),
    }];
    a.actions = vec![
        AutomationAction::Notify {
            title: "t".into(),
            body: "b".into(),
        },
        AutomationAction::RunShell {
            script: "echo hi".into(),
        },
        AutomationAction::RunCommand {
            program: "/bin/ls".into(),
            args: vec!["-l".into()],
        },
    ];
    a.run_count = 2;
    a.last_run_at = Some(Millis(9));
    a.last_error = Some("e".into());
    a
}

#[tokio::test]
async fn automations_consents_and_runs() {
    let store = Store::open_in_memory().unwrap();
    let a = automation();
    store.upsert_automation(&a).await.unwrap();
    assert_eq!(store.list_automations().await.unwrap(), vec![a.clone()]);

    // consents: replace-all semantics
    let consent = |idx: u32, hash: &str| ConsentRecord {
        automation_id: a.id.clone(),
        action_index: idx,
        hash: hash.into(),
        granted_at: Millis(1),
    };
    store
        .grant_consents(&a.id, vec![consent(1, "h1"), consent(2, "h2")])
        .await
        .unwrap();
    assert_eq!(
        store.consents_for(&a.id).await.unwrap(),
        vec![consent(1, "h1"), consent(2, "h2")]
    );
    store
        .grant_consents(&a.id, vec![consent(2, "h2-new")])
        .await
        .unwrap();
    assert_eq!(
        store.consents_for(&a.id).await.unwrap(),
        vec![consent(2, "h2-new")]
    );
    // consent for another automation is rejected
    let other = ConsentRecord {
        automation_id: AutomationId::new(),
        ..consent(0, "x")
    };
    assert!(store.grant_consents(&a.id, vec![other]).await.is_err());
    // unknown automation violates the foreign key
    let ghost = AutomationId::new();
    let err = store
        .grant_consents(
            &ghost,
            vec![ConsentRecord {
                automation_id: ghost.clone(),
                ..consent(0, "x")
            }],
        )
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Sqlite(_)));
    assert_eq!(store.revoke_consents(&a.id).await.unwrap(), 1);
    assert!(store.consents_for(&a.id).await.unwrap().is_empty());

    // runs: newest first, per-automation filter, capped at 1000
    let b = AutomationRule::new("other");
    store.upsert_automation(&b).await.unwrap();
    for i in 0..1010i64 {
        let run = AutomationRun {
            automation_id: if i % 2 == 0 {
                a.id.clone()
            } else {
                b.id.clone()
            },
            task_id: if i % 3 == 0 {
                None
            } else {
                Some(TaskId::new())
            },
            event: AutomationEvent::DownloadCompleted,
            at: Millis(i),
            success: i % 5 != 0,
            message: format!("run {i}"),
        };
        store.append_automation_run(&run).await.unwrap();
    }
    let all = store.automation_runs(None, 0).await.unwrap();
    assert_eq!(all.len(), 1000);
    assert_eq!(all[0].at, Millis(1009));
    assert_eq!(all.last().unwrap().at, Millis(10));
    let only_a = store.automation_runs(Some(&a.id), 3).await.unwrap();
    assert_eq!(
        only_a.iter().map(|r| r.at.0).collect::<Vec<_>>(),
        [1008, 1006, 1004]
    );
    assert!(only_a.iter().all(|r| r.automation_id == a.id));
    assert_eq!(only_a[0].event, AutomationEvent::DownloadCompleted);

    // deleting the automation cascades its consents
    store
        .grant_consents(&a.id, vec![consent(1, "h")])
        .await
        .unwrap();
    assert!(store.delete_automation(&a.id).await.unwrap());
    assert!(store.consents_for(&a.id).await.unwrap().is_empty());
    assert_eq!(store.list_automations().await.unwrap(), vec![b]);
}

#[tokio::test]
async fn recipes_round_trip() {
    let store = Store::open_in_memory().unwrap();
    let r = RecipeRecord {
        id: RecipeId::new(),
        name: "PDF to Documents".into(),
        json: serde_json::json!({"name": "PDF to Documents", "tags": ["a"], "directory": "/docs"}),
        created_at: Millis(1),
        updated_at: Millis(2),
    };
    store.upsert_recipe(&r).await.unwrap();
    assert_eq!(store.list_recipes().await.unwrap(), vec![r.clone()]);
    let mut r2 = r.clone();
    r2.name = "Changed".into();
    r2.updated_at = Millis(3);
    store.upsert_recipe(&r2).await.unwrap();
    assert_eq!(store.list_recipes().await.unwrap(), vec![r2.clone()]);
    assert!(store.delete_recipe(&r2.id).await.unwrap());
    assert!(store.list_recipes().await.unwrap().is_empty());
}

#[tokio::test]
async fn settings_round_trip_and_forward_compat() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.load_settings().await.unwrap().is_none());
    let mut s = Settings::default();
    s.network.connections_per_task = 12;
    s.storage.download_directory = PathBuf::from("/Volumes/Data");
    s.plugins
        .insert("p".into(), serde_json::json!({"enabled": true}));
    store.save_settings(&s).await.unwrap();
    assert_eq!(store.load_settings().await.unwrap(), Some(s.clone()));

    // An older, partial document (schema_version 0, missing sections) still loads with defaults.
    let mut old = Settings {
        schema_version: 0,
        ..Default::default()
    };
    old.remote.port = 5555;
    store.save_settings(&old).await.unwrap();
    let mut partial = serde_json::to_value(&old).unwrap();
    partial.as_object_mut().unwrap().remove("torrent");
    partial.as_object_mut().unwrap().remove("appearance");
    // write the trimmed JSON through the public API by round-tripping it as Settings
    let trimmed: Settings = serde_json::from_value(partial).unwrap();
    store.save_settings(&trimmed).await.unwrap();
    let loaded = store.load_settings().await.unwrap().unwrap();
    assert_eq!(loaded.remote.port, 5555);
    assert_eq!(loaded.torrent, Settings::default().torrent);
    assert_eq!(loaded.schema_version, 0);
    assert!(loaded.schema_version < SETTINGS_SCHEMA_VERSION);
}

#[tokio::test]
async fn credentials_grabber_speed_and_meta() {
    let store = Store::open_in_memory().unwrap();
    let id = CredentialId::new();
    store
        .upsert_credential_meta(&id, "NAS", Some("bob"))
        .await
        .unwrap();
    store
        .upsert_credential_meta(&CredentialId::new(), "Alpha", None)
        .await
        .unwrap();
    let metas = store.list_credential_meta().await.unwrap();
    assert_eq!(metas.len(), 2);
    assert_eq!(metas[0].name, "Alpha");
    assert_eq!(metas[1].username.as_deref(), Some("bob"));
    store
        .upsert_credential_meta(&id, "NAS2", None)
        .await
        .unwrap();
    let metas = store.list_credential_meta().await.unwrap();
    assert_eq!(metas[1].name, "NAS2");
    assert_eq!(metas[1].username, None);
    assert!(store.delete_credential_meta(&id).await.unwrap());
    assert_eq!(store.list_credential_meta().await.unwrap().len(), 1);

    let doc = serde_json::json!({"id": "g1", "files": [{"url": "https://x/y"}], "done": false});
    store.save_grabber_session("g1", &doc).await.unwrap();
    store
        .save_grabber_session("g2", &serde_json::json!({"id": "g2"}))
        .await
        .unwrap();
    let sessions = store.load_grabber_sessions().await.unwrap();
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().any(|(id, j)| id == "g1" && *j == doc));
    assert!(store.delete_grabber_session("g1").await.unwrap());
    assert!(!store.delete_grabber_session("g1").await.unwrap());

    let now = Millis::now();
    let day = 24 * 3600 * 1000;
    store
        .append_speed_sample(Millis(now.0 - day - 60_000), 1, 1)
        .await
        .unwrap();
    store
        .append_speed_sample(Millis(now.0 - 20_000), 100, 10)
        .await
        .unwrap();
    store
        .append_speed_sample(Millis(now.0 - 10_000), 200, 20)
        .await
        .unwrap();
    store.append_speed_sample(now, 300, 30).await.unwrap();
    store.append_speed_sample(now, 301, 31).await.unwrap(); // same instant: replaced
    store.flush().await.unwrap();
    let samples = store.speed_samples(Millis(0)).await.unwrap();
    assert_eq!(samples.len(), 3, "old sample pruned: {samples:?}");
    assert_eq!(samples[0].download, 100);
    assert_eq!(samples[2].download, 301);
    let recent = store.speed_samples(Millis(now.0 - 15_000)).await.unwrap();
    assert_eq!(recent.len(), 2);

    assert_eq!(store.meta_get("k").await.unwrap(), None);
    store.meta_set("k", "v1").await.unwrap();
    store.meta_set("k", "v2").await.unwrap();
    assert_eq!(store.meta_get("k").await.unwrap().as_deref(), Some("v2"));
    assert_eq!(
        store.schema_version().await.unwrap(),
        swoop_store::CURRENT_SCHEMA_VERSION
    );
}
