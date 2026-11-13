//! Skill save acknowledgements represent persistence, not a catalogue refresh.
use sovra_core::skills::{
    Skill, SkillCatalog, SkillCommand, SkillError, SkillEvent, SkillListing, SkillStore,
};
use sovra_core::{AgentConfig, Command, Event};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Default)]
struct Store {
    fail: AtomicBool,
    skills: Mutex<Vec<Skill>>,
}

impl SkillStore for Store {
    fn list(&self) -> Result<SkillListing, SkillError> {
        Ok(SkillListing {
            skills: self.skills.lock().expect("test skill store lock").clone(),
            issues: vec![],
        })
    }
    fn save(&self, skill: &Skill) -> Result<(), SkillError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(SkillError("Test storage unavailable".into()));
        }
        *self.skills.lock().expect("test skill store lock") = vec![skill.clone()];
        Ok(())
    }
    fn import(&self, _: &std::path::Path) -> Result<Skill, SkillError> {
        Err(SkillError("Unused test operation".into()))
    }
    fn remove(&self, _: &str) -> Result<(), SkillError> {
        Err(SkillError("Unused test operation".into()))
    }
    fn resource(&self, _: &str, _: &str) -> Result<String, SkillError> {
        Err(SkillError("Unused test operation".into()))
    }
}

async fn next_skill(events: &async_channel::Receiver<Event>) -> SkillEvent {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Event::Skills(event) = events
                .recv()
                .await
                .expect("runtime event stream remains open")
            {
                return event;
            }
        }
    })
    .await
    .expect("skill response before deadline")
}

#[tokio::test]
async fn skill_save_acknowledges_persistence_and_refresh_does_not() {
    let store = Arc::new(Store::default());
    let client = sovra_runtime::start_with_services(
        "skills-test",
        AgentConfig::default(),
        sovra_runtime::RuntimeServices {
            skills: Some(store.clone()),
            ..sovra_runtime::RuntimeServices::default()
        },
    )
    .unwrap();
    let events = client.events();
    assert!(matches!(
        next_skill(&events).await,
        SkillEvent::Loaded { .. }
    ));
    let mut skill = SkillCatalog::default().find("read-files").unwrap().clone();
    skill.name = "garden-plan".into();
    skill.title = "Garden plan".into();
    skill.builtin = false;
    skill.aliases.clear();
    store.fail.store(true, Ordering::SeqCst);
    client
        .send(Command::Skills(SkillCommand::Save(Box::new(skill.clone()))))
        .unwrap();
    assert!(matches!(next_skill(&events).await, SkillEvent::Error(_)));
    assert_eq!(store.list().unwrap().skills, Vec::<Skill>::new());
    store.fail.store(false, Ordering::SeqCst);
    client
        .send(Command::Skills(SkillCommand::Save(Box::new(skill.clone()))))
        .unwrap();
    let SkillEvent::Loaded { skills, .. } = next_skill(&events).await else {
        panic!("catalogue precedes acknowledgement")
    };
    assert!(skills.contains(&skill));
    assert_eq!(
        next_skill(&events).await,
        SkillEvent::Saved(Box::new(skill.clone()))
    );
    assert_eq!(store.list().unwrap().skills, vec![skill]);
    client.send(Command::Skills(SkillCommand::Refresh)).unwrap();
    assert!(matches!(
        next_skill(&events).await,
        SkillEvent::Loaded { .. }
    ));
    client
        .send(Command::Skills(SkillCommand::Remove {
            name: "unused".into(),
        }))
        .unwrap();
    assert!(
        matches!(next_skill(&events).await, SkillEvent::Error(_)),
        "refresh must not emit a save acknowledgement"
    );
    client.shutdown_blocking().unwrap();
}
