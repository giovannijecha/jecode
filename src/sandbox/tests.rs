use super::*;
use crate::permissions::Kind;
use crate::test_support::Directory;

#[test]
fn workspace_policy_names_only_root_session_scratch_and_approved_cwd() {
    let project = Directory::new();
    let home = Directory::new();
    let scratch = home.path().join("scratch");
    let outside = home.path().join("approved");
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let access = Access {
        kind: Kind::Runs,
        target: "program".into(),
        place: Some(outside.to_string_lossy().into_owned()),
        outside: true,
        base: None,
        identity: None,
        members: Vec::new(),
    };
    let policy = Policy::for_command(
        Level::Workspace,
        project.path(),
        Some(&scratch),
        Some(&access),
    )
    .unwrap()
    .unwrap();
    let encoded = crate::json::parse(&policy.encode()).unwrap();
    let paths = |key| {
        encoded
            .get(key)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|path| path.as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let expected = [project.path(), scratch.as_path(), outside.as_path()].map(|path| {
        std::fs::canonicalize(path)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    });
    assert_eq!(paths("read"), expected);
    assert_eq!(paths("write"), expected);
    assert!(paths("deny").is_empty());
    assert!(paths("enumerate").is_empty());
    assert_eq!(encoded.get("network"), Some(&Value::Bool(true)));
}

#[test]
fn actual_session_home_inside_workspace_fails_closed() {
    let project = Directory::new();
    let storage = project.path().join(".jecode");
    let scratch = storage.join("tmp/current");
    std::fs::create_dir_all(&scratch).unwrap();
    let policy = Policy::for_command(Level::Workspace, project.path(), Some(&scratch), None)
        .unwrap()
        .unwrap();
    assert!(policy.protect_session_home(None).is_ok());
    assert!(
        policy
            .protect_session_home(Some(&storage))
            .unwrap_err()
            .contains("other sessions")
    );
    let other_home = Directory::new();
    assert!(policy.protect_session_home(Some(other_home.path())).is_ok());
}

#[test]
fn rust_runtime_roots_are_read_only_and_cargo_cache_uses_session_scratch() {
    let project = Directory::new();
    let home = Directory::new();
    let scratch = home.path().join("scratch");
    let rustup = home.path().join(".rustup");
    let cargo_bin = home.path().join(".cargo/bin");
    for path in [&scratch, &rustup, &cargo_bin] {
        std::fs::create_dir_all(path).unwrap();
    }
    let mut policy = Policy::for_command(Level::Workspace, project.path(), Some(&scratch), None)
        .unwrap()
        .unwrap();
    policy.allow_existing_read_directory(&rustup, None).unwrap();
    policy
        .allow_existing_read_directory(&cargo_bin, None)
        .unwrap();
    policy
        .allow_existing_read_directory(&home.path().join("missing"), None)
        .unwrap();
    let read = policy
        .read
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        read.contains(
            &std::fs::canonicalize(&rustup)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        )
    );
    assert!(
        read.contains(
            &std::fs::canonicalize(&cargo_bin)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        )
    );
    assert_eq!(policy.read.len(), 4);
    assert_eq!(policy.write.len(), 2);
    assert!(
        policy
            .allow_existing_read_directory(home.path(), Some(home.path()))
            .is_err()
    );

    let expected = std::fs::canonicalize(&scratch).unwrap().join("cargo");
    let mut command = Command::new("example");
    let _ = apply(&mut command, Some(&policy));
    let cargo_home = command
        .get_envs()
        .find(|(key, _)| *key == "CARGO_HOME")
        .unwrap()
        .1
        .unwrap();
    assert_eq!(
        cargo_home,
        OsStr::new(&crate::permissions::plain(&expected))
    );
    let git_config = command
        .get_envs()
        .find(|(key, _)| *key == "GIT_CONFIG_GLOBAL")
        .unwrap()
        .1
        .unwrap();
    let expected_git = std::fs::canonicalize(&scratch).unwrap().join("gitconfig");
    assert_eq!(
        git_config,
        OsStr::new(&crate::permissions::plain(&expected_git))
    );
    let explicit = home.path().join("other-cargo");
    let explicit_git = home.path().join("other-gitconfig");
    let mut command = Command::new("example");
    command.env("CARGO_HOME", &explicit);
    command.env("GIT_CONFIG_GLOBAL", &explicit_git);
    let _ = apply(&mut command, Some(&policy));
    let cargo_home = command
        .get_envs()
        .find(|(key, _)| *key == "CARGO_HOME")
        .unwrap()
        .1
        .unwrap();
    assert_eq!(cargo_home, explicit.as_os_str());
    let git_config = command
        .get_envs()
        .find(|(key, _)| *key == "GIT_CONFIG_GLOBAL")
        .unwrap()
        .1
        .unwrap();
    assert_eq!(git_config, explicit_git.as_os_str());
}

#[test]
fn confined_environment_drops_parent_secrets_and_keeps_explicit_overrides() {
    let project = Directory::new();
    let scratch = project.path().join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let policy = Policy::for_command(Level::Workspace, project.path(), Some(&scratch), None)
        .unwrap()
        .unwrap();
    let mut command = Command::new("example");
    command.env("CUSTOM", "explicit");
    command.env_remove("TEMP");
    command.env(MARKER, "forged");
    confined_environment(
        &mut command,
        &policy,
        [
            (OsString::from("PATH"), OsString::from("C:\\Windows")),
            (OsString::from("TEMP"), OsString::from("C:\\outside")),
            (OsString::from("GH_TOKEN"), OsString::from("secret")),
            (OsString::from("VERCEL_TOKEN"), OsString::from("secret")),
            (
                OsString::from("CARGO_REGISTRIES_CRATES_IO_TOKEN"),
                OsString::from("secret"),
            ),
        ],
    );
    let entries = command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect::<Vec<_>>();
    assert!(
        entries
            .iter()
            .any(|(key, value)| key == "PATH" && value.as_deref() == Some("C:\\Windows"))
    );
    assert!(
        entries
            .iter()
            .any(|(key, value)| key == "CUSTOM" && value.as_deref() == Some("explicit"))
    );
    assert!(
        entries
            .iter()
            .all(|(key, value)| key != "TEMP" || value.is_none())
    );
    for secret in [
        "GH_TOKEN",
        "VERCEL_TOKEN",
        "CARGO_REGISTRIES_CRATES_IO_TOKEN",
        MARKER,
    ] {
        assert!(
            !entries
                .iter()
                .any(|(key, _)| key.eq_ignore_ascii_case(secret))
        );
    }
}

#[test]
fn trusted_marker_replaces_user_environment_and_full_clears_it() {
    let project = Directory::new();
    let policy = Policy::for_command(Level::Workspace, project.path(), None, None)
        .unwrap()
        .unwrap();
    let mut command = Command::new("example");
    command.env(MARKER, "user supplied");
    let result = apply(&mut command, Some(&policy));
    if cfg!(windows) {
        result.unwrap();
        let value = command
            .get_envs()
            .find(|(key, _)| *key == MARKER)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(value.to_string_lossy(), policy.encode());
    } else {
        assert!(result.unwrap_err().contains("only on Windows"));
    }
    apply(&mut command, None).unwrap();
    assert!(
        command
            .get_envs()
            .all(|(key, value)| key != MARKER || value.is_none())
    );
    assert!(
        Policy::for_command(Level::Full, project.path(), None, None)
            .unwrap()
            .is_none()
    );
    assert!(Policy::for_command(Level::Read, project.path(), None, None).is_err());
}

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
    // Confinement attaches only on Windows; the environment is set everywhere.
    let _ = apply(&mut command, Some(&policy));
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
