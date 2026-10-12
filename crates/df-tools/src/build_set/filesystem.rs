use super::{
    BuildSetError, IDENTITY_LIMIT, MANIFEST_LIMIT, Manifest, PART_LIMIT, Part, REQUIRED, Result,
    SelectedSet, VisibleSet, compare_identity, content_parts, hex, parse_digest, parse_identity,
    role_limit, safe_relative,
};
use crate::file_identity::{self, FileIdentity, same};
use df_types::BuildIdentity;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const REFERENCE_LIMIT: usize = 128;
const TREE_LIMIT: usize = PART_LIMIT * 8 + 4;
static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    FileCreated(usize),
    FileWritten(usize),
    FileSynced(usize),
    ManifestWritten,
    ManifestSynced,
    SetPlaced,
    BeforeReferenceRename,
    AfterReferenceRename,
    AfterReferenceSync,
}

fn unique(prefix: &str) -> String {
    format!(
        ".{prefix}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn canonical_directory(path: &Path, mode: u32) -> Result<FileIdentity> {
    if !path.is_absolute() || path.as_os_str().len() > 4096 {
        return Err(BuildSetError::UnsafePath);
    }
    if !file_identity::canonical_equals(&fs::canonicalize(path)?, path) {
        return Err(BuildSetError::UnsafePath);
    }
    let identity = file_identity::snapshot_path(path)?;
    if !identity.is_dir() || identity.is_symlink() || !identity.mode_equals(mode) {
        return Err(BuildSetError::UnsafePath);
    }
    Ok(identity)
}

fn regular(path: &Path, maximum: u64) -> Result<FileIdentity> {
    if !path.is_absolute() || path.as_os_str().len() > 4096 {
        return Err(BuildSetError::UnsafePath);
    }
    if !file_identity::canonical_equals(&fs::canonicalize(path)?, path) {
        return Err(BuildSetError::UnsafePath);
    }
    let identity = file_identity::snapshot_path(path)?;
    if !identity.is_file() || identity.is_symlink() || identity.links() != 1 {
        return Err(BuildSetError::UnsafePath);
    }
    if identity.len() > maximum {
        return Err(BuildSetError::Limit);
    }
    Ok(identity)
}

fn open_regular(path: &Path, maximum: u64) -> Result<(File, FileIdentity)> {
    let before = regular(path, maximum)?;
    let file = File::open(path)?;
    let after_open = file_identity::snapshot_file(&file)?;
    if !same(&before, &after_open) || !same(&before, &file_identity::snapshot_path(path)?) {
        return Err(BuildSetError::OwnershipChanged);
    }
    Ok((file, before))
}

fn finish_read(path: &Path, file: &File, before: &FileIdentity) -> Result<()> {
    if !same(before, &file_identity::snapshot_file(file)?)
        || !same(before, &file_identity::snapshot_path(path)?)
    {
        return Err(BuildSetError::OwnershipChanged);
    }
    Ok(())
}

pub(super) fn read_external(path: &Path, maximum: usize) -> Result<Vec<u8>> {
    let (mut file, before) = open_regular(path, maximum as u64)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(BuildSetError::Limit);
    }
    finish_read(path, &file, &before)?;
    Ok(bytes)
}

pub(super) fn observe_file(path: &Path, maximum: u64) -> Result<(u64, [u8; 32])> {
    let (mut file, before) = open_regular(path, maximum)?;
    let mut hash = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0; 16 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        length = length
            .checked_add(read as u64)
            .ok_or(BuildSetError::Limit)?;
        if length > maximum {
            return Err(BuildSetError::Limit);
        }
        hash.update(&buffer[..read]);
    }
    finish_read(path, &file, &before)?;
    if length != before.len() {
        return Err(BuildSetError::DigestMismatch);
    }
    Ok((length, hash.finalize().into()))
}

fn inside(root: &Path, relative: &str) -> Result<PathBuf> {
    safe_relative(relative)?;
    let root_identity = file_identity::snapshot_path(root)?;
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        path.push(component.as_os_str());
        let identity = file_identity::snapshot_path(&path)?;
        if identity.is_symlink()
            || identity.volume() != root_identity.volume()
            || identity.owner() != root_identity.owner()
            || (!identity.is_file() && !identity.is_dir())
            || (identity.is_file() && identity.links() != 1)
        {
            return Err(BuildSetError::UnsafePath);
        }
    }
    Ok(path)
}

fn tree(root: &Path, files: &BTreeSet<String>, immutable: bool) -> Result<()> {
    let root_identity = file_identity::snapshot_path(root)?;
    let mut directories = BTreeSet::from([String::from("web/assets")]);
    for name in files {
        safe_relative(name)?;
        let mut parent = Path::new(name).parent();
        while let Some(path) = parent.filter(|p| !p.as_os_str().is_empty()) {
            directories.insert(path.to_str().ok_or(BuildSetError::UnsafePath)?.into());
            parent = path.parent();
        }
    }
    let mut pending = vec![root.to_path_buf()];
    let mut observed = BTreeSet::new();
    let mut count = 0;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            count += 1;
            if count > TREE_LIMIT {
                return Err(BuildSetError::Limit);
            }
            let path = entry?.path();
            let relative =
                file_identity::relative_name(root, &path).ok_or(BuildSetError::UnsafePath)?;
            safe_relative(&relative)?;
            let identity = file_identity::snapshot_path(&path)?;
            if identity.is_symlink()
                || identity.volume() != root_identity.volume()
                || identity.owner() != root_identity.owner()
            {
                return Err(BuildSetError::UnsafePath);
            }
            if identity.is_dir() {
                if !directories.contains(&relative) || (immutable && !identity.mode_equals(0o500)) {
                    return Err(BuildSetError::ComponentClosure);
                }
                pending.push(path);
            } else if identity.is_file() && identity.links() == 1 {
                if !files.contains(&relative)
                    || (immutable
                        && !identity.mode_equals(if relative == "native/df-transport-fixture" {
                            0o500
                        } else {
                            0o400
                        }))
                {
                    return Err(BuildSetError::ComponentClosure);
                }
                observed.insert(relative);
            } else {
                return Err(BuildSetError::UnsafePath);
            }
        }
    }
    if &observed != files {
        return Err(BuildSetError::ComponentClosure);
    }
    Ok(())
}

fn verify_content(root: &Path, manifest: &Manifest) -> Result<()> {
    let bytes = read_external(&inside(root, "web/content-manifest.txt")?, MANIFEST_LIMIT)?;
    let expected = content_parts(&bytes, &manifest.identity)?;
    let observed: Vec<_> = manifest
        .parts
        .iter()
        .filter(|p| p.role.starts_with("asset:"))
        .cloned()
        .collect();
    if expected != observed {
        return Err(BuildSetError::ComponentClosure);
    }
    Ok(())
}

fn verify_parts(root: &Path, manifest: &Manifest, immutable: bool) -> Result<()> {
    let mut files: BTreeSet<_> = manifest.parts.iter().map(|p| p.path.clone()).collect();
    files.insert(
        if immutable {
            "manifest.txt"
        } else {
            "identity.txt"
        }
        .into(),
    );
    tree(root, &files, immutable)?;
    for part in &manifest.parts {
        if observe_file(&inside(root, &part.path)?, role_limit(part)?)? != (part.len, part.digest) {
            return Err(BuildSetError::DigestMismatch);
        }
    }
    verify_content(root, manifest)?;
    Ok(())
}

pub(super) fn seal(input: &Path, expected: &BuildIdentity) -> Result<Manifest> {
    let before = canonical_directory(input, 0o700)?;
    let observed = parse_identity(&read_external(
        &inside(input, "identity.txt")?,
        IDENTITY_LIMIT,
    )?)?;
    compare_identity(expected, &observed)?;
    let mut parts = Vec::with_capacity(REQUIRED.len());
    for (role, path, maximum) in REQUIRED {
        let (len, digest) = observe_file(&inside(input, path)?, maximum)?;
        parts.push(Part {
            role: role.into(),
            path: path.into(),
            len,
            digest,
        });
    }
    parts.extend(content_parts(
        &read_external(&inside(input, "web/content-manifest.txt")?, MANIFEST_LIMIT)?,
        expected,
    )?);
    let manifest = Manifest {
        identity: expected.clone(),
        parts,
    };
    manifest.validate()?;
    verify_parts(input, &manifest, false)?;
    if !same(&before, &file_identity::snapshot_path(input)?) {
        return Err(BuildSetError::OwnershipChanged);
    }
    Ok(manifest)
}

pub(super) fn write_sealed(path: &Path, bytes: &[u8]) -> Result<()> {
    if !path.is_absolute() || path.as_os_str().len() > 4096 || bytes.len() > MANIFEST_LIMIT {
        return Err(BuildSetError::UnsafePath);
    }
    let parent = path.parent().ok_or(BuildSetError::UnsafePath)?;
    canonical_directory(parent, 0o700)?;
    let mut file = file_identity::create_file_exclusive(path)?;
    file.write_all(bytes)?;
    file_identity::set_mode(path, 0o400)?;
    file.sync_all()?;
    file_identity::sync_directory(parent)?;
    Ok(())
}

struct Root {
    path: PathBuf,
    identity: FileIdentity,
}

impl Root {
    fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            path: path.into(),
            identity: canonical_directory(path, 0o700)?,
        })
    }
    fn check(&self) -> Result<()> {
        if !same(&self.identity, &file_identity::snapshot_path(&self.path)?)
            || !file_identity::canonical_equals(&fs::canonicalize(&self.path)?, &self.path)
        {
            return Err(BuildSetError::OwnershipChanged);
        }
        Ok(())
    }
}

pub(super) struct Publisher {
    root: Root,
    lock: File,
    lock_identity: FileIdentity,
    sets_identity: FileIdentity,
}

impl Publisher {
    pub(super) fn acquire(path: &Path) -> Result<Self> {
        if !path.is_absolute() || path.as_os_str().len() > 4096 {
            return Err(BuildSetError::UnsafePath);
        }
        match file_identity::create_dir(path) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
        let root = Root::open(path)?;
        let lock_path = path.join("publisher.lock");
        let lock = match file_identity::create_file_exclusive(&lock_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let before = regular(&lock_path, 0)?;
                let file = OpenOptions::new().read(true).write(true).open(&lock_path)?;
                if !same(&before, &file_identity::snapshot_file(&file)?) {
                    return Err(BuildSetError::OwnershipChanged);
                }
                file
            }
            Err(error) => return Err(error.into()),
        };
        let lock_identity = file_identity::snapshot_file(&lock)?;
        if lock_identity.owner() != root.identity.owner()
            || !lock_identity.mode_equals(0o600)
            || !lock_identity.is_file()
            || lock_identity.links() != 1
            || !same(&lock_identity, &file_identity::snapshot_path(&lock_path)?)
        {
            return Err(BuildSetError::OwnershipChanged);
        }
        lock.try_lock().map_err(|_| BuildSetError::PublisherBusy)?;
        let sets = path.join("sets");
        match file_identity::create_dir(&sets) {
            Ok(()) => file_identity::sync_directory(path)?,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
        let sets_identity = canonical_directory(&sets, 0o700)?;
        if sets_identity.owner() != root.identity.owner()
            || sets_identity.volume() != root.identity.volume()
        {
            return Err(BuildSetError::OwnershipChanged);
        }
        let owner = Self {
            root,
            lock,
            lock_identity,
            sets_identity,
        };
        owner.check()?;
        Ok(owner)
    }

    fn check(&self) -> Result<()> {
        self.root.check()?;
        if !same(
            &self.lock_identity,
            &file_identity::snapshot_file(&self.lock)?,
        ) || !same(
            &self.lock_identity,
            &file_identity::snapshot_path(&self.root.path.join("publisher.lock"))?,
        ) || !same(
            &self.sets_identity,
            &file_identity::snapshot_path(&self.root.path.join("sets"))?,
        ) {
            return Err(BuildSetError::OwnershipChanged);
        }
        Ok(())
    }

    pub(super) fn publish(
        &self,
        input: &Path,
        bytes: &[u8],
        step: &mut dyn FnMut(Phase) -> Result<()>,
    ) -> Result<[u8; 32]> {
        self.check()?;
        let input_identity = canonical_directory(input, 0o700)?;
        if input.starts_with(&self.root.path) || self.root.path.starts_with(input) {
            return Err(BuildSetError::UnsafePath);
        }
        let manifest = Manifest::parse(bytes)?;
        let observed = parse_identity(&read_external(
            &inside(input, "identity.txt")?,
            IDENTITY_LIMIT,
        )?)?;
        compare_identity(&manifest.identity, &observed)?;
        verify_parts(input, &manifest, false)?;
        let previous = current_record(&self.root.path)?;
        // A malformed or incomplete current selection is an explicit recovery problem;
        // publication never masks it by choosing a newer directory.
        if previous.is_some() {
            resolve(&self.root.path)?;
        }
        let id: [u8; 32] = Sha256::digest(bytes).into();
        let destination = self.root.path.join("sets").join(hex(&id));
        if destination.exists() {
            select(&self.root.path, id)?;
        } else {
            let staging = self.root.path.join(unique("stage"));
            file_identity::create_dir(&staging)?;
            for (index, part) in manifest.parts.iter().enumerate() {
                copy_part(input, &staging, part, index, step)?;
            }
            let path = staging.join("manifest.txt");
            let mut file = file_identity::create_file_exclusive(&path)?;
            file.write_all(bytes)?;
            step(Phase::ManifestWritten)?;
            file_identity::set_mode(&path, 0o400)?;
            file.sync_all()?;
            step(Phase::ManifestSynced)?;
            // A directory cannot be renamed on Windows while a file inside it
            // is open, so the staged manifest handle closes here; harmless on Unix.
            drop(file);
            freeze_directories(&staging)?;
            if !same(&input_identity, &file_identity::snapshot_path(input)?) {
                return Err(BuildSetError::OwnershipChanged);
            }
            self.check()?;
            // Darwin requires the moved directory to remain writable for rename.
            // Only this held publication owner writes sets. The target is never replaced.
            fs::rename(&staging, &destination)?;
            file_identity::set_mode(&destination, 0o500)?;
            file_identity::sync_directory(&destination)?;
            verify_parts(&destination, &manifest, true)?;
            file_identity::sync_directory(&self.root.path.join("sets"))?;
            step(Phase::SetPlaced)?;
        }
        let path = self.root.path.join(unique("current"));
        let mut file = file_identity::create_file_exclusive(&path)?;
        file.write_all(format!("DF-CURRENT-BUILD-V1\n{}\n", hex(&id)).as_bytes())?;
        file_identity::set_mode(&path, 0o400)?;
        file.sync_all()?;
        // The reference is renamed below; its handle closes here for the same
        // Windows reason as the staged manifest above.
        drop(file);
        self.check()?;
        if !reference_unchanged(&previous, &current_record(&self.root.path)?) {
            return Err(BuildSetError::OwnershipChanged);
        }
        step(Phase::BeforeReferenceRename)?;
        self.check()?;
        if !reference_unchanged(&previous, &current_record(&self.root.path)?) {
            return Err(BuildSetError::OwnershipChanged);
        }
        // This rename is the sole visibility point. Nothing below it reports rejection
        // or attempts rollback without classifying the complete observed reference.
        if fs::rename(path, self.root.path.join("current")).is_err() {
            return Err(self.after_visibility(id, &previous));
        }
        let completed: Result<()> = (|| {
            step(Phase::AfterReferenceRename)?;
            file_identity::sync_directory(&self.root.path)?;
            step(Phase::AfterReferenceSync)?;
            self.check()?;
            Ok(())
        })();
        if completed.is_err() {
            return Err(self.after_visibility(id, &previous));
        }
        Ok(id)
    }

    fn after_visibility(&self, candidate: [u8; 32], previous: &Option<Reference>) -> BuildSetError {
        let previous_id = previous
            .as_ref()
            .and_then(|record| reference_id(&record.bytes).ok());
        let visible = match resolve(&self.root.path) {
            Ok(selected) if selected.id == candidate => VisibleSet::Candidate,
            Ok(selected) if Some(selected.id) == previous_id => VisibleSet::Previous,
            Ok(_) => VisibleSet::OtherComplete,
            Err(_) => VisibleSet::Unavailable,
        };
        BuildSetError::AfterVisibility(visible)
    }
}

fn copy_part(
    input: &Path,
    staging: &Path,
    part: &Part,
    index: usize,
    step: &mut dyn FnMut(Phase) -> Result<()>,
) -> Result<()> {
    let source = inside(input, &part.path)?;
    let (mut reader, before) = open_regular(&source, role_limit(part)?)?;
    let target = staging.join(&part.path);
    let parent = target.parent().ok_or(BuildSetError::UnsafePath)?;
    file_identity::create_dir_all(parent)?;
    let mut writer = file_identity::create_file_exclusive(&target)?;
    step(Phase::FileCreated(index))?;
    let mut hash = Sha256::new();
    let mut len = 0_u64;
    let mut buffer = [0; 16 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        len += read as u64;
        if len > part.len {
            return Err(BuildSetError::DigestMismatch);
        }
        writer.write_all(&buffer[..read])?;
        hash.update(&buffer[..read]);
    }
    finish_read(&source, &reader, &before)?;
    let digest: [u8; 32] = hash.finalize().into();
    if len != part.len || digest != part.digest {
        return Err(BuildSetError::DigestMismatch);
    }
    step(Phase::FileWritten(index))?;
    file_identity::set_mode(&target, if part.role == "native" { 0o500 } else { 0o400 })?;
    writer.sync_all()?;
    step(Phase::FileSynced(index))?;
    Ok(())
}

fn freeze_directories(root: &Path) -> Result<()> {
    let mut pending = vec![root.to_path_buf()];
    let mut directories = Vec::new();
    while let Some(directory) = pending.pop() {
        if directories.len() > TREE_LIMIT {
            return Err(BuildSetError::Limit);
        }
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            if file_identity::snapshot_path(&path)?.is_dir() {
                pending.push(path);
            }
        }
        directories.push(directory);
    }
    for path in directories.into_iter().rev() {
        if path != root {
            file_identity::set_mode(&path, 0o500)?;
        }
        file_identity::sync_directory(&path)?;
    }
    Ok(())
}

struct Reference {
    bytes: Vec<u8>,
    identity: FileIdentity,
}

fn reference_unchanged(expected: &Option<Reference>, observed: &Option<Reference>) -> bool {
    match (expected, observed) {
        (None, None) => true,
        (Some(a), Some(b)) => a.bytes == b.bytes && same(&a.identity, &b.identity),
        _ => false,
    }
}

fn current_record(root: &Path) -> Result<Option<Reference>> {
    let path = root.join("current");
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            let (mut file, identity) = open_regular(&path, REFERENCE_LIMIT as u64)?;
            let root_identity = file_identity::snapshot_path(root)?;
            if identity.owner() != root_identity.owner()
                || identity.volume() != root_identity.volume()
                || !identity.mode_equals(0o400)
            {
                return Err(BuildSetError::OwnershipChanged);
            }
            let mut bytes = Vec::new();
            (&mut file)
                .take(REFERENCE_LIMIT as u64 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > REFERENCE_LIMIT {
                return Err(BuildSetError::Limit);
            }
            finish_read(&path, &file, &identity)?;
            Ok(Some(Reference { bytes, identity }))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn resolve(root: &Path) -> Result<SelectedSet> {
    let owner = Root::open(root)?;
    let record = current_record(root)?.ok_or(BuildSetError::InvalidFormat)?;
    let id = reference_id(&record.bytes)?;
    let selected = select(root, id)?;
    owner.check()?;
    Ok(selected)
}

fn reference_id(bytes: &[u8]) -> Result<[u8; 32]> {
    let mut lines = super::text(bytes, REFERENCE_LIMIT)?.lines();
    if lines.next() != Some("DF-CURRENT-BUILD-V1") {
        return Err(BuildSetError::InvalidFormat);
    }
    let id = parse_digest(lines.next().ok_or(BuildSetError::InvalidFormat)?)?;
    if lines.next().is_some() {
        return Err(BuildSetError::InvalidFormat);
    }
    Ok(id)
}

pub(super) fn select(root: &Path, id: [u8; 32]) -> Result<SelectedSet> {
    let owner = Root::open(root)?;
    let sets = root.join("sets");
    let sets_identity = canonical_directory(&sets, 0o700)?;
    if sets_identity.volume() != owner.identity.volume()
        || sets_identity.owner() != owner.identity.owner()
    {
        return Err(BuildSetError::UnsafePath);
    }
    let directory = sets.join(hex(&id));
    let directory_identity = canonical_directory(&directory, 0o500)?;
    if directory_identity.volume() != owner.identity.volume()
        || directory_identity.owner() != owner.identity.owner()
    {
        return Err(BuildSetError::UnsafePath);
    }
    let bytes = read_external(&directory.join("manifest.txt"), MANIFEST_LIMIT)?;
    let observed: [u8; 32] = Sha256::digest(&bytes).into();
    if observed != id {
        return Err(BuildSetError::DigestMismatch);
    }
    let manifest = Manifest::parse(&bytes)?;
    verify_parts(&directory, &manifest, true)?;
    owner.check()?;
    if !same(
        &directory_identity,
        &file_identity::snapshot_path(&directory)?,
    ) {
        return Err(BuildSetError::OwnershipChanged);
    }
    Ok(SelectedSet {
        root: root.into(),
        id,
        directory,
        manifest,
        manifest_bytes: bytes,
    })
}

pub(super) fn read_checked(root: &Path, part: &Part) -> Result<Vec<u8>> {
    let bytes = read_external(&inside(root, &part.path)?, role_limit(part)? as usize)?;
    let observed: [u8; 32] = Sha256::digest(&bytes).into();
    if bytes.len() as u64 != part.len || observed != part.digest {
        return Err(BuildSetError::DigestMismatch);
    }
    Ok(bytes)
}
