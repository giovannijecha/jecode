use super::*;
use crate::workspace_fixture as support;
use std::{
    fs,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[test]
fn cancelled_publication_restores_original_and_destination_race_retains_both() {
    for collision in [false, true] {
        let files = support::Fixture::new();
        files.write("notes", "original\n");
        let ws = Workspace::open(&files.0).unwrap();
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(5),
        };
        let change = ws
            .prepare_edit("notes", "original", "approved", &budget)
            .unwrap();
        let result = ws.apply_with(change, &budget, || {
            if collision {
                files.write("notes", "concurrent\n");
            } else {
                cancelled.store(true, Ordering::Release);
            }
        });
        let error = result.err().unwrap().to_string();
        if files.unsupported_host_filesystem(&error) {
            assert_eq!(
                fs::read_to_string(files.0.join("notes")).unwrap(),
                "original\n"
            );
            assert_eq!(fs::read_dir(&files.0).unwrap().count(), 1);
            continue;
        }
        assert_eq!(
            fs::read_to_string(files.0.join("notes")).unwrap(),
            if collision {
                "concurrent\n"
            } else {
                "original\n"
            }
        );
        let retained: Vec<_> = fs::read_dir(&files.0)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".jecode-")
            })
            .collect();
        if collision {
            assert_eq!(retained.len(), 1);
            assert_eq!(fs::read_to_string(&retained[0]).unwrap(), "original\n");
            assert!(error.contains("original retained at"));
        } else {
            assert!(retained.is_empty());
            assert!(error.contains("restored"));
        }
    }
}
