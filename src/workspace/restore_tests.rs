use super::*;
use crate::workspace_fixture::Fixture;
use std::{
    fs,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[test]
fn restoration_interruption_at_each_effect_boundary_is_repairable() {
    for after_publication in [false, true] {
        let files = Fixture::new();
        files.write("notes", "before\n");
        let store = crate::state::Store::in_home(&files.home()).unwrap();
        let recoveries = RecoveryStore::in_store(&store).unwrap();
        let ws = Workspace::open(&files.0)
            .unwrap()
            .with_access(super::super::Access::Local);
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(10),
        };
        let change = ws
            .prepare_edit("notes", "before", "after", &budget)
            .unwrap();
        let id = ws
            .apply(change, &budget, &recoveries, Some("s"), "edit")
            .unwrap()
            .recovery
            .unwrap();
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            recoveries
                .restore_with(
                    &ws,
                    &id,
                    &budget,
                    || {
                        if !after_publication {
                            panic!("restore interrupted after stash")
                        }
                    },
                    || {
                        if after_publication {
                            panic!("restore interrupted after publication")
                        }
                    },
                )
                .unwrap();
        }));
        assert!(interrupted.is_err());
        assert_eq!(recoveries.get(&id).unwrap().state, "restoring");
        let inspected = recoveries.inspect(&ws, &id, &budget).unwrap();
        assert_eq!(
            inspected.target,
            if after_publication {
                "retained original"
            } else {
                "absent"
            }
        );
        assert!(inspected.adjacent.starts_with("present"));
        recoveries.repair(&ws, &id, &budget).unwrap();
        assert_eq!(
            fs::read_to_string(files.0.join("notes")).unwrap(),
            "before\n"
        );
        assert_eq!(recoveries.get(&id).unwrap().state, "restored");
        assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
    }
}
