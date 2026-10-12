use super::*;
use filesystem::{Phase, Publisher, resolve, seal};
use sha2::{Digest, Sha256};
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};
// Unix-only file properties (modes, symlinks) keep their assertions on Unix.
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    base: PathBuf,
    root: PathBuf,
    input: PathBuf,
    manifest: Manifest,
}

fn identity(generation: usize) -> BuildIdentity {
    BuildIdentity::new(
        Some(&format!("source-{generation}")),
        Some(&format!("native-{generation}")),
        Some(&format!("wasm-{generation}")),
        Some(&format!("configuration-{generation}")),
        Some(&format!("content-{generation}")),
    )
    .unwrap()
}

fn identity_bytes(identity: &BuildIdentity) -> Vec<u8> {
    let mut text = String::from("DF-BUILD-IDENTITY-V1\n");
    for (name, revision) in LABELS.into_iter().zip(REVISIONS) {
        text.push_str(&format!(
            "{name}={}\n",
            identity.revision(revision).as_str()
        ));
    }
    text.into_bytes()
}

fn directory(path: &Path) {
    // Directory modes are a Unix-only property.
    #[cfg(unix)]
    {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .unwrap();
    }
    #[cfg(not(unix))]
    {
        fs::DirBuilder::new().recursive(true).create(path).unwrap();
    }
}

fn candidate(base: &Path, generation: usize) -> (PathBuf, Manifest) {
    let input = base.join(format!("input-{generation}"));
    directory(&input.join("native"));
    directory(&input.join("web/assets"));
    let identity = identity(generation);
    fs::write(input.join("identity.txt"), identity_bytes(&identity)).unwrap();
    // These are real finite filesystem bytes, not a claim that synthetic WASM/native
    // fixture bytes were produced by a compiler or executed as an application.
    for (role, path, _) in REQUIRED {
        fs::write(input.join(path), format!("fixture-{role}-{generation}")).unwrap();
    }
    let asset = format!("asset-{generation}").into_bytes();
    fs::write(input.join("web/assets/map.png"), &asset).unwrap();
    let content = format!(
        "DF-BUILD-CONTENT-V1\nrevision=content-{generation}\nasset:map\tweb/assets/map.png\t{}\t{}\n",
        asset.len(),
        hex(&Sha256::digest(&asset).into())
    );
    fs::write(input.join("web/content-manifest.txt"), content).unwrap();
    let manifest = seal(&input, &identity).unwrap();
    (input, manifest)
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "build-set-boundary-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        directory(&base);
        let (input, manifest) = candidate(&base, 1);
        let root = base.join("published");
        let owner = Publisher::acquire(&root).unwrap();
        owner
            .publish(&input, &manifest.serialize().unwrap(), &mut |_| Ok(()))
            .unwrap();
        // Only this fresh fixture namespace is retained. No ambient artifact cleanup.
        Self {
            base,
            root,
            input,
            manifest,
        }
    }

    fn current(&self) -> Vec<u8> {
        fs::read(self.root.join("current")).unwrap()
    }
}

fn cut() -> BuildSetError {
    BuildSetError::Io(io::Error::other("controlled publication cut"))
}

#[test]
fn publication_recovery_boundary() {
    let fixture = Fixture::new();
    let previous = fixture.current();
    let selected = resolve(&fixture.root).unwrap();
    let previous_id = selected.id;
    let old = ready(
        selected,
        &fixture.input.join("native/df-transport-fixture"),
        "source-1",
        43180,
    )
    .unwrap();
    let old_loader = old.asset(&old.set_id(), "df_tools.js").unwrap();
    let (input, manifest) = candidate(&fixture.base, 2);
    let owner = Publisher::acquire(&fixture.root).unwrap();
    assert!(matches!(
        Publisher::acquire(&fixture.root),
        Err(BuildSetError::PublisherBusy)
    ));
    let mut before = false;
    let expected_id: [u8; 32] = Sha256::digest(manifest.serialize().unwrap()).into();
    let id = owner
        .publish(&input, &manifest.serialize().unwrap(), &mut |phase| {
            if phase == Phase::BeforeReferenceRename {
                before = true;
                assert_eq!(
                    filesystem::select(&fixture.root, expected_id).unwrap().id,
                    expected_id
                );
                assert_eq!(resolve(&fixture.root).unwrap().id, previous_id);
                assert_eq!(fixture.current(), previous);
            }
            Ok(())
        })
        .unwrap();
    assert!(before);
    assert_ne!(id, previous_id);
    let selected = resolve(&fixture.root).unwrap();
    assert_eq!(selected.id, id);
    let new = ready(
        selected,
        &input.join("native/df-transport-fixture"),
        "source-2",
        43180,
    )
    .unwrap();
    assert_ne!(old_loader, new.asset(&new.set_id(), "df_tools.js").unwrap());
    assert_eq!(old.asset(&old.set_id(), "df_tools.js").unwrap(), old_loader);
    assert!(old.asset(&new.set_id(), "df_tools.js").is_none());
    assert!(new.asset(&old.set_id(), "df_tools.js").is_none());
    assert!(
        new.asset(&new.set_id(), "../native/df-transport-fixture")
            .is_none()
    );
    assert!(
        new.asset(&new.set_id(), "native/df-transport-fixture")
            .is_none()
    );
    for (revision, name) in REVISIONS.into_iter().zip(LABELS) {
        let bytes = identity_bytes(&identity(1));
        let value = String::from_utf8(bytes)
            .unwrap()
            .replace(&format!("{name}={name}-1"), &format!("{name}=different"));
        let changed = parse_identity(value.as_bytes()).unwrap();
        assert!(
            matches!(compare_identity(&identity(1), &changed), Err(BuildSetError::RevisionMismatch(r)) if r == revision)
        );
    }
    for value in ["", "bad label", "λ", &"x".repeat(129)] {
        let bytes = identity_bytes(&identity(1));
        let value = String::from_utf8(bytes)
            .unwrap()
            .replace("source=source-1", &format!("source={value}"));
        assert!(parse_identity(value.as_bytes()).is_err());
    }
    assert!(parse_identity(b"DF-BUILD-IDENTITY-V1\nsource=source-1\n").is_err());
    let source = old.identity().revision(BuildRevision::Source).as_str();
    assert_eq!(source, "source-1");
    assert!(matches!(
        ready(
            resolve(&fixture.root).unwrap(),
            &fixture.input.join("native/df-transport-fixture"),
            "source-2",
            43180
        ),
        Err(BuildSetError::NativeMismatch)
    ));
    assert!(matches!(
        ready(
            resolve(&fixture.root).unwrap(),
            &input.join("native/df-transport-fixture"),
            "unregistered-cargo-build",
            43180
        ),
        Err(BuildSetError::NativeMismatch)
    ));
    assert!(
        ready(
            resolve(&fixture.root).unwrap(),
            &input.join("native/df-transport-fixture"),
            "source-2",
            0
        )
        .is_err()
    );
    assert!(dispatch(&[OsString::from("/raw/web"), OsString::from("43180")]).is_err());
    // The production dispatcher supplies this test process's actual current_exe.
    // Synthetic candidate bytes must never become a serving capability through it.
    assert!(matches!(
        dispatch(&[
            OsString::from("--serve-selected"),
            fixture.root.as_os_str().to_owned(),
            OsString::from(new.set_id()),
            OsString::from("43180"),
        ]),
        Err(BuildSetError::NativeMismatch)
    ));
    let sealed_path = fixture.base.join("dispatcher.manifest");
    filesystem::write_sealed(&sealed_path, &manifest.serialize().unwrap()).unwrap();
    let refused_root = fixture.base.join("refused-publication");
    assert!(matches!(
        dispatch(&[
            OsString::from("--publish-build-set"),
            refused_root.as_os_str().to_owned(),
            input.as_os_str().to_owned(),
            sealed_path.as_os_str().to_owned(),
        ]),
        Err(BuildSetError::NativeMismatch)
    ));
    assert!(!refused_root.exists());
    drop(owner);

    bounded_inputs();

    let bytes = fixture.manifest.serialize().unwrap();
    for changed in [
        bytes[..bytes.len() - 1].to_vec(),
        [bytes.as_slice(), b"extra\n"].concat(),
        vec![b'x'; MANIFEST_LIMIT + 1],
        String::from_utf8(bytes.clone())
            .unwrap()
            .replace("DF-BUILD-SET-V1", "DF-BUILD-SET-V2")
            .into_bytes(),
    ] {
        assert!(Manifest::parse(&changed).is_err());
    }
    for index in 0..fixture.manifest.parts.len() {
        let missing_bytes = remove_record(&bytes, index);
        let retained = fixture.current();
        assert!(
            Publisher::acquire(&fixture.root)
                .unwrap()
                .publish(&fixture.input, &missing_bytes, &mut |_| Ok(()))
                .is_err()
        );
        assert_eq!(fixture.current(), retained);
        let mut duplicate = fixture.manifest.clone();
        duplicate.parts.push(duplicate.parts[index].clone());
        assert!(duplicate.serialize().is_err());
        let mut changed = fixture.manifest.clone();
        changed.parts[index].digest[0] ^= 1;
        assert!(
            Publisher::acquire(&fixture.root)
                .unwrap()
                .publish(&fixture.input, &changed.serialize().unwrap(), &mut |_| Ok(
                    ()
                ))
                .is_err()
        );
        let mut length = fixture.manifest.clone();
        length.parts[index].len += 1;
        assert!(
            Publisher::acquire(&fixture.root)
                .unwrap()
                .publish(
                    &fixture.input,
                    &length.serialize().unwrap(),
                    &mut |_| Ok(())
                )
                .is_err()
        );
    }
    let mut extra = fixture.manifest.clone();
    extra.parts.push(Part {
        role: "unknown".into(),
        path: "web/unknown".into(),
        len: 1,
        digest: [0; 32],
    });
    assert!(extra.serialize().is_err());
    let mut overbound = fixture.manifest.clone();
    overbound.parts[0].len = REQUIRED[0].2 + 1;
    assert!(matches!(overbound.validate(), Err(BuildSetError::Limit)));
    for path in [
        "/absolute",
        "web/../outside",
        "web//double",
        "web/./dot",
        "web/λ",
        &"x".repeat(241),
    ] {
        assert!(safe_relative(path).is_err());
    }
    // Creating a symlink is a Unix-only case; the other mutations keep running
    // on Windows.
    #[cfg(unix)]
    let kinds = ["extra", "missing", "symlink", "hardlink", "oversized"];
    #[cfg(not(unix))]
    let kinds = ["extra", "missing", "hardlink", "oversized"];
    for kind in kinds {
        let f = Fixture::new();
        let previous = f.current();
        let target = f.input.join("web/assets/map.png");
        match kind {
            "extra" => fs::write(f.input.join("web/extra"), b"extra").unwrap(),
            "missing" => fs::remove_file(&target).unwrap(),
            #[cfg(unix)]
            "symlink" => {
                fs::remove_file(&target).unwrap();
                symlink(f.input.join("web/df_tools.js"), &target).unwrap();
            }
            "hardlink" => fs::hard_link(&target, f.base.join("link")).unwrap(),
            "oversized" => fs::OpenOptions::new()
                .write(true)
                .open(&target)
                .unwrap()
                .set_len(16 * 1024 * 1024 + 1)
                .unwrap(),
            _ => unreachable!(),
        }
        assert!(seal(&f.input, &identity(1)).is_err(), "{kind}");
        assert!(
            Publisher::acquire(&f.root)
                .unwrap()
                .publish(&f.input, &f.manifest.serialize().unwrap(), &mut |_| Ok(()))
                .is_err(),
            "{kind}"
        );
        assert_eq!(f.current(), previous);
    }
    for index in 0..fixture.manifest.parts.len() {
        for kind in ["missing", "truncated", "mutated"] {
            let f = Fixture::new();
            let previous = f.current();
            let target = f.input.join(&f.manifest.parts[index].path);
            match kind {
                "missing" => fs::remove_file(&target).unwrap(),
                "truncated" => {
                    let file = fs::OpenOptions::new().write(true).open(&target).unwrap();
                    file.set_len(f.manifest.parts[index].len - 1).unwrap();
                }
                "mutated" => {
                    let mut bytes = fs::read(&target).unwrap();
                    bytes[0] ^= 1;
                    fs::write(&target, bytes).unwrap();
                }
                _ => unreachable!(),
            }
            // Sealing hashes current input; publishing an independently sealed
            // previous expectation must reject every changed component's bytes.
            assert!(
                Publisher::acquire(&f.root)
                    .unwrap()
                    .publish(&f.input, &f.manifest.serialize().unwrap(), &mut |_| Ok(()))
                    .is_err(),
                "{} {kind}",
                f.manifest.parts[index].role
            );
            assert_eq!(f.current(), previous);
        }
    }

    let mut phases = vec![
        Phase::ManifestWritten,
        Phase::ManifestSynced,
        Phase::SetPlaced,
        Phase::BeforeReferenceRename,
    ];
    for index in 0..fixture.manifest.parts.len() {
        phases.extend([
            Phase::FileCreated(index),
            Phase::FileWritten(index),
            Phase::FileSynced(index),
        ]);
    }
    for phase in phases {
        let f = Fixture::new();
        let previous = f.current();
        let old_id = resolve(&f.root).unwrap().id;
        let (input, manifest) = candidate(&f.base, 2);
        let owner = Publisher::acquire(&f.root).unwrap();
        let error = owner.publish(&input, &manifest.serialize().unwrap(), &mut |at| {
            if at == phase { Err(cut()) } else { Ok(()) }
        });
        assert!(error.is_err(), "{phase:?}");
        assert_eq!(f.current(), previous, "{phase:?}");
        assert_eq!(resolve(&f.root).unwrap().id, old_id, "{phase:?}");
    }
    for phase in [Phase::AfterReferenceRename, Phase::AfterReferenceSync] {
        let f = Fixture::new();
        let previous = f.current();
        let (input, manifest) = candidate(&f.base, 2);
        let candidate: [u8; 32] = Sha256::digest(manifest.serialize().unwrap()).into();
        let owner = Publisher::acquire(&f.root).unwrap();
        let error = owner.publish(&input, &manifest.serialize().unwrap(), &mut |at| {
            if at == phase { Err(cut()) } else { Ok(()) }
        });
        assert!(matches!(
            error,
            Err(BuildSetError::AfterVisibility(VisibleSet::Candidate))
        ));
        assert_ne!(f.current(), previous);
        assert_eq!(resolve(&f.root).unwrap().id, candidate);
    }

    for kind in [
        "missing-reference",
        "torn-reference",
        "missing-manifest",
        "missing-component",
        "changed-component",
    ] {
        let f = Fixture::new();
        let selected = resolve(&f.root).unwrap();
        let reference = f.root.join("current");
        match kind {
            "missing-reference" => fs::remove_file(reference).unwrap(),
            "torn-reference" => rewrite(&reference, b"DF-CURRENT-BUILD-V1\npartial\n"),
            "missing-manifest" => {
                // Published-set modes are a Unix-only property; removal alone
                // still exercises the missing-file refusal on Windows.
                #[cfg(unix)]
                fs::set_permissions(&selected.directory, fs::Permissions::from_mode(0o700))
                    .unwrap();
                fs::remove_file(selected.directory.join("manifest.txt")).unwrap();
                #[cfg(unix)]
                fs::set_permissions(&selected.directory, fs::Permissions::from_mode(0o500))
                    .unwrap();
            }
            "missing-component" => {
                #[cfg(unix)]
                fs::set_permissions(
                    selected.directory.join("web"),
                    fs::Permissions::from_mode(0o700),
                )
                .unwrap();
                fs::remove_file(selected.directory.join("web/df_tools.js")).unwrap();
                #[cfg(unix)]
                fs::set_permissions(
                    selected.directory.join("web"),
                    fs::Permissions::from_mode(0o500),
                )
                .unwrap();
            }
            "changed-component" => rewrite(&selected.directory.join("web/df_tools.js"), b"changed"),
            _ => unreachable!(),
        }
        assert!(resolve(&f.root).is_err(), "{kind}");
        // No resolver writes a pointer or chooses a staging/orphan directory.
        if kind == "missing-reference" {
            assert!(!f.root.join("current").exists());
        }
    }
    // Swapping the held root mid-publish needs a Unix directory rename while
    // its lock file is open; the other two sabotage cases keep running on
    // Windows.
    #[cfg(unix)]
    let kinds = ["reference", "lock", "root"];
    #[cfg(not(unix))]
    let kinds = ["reference", "lock"];
    for kind in kinds {
        let f = Fixture::new();
        let previous = f.current();
        let (input, manifest) = candidate(&f.base, 2);
        let owner = Publisher::acquire(&f.root).unwrap();
        let result = owner.publish(&input, &manifest.serialize().unwrap(), &mut |phase| {
            if phase == Phase::BeforeReferenceRename {
                match kind {
                    "reference" => {
                        let replacement = f.root.join("replacement");
                        fs::write(&replacement, &previous).unwrap();
                        // Reference modes are a Unix-only property.
                        #[cfg(unix)]
                        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o400))
                            .unwrap();
                        fs::rename(replacement, f.root.join("current")).unwrap();
                    }
                    "lock" => {
                        fs::rename(f.root.join("publisher.lock"), f.root.join("old-lock")).unwrap();
                        fs::write(f.root.join("publisher.lock"), b"").unwrap();
                        #[cfg(unix)]
                        fs::set_permissions(
                            f.root.join("publisher.lock"),
                            fs::Permissions::from_mode(0o600),
                        )
                        .unwrap();
                    }
                    #[cfg(unix)]
                    "root" => {
                        fs::rename(&f.root, f.base.join("retained-root")).unwrap();
                        directory(&f.root);
                    }
                    _ => unreachable!(),
                }
            }
            Ok(())
        });
        assert!(
            matches!(result, Err(BuildSetError::OwnershipChanged)),
            "{kind}: {result:?}"
        );
        if kind != "root" {
            assert_eq!(f.current(), previous);
        }
    }
    // Resolving through a symlinked root is a Unix-only case.
    #[cfg(unix)]
    {
        let alias_fixture = Fixture::new();
        let alias = alias_fixture.base.join("alias");
        symlink(&alias_fixture.root, &alias).unwrap();
        assert!(Publisher::acquire(&alias).is_err());
        assert!(resolve(&alias).is_err());
    }
}

fn rewrite(path: &Path, bytes: &[u8]) {
    // File modes are a Unix-only property; on Windows the bytes alone change.
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o400)).unwrap();
}

fn remove_record(bytes: &[u8], index: usize) -> Vec<u8> {
    let value = std::str::from_utf8(bytes).unwrap();
    let mut lines: Vec<_> = value.lines().map(str::to_owned).collect();
    let count = lines[6]
        .strip_prefix("parts=")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    lines[6] = format!("parts={}", count - 1);
    lines.remove(7 + index);
    (lines.join("\n") + "\n").into_bytes()
}

fn bounded_inputs() {
    for limit in [IDENTITY_LIMIT, MANIFEST_LIMIT, 128] {
        let mut exact = vec![b'x'; limit];
        exact[limit - 1] = b'\n';
        assert!(text(&exact, limit).is_ok());
        exact.insert(0, b'x');
        assert!(matches!(text(&exact, limit), Err(BuildSetError::Limit)));
    }
    let label = "x".repeat(128);
    assert!(RevisionLabel::new(Some(&label)).is_ok());
    assert!(RevisionLabel::new(Some(&(label + "x"))).is_err());
    assert!(safe_relative(&"x".repeat(240)).is_ok());
    assert!(safe_relative(&"x".repeat(241)).is_err());
    assert!(safe_relative("a/b/c/d/e/f/g/h").is_ok());
    assert!(safe_relative("a/b/c/d/e/f/g/h/i").is_err());
    assert!(parse_digest(&"0".repeat(64)).is_ok());
    assert!(parse_digest(&"0".repeat(65)).is_err());
    assert!(path_argument(&OsString::from("x".repeat(4096))).is_ok());
    assert!(path_argument(&OsString::from("x".repeat(4097))).is_err());

    let f = Fixture::new();
    let mut maximum = f.manifest.clone();
    maximum
        .parts
        .retain(|part| !part.role.starts_with("asset:"));
    let mut content = String::from("DF-BUILD-CONTENT-V1\nrevision=content-1\n");
    for index in 0..ASSET_LIMIT {
        let part = Part {
            role: format!("asset:a{index}"),
            path: format!("web/assets/a{index}"),
            len: 1,
            digest: [0; 32],
        };
        content.push_str(&format!(
            "{}\t{}\t1\t{}\n",
            part.role,
            part.path,
            hex(&part.digest)
        ));
        maximum.parts.push(part);
    }
    assert_eq!(maximum.parts.len(), PART_LIMIT);
    assert!(Manifest::parse(&maximum.serialize().unwrap()).is_ok());
    assert_eq!(
        content_parts(content.as_bytes(), &identity(1))
            .unwrap()
            .len(),
        ASSET_LIMIT
    );
    content.push_str(&format!(
        "asset:overflow\tweb/assets/overflow\t1\t{}\n",
        hex(&[0; 32])
    ));
    assert!(matches!(
        content_parts(content.as_bytes(), &identity(1)),
        Err(BuildSetError::Limit)
    ));
    let over_count = String::from_utf8(maximum.serialize().unwrap())
        .unwrap()
        .replace(
            &format!("parts={PART_LIMIT}\n"),
            &format!("parts={}\n", PART_LIMIT + 1),
        );
    assert!(matches!(
        Manifest::parse(over_count.as_bytes()),
        Err(BuildSetError::Limit)
    ));

    let mut web_boundary = maximum.clone();
    for part in &mut web_boundary.parts[REQUIRED.len()..REQUIRED.len() + 7] {
        part.len = 16 * 1024 * 1024;
    }
    let last = REQUIRED.len() + 7;
    let other_web: u64 = web_boundary
        .parts
        .iter()
        .enumerate()
        .filter(|(index, part)| *index != last && part.path.starts_with("web/"))
        .map(|(_, part)| part.len)
        .sum();
    web_boundary.parts[last].len = WEB_LIMIT - other_web;
    assert!(web_boundary.validate().is_ok());
    web_boundary.parts[last].len += 1;
    assert!(matches!(web_boundary.validate(), Err(BuildSetError::Limit)));

    // Each sparse file is created only inside its fresh fixture namespace. Metadata
    // admission rejects limit+1 before hashing its potentially large logical length.
    for (index, (_, path, limit)) in REQUIRED.into_iter().enumerate() {
        let f = Fixture::new();
        let previous = f.current();
        let mut boundary = f.manifest.clone();
        boundary.parts[index].len = limit;
        assert!(boundary.validate().is_ok(), "{path}");
        boundary.parts[index].len += 1;
        assert!(
            matches!(boundary.validate(), Err(BuildSetError::Limit)),
            "{path}"
        );
        fs::OpenOptions::new()
            .write(true)
            .open(f.input.join(path))
            .unwrap()
            .set_len(limit + 1)
            .unwrap();
        assert!(seal(&f.input, &identity(1)).is_err(), "{path}");
        assert!(
            Publisher::acquire(&f.root)
                .unwrap()
                .publish(&f.input, &f.manifest.serialize().unwrap(), &mut |_| Ok(()))
                .is_err(),
            "{path}"
        );
        assert_eq!(f.current(), previous, "{path}");
    }
    {
        let path = "identity.txt";
        let limit = IDENTITY_LIMIT as u64;
        let f = Fixture::new();
        let previous = f.current();
        fs::OpenOptions::new()
            .write(true)
            .open(f.input.join(path))
            .unwrap()
            .set_len(limit + 1)
            .unwrap();
        assert!(seal(&f.input, &identity(1)).is_err());
        assert!(
            Publisher::acquire(&f.root)
                .unwrap()
                .publish(&f.input, &f.manifest.serialize().unwrap(), &mut |_| Ok(()))
                .is_err()
        );
        assert_eq!(f.current(), previous);
    }
    let f = Fixture::new();
    rewrite(&f.root.join("current"), &[b'x'; 129]);
    assert!(matches!(resolve(&f.root), Err(BuildSetError::Limit)));
    assert_eq!(f.current().len(), 129);
}
