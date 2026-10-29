use super::*;

/// A minimal fake repository for tests across the crate: the `.git`
/// layout `is_git_dir` accepts, without running git.
pub(crate) fn fake_repo(dir: &Path) {
    let git = dir.join(".git");
    fs::create_dir_all(git.join("refs")).unwrap();
    fs::create_dir_all(git.join("objects")).unwrap();
    fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
}

/// The message of a path error, or a panic when the resolve was
/// something else.
fn expect_path_error(resolved: &Resolved) -> &str {
    match resolved {
        Resolved::PathError(message) => message,
        _ => panic!("not an error"),
    }
}

fn resolved_names(resolved: &Resolved) -> Vec<&str> {
    match resolved {
        Resolved::Repos(repos) => repos.iter().map(|repo| repo.name.as_str()).collect(),
        _ => panic!("not repos"),
    }
}

#[test]
fn expand_home_uses_the_given_home() {
    assert_eq!(
        expand_home("~/x", Some(Path::new("/home/a"))),
        PathBuf::from("/home/a/x")
    );
    assert_eq!(expand_home("~/x", None), PathBuf::from("~/x"));
    assert_eq!(
        expand_home("~", Some(Path::new("/home/a"))),
        PathBuf::from("/home/a")
    );
}

#[test]
fn expand_home_leaves_a_plain_path_alone() {
    assert_eq!(
        expand_home("plain/dir", Some(Path::new("/home/a"))),
        PathBuf::from("plain/dir")
    );
}

#[test]
fn last_component_falls_back_to_the_whole_path() {
    assert_eq!(last_component(Path::new("/")), "/");
    assert_eq!(last_component(Path::new("/a/b")), "b");
}

#[test]
#[should_panic(expected = "not repos")]
fn resolved_names_rejects_a_non_repository_result() {
    resolved_names(&Resolved::Comment("x".to_owned()));
}

#[test]
fn a_repository_outside_the_header_takes_its_last_component() {
    assert_eq!(
        name_for(Path::new("/elsewhere/repo"), Path::new("/base")),
        "repo"
    );
}

#[test]
fn a_repository_at_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fake_repo(&repo);
    let resolved = resolve(repo.to_str().unwrap(), dir.path(), None, 3);
    assert_eq!(resolved_names(&resolved), ["repo"]);
}

#[test]
fn repositories_below_the_path_respect_depth() {
    let dir = tempfile::tempdir().unwrap();
    let deep = dir.path().join("a/b/repo");
    fs::create_dir_all(&deep).unwrap();
    fake_repo(&deep);
    let base = dir.path().to_str().unwrap().to_owned();
    assert!(resolved_names(&resolve(&base, dir.path(), None, 1)).is_empty());
    assert!(resolved_names(&resolve(&base, dir.path(), None, 2)).is_empty());
    assert_eq!(
        resolved_names(&resolve(&base, dir.path(), None, 3)),
        ["a/b/repo"]
    );
    assert_eq!(
        resolved_names(&resolve(&base, dir.path(), None, -1)),
        ["a/b/repo"]
    );
}

#[test]
fn relative_inputs_name_repositories_by_relative_path() {
    let dir = tempfile::tempdir().unwrap();
    let deep = dir.path().join("base/a/repo");
    fs::create_dir_all(&deep).unwrap();
    fake_repo(&deep);
    assert_eq!(
        resolved_names(&resolve("base", dir.path(), None, 3)),
        ["a/repo"]
    );
    let solo = dir.path().join("solo");
    fs::create_dir_all(&solo).unwrap();
    fake_repo(&solo);
    // "." from inside the repository names it by its directory.
    assert_eq!(resolved_names(&resolve(".", &solo, None, 3)), ["solo"]);
}

#[test]
fn a_bare_repository_and_a_worktree_count() {
    let dir = tempfile::tempdir().unwrap();
    let bare = dir.path().join("bare.git");
    fs::create_dir_all(bare.join("objects")).unwrap();
    fs::create_dir_all(bare.join("refs")).unwrap();
    fs::write(bare.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    assert_eq!(
        resolved_names(&resolve(bare.to_str().unwrap(), dir.path(), None, 3)),
        ["bare.git"]
    );

    let admin = dir.path().join("admin");
    fs::create_dir_all(&admin).unwrap();
    fs::write(admin.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(admin.join("commondir"), "../..\n").unwrap();
    let work = dir.path().join("work");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
    assert_eq!(
        resolved_names(&resolve(work.to_str().unwrap(), dir.path(), None, 3)),
        ["work"]
    );
}

#[test]
fn comments_errors_and_missing_paths() {
    let dir = tempfile::tempdir().unwrap();
    match resolve("#  hello ", dir.path(), None, 3) {
        Resolved::Comment(text) => assert_eq!(text, "hello"),
        _ => panic!("not a comment"),
    }
    expect_path_error(&resolve("/nonexistent-9up-test", dir.path(), None, 3));
    let file = dir.path().join("file");
    fs::write(&file, "x").unwrap();
    assert!(
        expect_path_error(&resolve(file.to_str().unwrap(), dir.path(), None, 3))
            .contains("isn't a repository")
    );
    expect_path_error(&resolve("", dir.path(), None, 3));
    expect_path_error(&resolve("/nonexistent-9up-test-*", dir.path(), None, 3));
    expect_path_error(&resolve("[", dir.path(), None, 3));
}

#[cfg(unix)]
#[test]
fn a_broken_symbolic_link_has_no_repositories() {
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(dir.path().join("missing"), &link).unwrap();
    match resolve(link.to_str().unwrap(), dir.path(), None, 3) {
        Resolved::Repos(repos) => assert!(repos.is_empty()),
        _ => panic!("not repos"),
    }
}

#[test]
fn search_skips_files_and_git_directories_and_names_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fake_repo(&repo);
    fs::write(dir.path().join("stray"), "x").unwrap();
    let inner = dir.path().join(".git").join("inner");
    fs::create_dir_all(&inner).unwrap();
    fake_repo(&inner);
    let resolved = resolve(dir.path().to_str().unwrap(), dir.path(), None, 3);
    assert_eq!(resolved_names(&resolved), ["repo"]);
    let found = search(&repo, 3, &repo);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "repo");
}

#[test]
fn a_double_star_glob_lists_each_repository_once() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a/one", "b/two"] {
        let repo = dir.path().join("base").join(name);
        fs::create_dir_all(&repo).unwrap();
        fake_repo(&repo);
    }
    let pattern = format!("{}/base/**", dir.path().display());
    let resolved = resolve(&pattern, dir.path(), None, -1);
    let names = resolved_names(&resolved);
    assert_eq!(names.len(), 2, "{names:?}");
    assert_eq!(names, ["a/one", "b/two"]);
}

#[test]
fn a_symbolic_link_cycle_terminates() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repo = base.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fake_repo(&repo);
    // A repository only reachable by following a symbolic link:
    // without follow_links(true) it is never found.
    let outside = dir.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fake_repo(&outside);
    // A cycle: base/loop points back at base. walkdir reports the
    // loop as an error entry, which search() skips.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&base, base.join("loop")).unwrap();
        std::os::unix::fs::symlink(&outside, base.join("linked")).unwrap();
    }
    let resolved = resolve(base.to_str().unwrap(), dir.path(), None, -1);
    assert_eq!(resolved_names(&resolved), ["linked", "repo"]);
}

#[test]
fn a_glob_finds_repositories() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["one", "two"] {
        let repo = dir.path().join(name);
        fs::create_dir_all(&repo).unwrap();
        fake_repo(&repo);
    }
    let pattern = format!("{}/*", dir.path().display());
    assert_eq!(
        resolved_names(&resolve(&pattern, dir.path(), None, 3)),
        ["one", "two"]
    );
    let dot = dir.path().join(".hidden");
    fs::create_dir_all(&dot).unwrap();
    fake_repo(&dot);
    assert_eq!(
        resolved_names(&resolve(&pattern, dir.path(), None, 3)),
        ["one", "two"]
    );
}

#[test]
fn path_is_live_covers_directories_files_and_broken_links() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    assert!(path_is_live(root));
    let file = root.join("file.txt");
    fs::write(&file, "").unwrap();
    assert!(path_is_live(&file));
    assert!(!path_is_live(&root.join("missing")));
    #[cfg(unix)]
    {
        let link = root.join("link");
        std::os::unix::fs::symlink(root.join("gone"), &link).unwrap();
        assert!(path_is_live(&link));
    }
}
