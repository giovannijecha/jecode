use super::*;
use crate::test_support::Directory;

#[test]
fn node_manager_homes_are_read_only_and_passed_by_name() {
    let project = Directory::new();
    let home = Directory::new();
    let local = home.path().join("Local");
    let volta = local.join("Volta");
    let nvm = home.path().join("nvm");
    for path in [&volta, &nvm] {
        std::fs::create_dir_all(path).unwrap();
    }
    let mut policy = Policy::for_command(Level::Workspace, project.path(), None, None)
        .unwrap()
        .unwrap();
    // A confined process sees its container as LOCALAPPDATA; Volta's default
    // home comes from the parent's value.
    policy
        .allow_node_tools_from(
            |name| match name {
                "LOCALAPPDATA" => Some(local.clone().into()),
                "NVM_HOME" => Some(nvm.clone().into()),
                "FNM_DIR" => Some(home.path().join("missing").into()),
                _ => None,
            },
            None,
        )
        .unwrap();
    for path in [&volta, &nvm] {
        assert!(policy.read.contains(&std::fs::canonicalize(path).unwrap()));
    }
    assert_eq!(policy.write.len(), 1);
    let mut command = Command::new("example");
    command.env("NVM_HOME", "explicit");
    apply(&mut command, Some(&policy)).unwrap();
    let variable = |name: &str| {
        command
            .get_envs()
            .find(|(key, _)| *key == name)
            .and_then(|(_, value)| value)
            .map(OsStr::to_os_string)
    };
    assert_eq!(variable("VOLTA_HOME"), Some(volta.clone().into_os_string()));
    assert_eq!(variable("NVM_HOME"), Some("explicit".into()));
    assert_eq!(variable("FNM_DIR"), None);
}

#[test]
fn a_node_prefix_is_readable_but_a_home_shaped_like_one_is_not() {
    let project = Directory::new();
    let home = Directory::new();
    let prefix = home.path().join("tools/node");
    for path in [prefix.join("bin"), prefix.join("lib/node_modules")] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::fs::write(prefix.join("bin/node"), "").unwrap();
    let grant = |bin: &Path| {
        let mut policy = Policy::for_command(Level::Workspace, project.path(), None, None)
            .unwrap()
            .unwrap();
        let (home, bin) = (home.path().to_path_buf(), bin.to_path_buf());
        policy
            .allow_node_tools_from(
                move |name| match name {
                    "HOME" => Some(home.clone().into()),
                    "PATH" => Some(bin.clone().into()),
                    _ => None,
                },
                None,
            )
            .unwrap();
        policy.read
    };
    let read = grant(&prefix.join("bin"));
    let prefix = std::fs::canonicalize(&prefix).unwrap();
    assert!(read.contains(&prefix.join("bin")));
    assert!(read.contains(&prefix.join("lib/node_modules")));
    // `~/.local` holds other programs' data beside `bin` and `lib`.
    assert!(!read.contains(&prefix));

    // `~/bin/node` beside `~/lib/node_modules` must not open the whole home.
    std::fs::create_dir_all(home.path().join("bin")).unwrap();
    std::fs::create_dir_all(home.path().join("lib/node_modules")).unwrap();
    std::fs::write(home.path().join("bin/node"), "").unwrap();
    let read = grant(&home.path().join("bin"));
    assert!(!read.contains(&std::fs::canonicalize(home.path()).unwrap()));
}

#[test]
fn empty_node_manager_homes_do_not_make_drives_enumerable() {
    let project = Directory::new();
    let home = Directory::new();
    let local = home.path().join("Local");
    let volta = local.join("Volta");
    let nvm = home.path().join("nvm");
    let fnm = home.path().join("fnm");
    let empty_path = home.path().join("bin");
    for path in [&volta, &nvm, &fnm, &empty_path] {
        std::fs::create_dir_all(path).unwrap();
    }
    let mut policy = Policy::for_command(Level::Workspace, project.path(), None, None)
        .unwrap()
        .unwrap();
    let variables = |name: &str| match name {
        "LOCALAPPDATA" => Some(local.clone().into()),
        "NVM_HOME" => Some(nvm.clone().into()),
        "FNM_DIR" => Some(fnm.clone().into()),
        "PATH" => Some(empty_path.clone().into()),
        _ => None,
    };
    policy.allow_node_tools_from(variables, None).unwrap();
    for path in [&volta, &nvm, &fnm] {
        assert!(policy.read.contains(&std::fs::canonicalize(path).unwrap()));
    }
    assert!(policy.enumerable_roots().is_empty());

    std::fs::write(empty_path.join("node.exe"), "").unwrap();
    let mut installed = Policy::for_command(Level::Workspace, project.path(), None, None)
        .unwrap()
        .unwrap();
    installed.allow_node_tools_from(variables, None).unwrap();
    for path in [&volta, &nvm, &fnm] {
        assert!(
            installed
                .node_paths
                .contains(&std::fs::canonicalize(path).unwrap())
        );
    }
    assert!(!installed.enumerable_roots().is_empty());
}

#[test]
fn installed_node_makes_only_its_drive_roots_enumerable() {
    let project = Directory::new();
    let home = Directory::new();
    let empty = home.path().join("bin");
    let nodejs = home.path().join("nodejs");
    for path in [&empty, &nodejs] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::fs::write(empty.join("node.cmd"), "").unwrap();
    let search = std::env::join_paths([&empty, &nodejs]).unwrap();
    let workspace = || {
        Policy::for_command(Level::Workspace, project.path(), None, None)
            .unwrap()
            .unwrap()
    };
    let roots = |policy: &Policy| {
        let encoded = crate::json::parse(&policy.encode()).unwrap();
        encoded
            .get("enumerate")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|path| PathBuf::from(path.as_str().unwrap()))
            .collect::<Vec<_>>()
    };

    // Without Node.js nothing becomes listable.
    let mut absent = workspace();
    absent
        .allow_node_tools_from(
            |name| (name == "PATH").then(|| empty.clone().into_os_string()),
            None,
        )
        .unwrap();
    assert!(roots(&absent).is_empty());

    // A runtime on PATH is detected but not granted; only drive roots list.
    std::fs::write(nodejs.join("node.exe"), "").unwrap();
    let mut installed = workspace();
    installed
        .allow_node_tools_from(|name| (name == "PATH").then(|| search.clone()), None)
        .unwrap();
    assert_eq!(installed.read.len(), 1);
    assert_eq!(installed.write.len(), 1);
    let listed = roots(&installed);
    assert!(!listed.is_empty());
    for root in &listed {
        assert!(root.has_root() && root.parent().is_none(), "{root:?}");
    }
    for path in [project.path(), nodejs.as_path()] {
        let root = std::fs::canonicalize(path).unwrap();
        let root = PathBuf::from(crate::permissions::plain(root.ancestors().last().unwrap()));
        assert!(listed.contains(&root), "{listed:?} lacks {root:?}");
    }
    for (index, root) in listed.iter().enumerate() {
        assert!(
            !listed[index + 1..]
                .iter()
                .any(|other| other.as_os_str().eq_ignore_ascii_case(root.as_os_str())),
            "{listed:?}"
        );
    }
}
