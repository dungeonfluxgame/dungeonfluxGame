//! The fixture's one build-set boundary. Labels are provenance, never approval or access.
mod filesystem;
#[cfg(test)]
mod tests;

use crate::file_identity;
use df_types::{BuildIdentity, BuildIdentityError, BuildRevision, RevisionLabel};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error as StdError,
    ffi::OsString,
    fmt, io,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const MANIFEST_LIMIT: usize = 64 * 1024;
const IDENTITY_LIMIT: usize = 1024;
const ASSET_LIMIT: usize = 64;
const PART_LIMIT: usize = 8 + ASSET_LIMIT;
const TOTAL_LIMIT: u64 = 256 * 1024 * 1024;
const WEB_LIMIT: u64 = 128 * 1024 * 1024;
const REVISIONS: [BuildRevision; 5] = [
    BuildRevision::Source,
    BuildRevision::Native,
    BuildRevision::Wasm,
    BuildRevision::Configuration,
    BuildRevision::Content,
];
const LABELS: [&str; 5] = ["source", "native", "wasm", "configuration", "content"];
const REQUIRED: [(&str, &str, u64); 8] = [
    ("native", "native/df-transport-fixture", 64 * 1024 * 1024),
    ("compiler-wasm", "compiler.wasm", 32 * 1024 * 1024),
    ("bound-wasm", "web/df_tools_bg.wasm", 32 * 1024 * 1024),
    ("loader", "web/df_tools.js", 1024 * 1024),
    ("loader-declarations", "web/df_tools.d.ts", 1024 * 1024),
    (
        "wasm-declarations",
        "web/df_tools_bg.wasm.d.ts",
        1024 * 1024,
    ),
    ("configuration", "web/configuration.bin", 1024 * 1024),
    ("content", "web/content-manifest.txt", MANIFEST_LIMIT as u64),
];

#[derive(Debug)]
pub enum BuildSetError {
    Arguments,
    InvalidIdentity(BuildIdentityError),
    RevisionMismatch(BuildRevision),
    InvalidFormat,
    Limit,
    ComponentClosure,
    DigestMismatch,
    UnsafePath,
    OwnershipChanged,
    PublisherBusy,
    NativeMismatch,
    Io(io::Error),
    /// Visibility was attempted. This classification comes from a complete reread,
    /// not a rollback or a claim that the directory sync survived power loss.
    AfterVisibility(VisibleSet),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibleSet {
    Candidate,
    Previous,
    OtherComplete,
    Unavailable,
}

impl fmt::Display for BuildSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arguments => f.write_str("invalid complete build-set command arguments"),
            Self::InvalidIdentity(_) => f.write_str("invalid required build identity"),
            Self::RevisionMismatch(_) => f.write_str("build revision mismatch"),
            Self::InvalidFormat => f.write_str("invalid bounded build-set record"),
            Self::Limit => f.write_str("build-set input exceeds its bound"),
            Self::ComponentClosure => f.write_str("incomplete or duplicate build-set closure"),
            Self::DigestMismatch => f.write_str("build-set bytes do not match sealed manifest"),
            Self::UnsafePath => f.write_str("unsafe build-set filesystem path"),
            Self::OwnershipChanged => f.write_str("build-set filesystem ownership changed"),
            Self::PublisherBusy => f.write_str("build-set publication owner is unavailable"),
            Self::NativeMismatch => {
                f.write_str("running native executable is outside selected set")
            }
            Self::Io(_) => f.write_str("build-set filesystem operation failed"),
            Self::AfterVisibility(visible) => {
                write!(
                    f,
                    "build-set durability is unknown after visibility; observed={visible:?}"
                )
            }
        }
    }
}

impl StdError for BuildSetError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for BuildSetError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

type Result<T> = std::result::Result<T, BuildSetError>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Part {
    role: String,
    path: String,
    length_bytes: u64,
    digest: [u8; 32],
}

#[derive(Clone, Debug)]
struct Manifest {
    identity: BuildIdentity,
    parts: Vec<Part>,
}

fn text(bytes: &[u8], limit: usize) -> Result<&str> {
    if bytes.is_empty() || bytes.len() > limit {
        return Err(BuildSetError::Limit);
    }
    let value = std::str::from_utf8(bytes).map_err(|_| BuildSetError::InvalidFormat)?;
    if !value.is_ascii() || !value.ends_with('\n') || value.contains('\r') {
        return Err(BuildSetError::InvalidFormat);
    }
    Ok(value)
}

fn identity_lines<'a>(lines: &mut impl Iterator<Item = &'a str>) -> Result<BuildIdentity> {
    let mut values = Vec::with_capacity(5);
    for label in LABELS {
        let prefix = format!("{label}=");
        let value = lines
            .next()
            .and_then(|line| line.strip_prefix(&prefix))
            .ok_or(BuildSetError::InvalidFormat)?;
        values.push(value);
    }
    match values.as_slice() {
        [source, native, wasm, configuration, content] => BuildIdentity::new(
            Some(source),
            Some(native),
            Some(wasm),
            Some(configuration),
            Some(content),
        )
        .map_err(BuildSetError::InvalidIdentity),
        _ => Err(BuildSetError::InvalidFormat),
    }
}

fn parse_identity(bytes: &[u8]) -> Result<BuildIdentity> {
    let mut lines = text(bytes, IDENTITY_LIMIT)?.lines();
    if lines.next() != Some("DF-BUILD-IDENTITY-V1") {
        return Err(BuildSetError::InvalidFormat);
    }
    let identity = identity_lines(&mut lines)?;
    if lines.next().is_some() {
        return Err(BuildSetError::InvalidFormat);
    }
    Ok(identity)
}

fn compare_identity(expected: &BuildIdentity, observed: &BuildIdentity) -> Result<()> {
    for revision in REVISIONS {
        if expected.revision(revision) != observed.revision(revision) {
            return Err(BuildSetError::RevisionMismatch(revision));
        }
    }
    Ok(())
}

fn safe_relative(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 240
        || value.split('/').count() > 8
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
        || value
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
        || Path::new(value)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(BuildSetError::UnsafePath);
    }
    Ok(())
}

fn hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn parse_digest(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(BuildSetError::InvalidFormat);
    }
    let mut digest = [0; 32];
    for (slot, pair) in digest
        .iter_mut()
        .zip(value.as_bytes().as_chunks::<2>().0.iter())
    {
        let pair = std::str::from_utf8(pair).map_err(|_| BuildSetError::InvalidFormat)?;
        *slot = u8::from_str_radix(pair, 16).map_err(|_| BuildSetError::InvalidFormat)?;
    }
    Ok(digest)
}

fn part_line(line: &str) -> Result<Part> {
    let fields: Vec<_> = line.split('\t').take(5).collect();
    match fields.as_slice() {
        [role, path, length, digest] => {
            safe_relative(path)?;
            let length_bytes = length
                .parse::<u64>()
                .map_err(|_| BuildSetError::InvalidFormat)?;
            if length_bytes == 0
                || length_bytes.to_string() != *fields.get(2).ok_or(BuildSetError::InvalidFormat)?
            {
                return Err(BuildSetError::InvalidFormat);
            }
            Ok(Part {
                role: (*role).into(),
                path: (*path).into(),
                length_bytes,
                digest: parse_digest(digest)?,
            })
        }
        _ => Err(BuildSetError::InvalidFormat),
    }
}

fn role_limit_bytes(part: &Part) -> Result<u64> {
    if let Some((_, path, limit_bytes)) = REQUIRED.iter().find(|(role, _, _)| *role == part.role) {
        if *path != part.path {
            return Err(BuildSetError::ComponentClosure);
        }
        return Ok(*limit_bytes);
    }
    let id = part
        .role
        .strip_prefix("asset:")
        .ok_or(BuildSetError::ComponentClosure)?;
    RevisionLabel::new(Some(id)).map_err(|_| BuildSetError::ComponentClosure)?;
    if !part.path.starts_with("web/assets/") {
        return Err(BuildSetError::ComponentClosure);
    }
    Ok(16 * 1024 * 1024)
}

impl Manifest {
    fn validate(&self) -> Result<()> {
        if self.parts.len() < REQUIRED.len() || self.parts.len() > PART_LIMIT {
            return Err(BuildSetError::ComponentClosure);
        }
        let mut roles = BTreeSet::new();
        let mut paths = BTreeSet::new();
        let mut total_bytes = 0_u64;
        let mut web_bytes = 0_u64;
        for part in &self.parts {
            safe_relative(&part.path)?;
            if !roles.insert(part.role.as_str()) || !paths.insert(part.path.as_str()) {
                return Err(BuildSetError::ComponentClosure);
            }
            if part.length_bytes == 0 || part.length_bytes > role_limit_bytes(part)? {
                return Err(BuildSetError::Limit);
            }
            total_bytes = total_bytes
                .checked_add(part.length_bytes)
                .ok_or(BuildSetError::Limit)?;
            if part.path.starts_with("web/") {
                web_bytes += part.length_bytes;
            }
        }
        if total_bytes > TOTAL_LIMIT || web_bytes > WEB_LIMIT {
            return Err(BuildSetError::Limit);
        }
        if REQUIRED.iter().any(|(role, _, _)| !roles.contains(role)) {
            return Err(BuildSetError::ComponentClosure);
        }
        Ok(())
    }

    fn parse(bytes: &[u8]) -> Result<Self> {
        let mut lines = text(bytes, MANIFEST_LIMIT)?.lines();
        if lines.next() != Some("DF-BUILD-SET-V1") {
            return Err(BuildSetError::InvalidFormat);
        }
        let identity = identity_lines(&mut lines)?;
        let count = lines
            .next()
            .and_then(|line| line.strip_prefix("parts="))
            .ok_or(BuildSetError::InvalidFormat)?
            .parse::<usize>()
            .map_err(|_| BuildSetError::InvalidFormat)?;
        if count > PART_LIMIT {
            return Err(BuildSetError::Limit);
        }
        let mut parts = Vec::with_capacity(count);
        for _ in 0..count {
            parts.push(part_line(
                lines.next().ok_or(BuildSetError::InvalidFormat)?,
            )?);
        }
        if lines.next().is_some() {
            return Err(BuildSetError::InvalidFormat);
        }
        let manifest = Self { identity, parts };
        manifest.validate()?;
        if manifest.serialize()? != bytes {
            return Err(BuildSetError::InvalidFormat);
        }
        Ok(manifest)
    }

    fn serialize(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut value = String::from("DF-BUILD-SET-V1\n");
        for (label, revision) in LABELS.into_iter().zip(REVISIONS) {
            value.push_str(&format!(
                "{label}={}\n",
                self.identity.revision(revision).as_str()
            ));
        }
        value.push_str(&format!("parts={}\n", self.parts.len()));
        for part in &self.parts {
            value.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                part.role,
                part.path,
                part.length_bytes,
                hex(&part.digest)
            ));
        }
        if value.len() > MANIFEST_LIMIT {
            return Err(BuildSetError::Limit);
        }
        Ok(value.into_bytes())
    }
}

/// Content is an explicit resolved byte closure. It grants no media access/rights.
/// An empty closure is valid; required assets cannot be omitted or replaced by a fallback.
fn content_parts(bytes: &[u8], identity: &BuildIdentity) -> Result<Vec<Part>> {
    let mut lines = text(bytes, MANIFEST_LIMIT)?.lines();
    let revision = format!(
        "revision={}",
        identity.revision(BuildRevision::Content).as_str()
    );
    if lines.next() != Some("DF-BUILD-CONTENT-V1") || lines.next() != Some(revision.as_str()) {
        return Err(BuildSetError::InvalidFormat);
    }
    let mut result = Vec::new();
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for line in lines {
        if result.len() == ASSET_LIMIT {
            return Err(BuildSetError::Limit);
        }
        let part = part_line(line)?;
        role_limit_bytes(&part)?;
        if !part.role.starts_with("asset:")
            || !ids.insert(part.role.clone())
            || !paths.insert(part.path.clone())
        {
            return Err(BuildSetError::ComponentClosure);
        }
        result.push(part);
    }
    Ok(result)
}

#[derive(Debug)]
struct SelectedSet {
    root: PathBuf,
    id: [u8; 32],
    directory: PathBuf,
    manifest: Manifest,
    manifest_bytes: Vec<u8>,
}

impl SelectedSet {
    fn verify_native(&self, executable: &Path, compiled_source: &str) -> Result<()> {
        verify_native(&self.manifest, executable, compiled_source)
    }
}

fn verify_native(manifest: &Manifest, executable: &Path, compiled_source: &str) -> Result<()> {
    if compiled_source == "unregistered-cargo-build"
        || manifest.identity.revision(BuildRevision::Source).as_str() != compiled_source
    {
        return Err(BuildSetError::NativeMismatch);
    }
    let native = manifest
        .parts
        .iter()
        .find(|part| part.role == "native")
        .ok_or(BuildSetError::ComponentClosure)?;
    if filesystem::observe_file(executable, role_limit_bytes(native)?)?
        != (native.length_bytes, native.digest)
    {
        return Err(BuildSetError::NativeMismatch);
    }
    Ok(())
}

/// Constructed only after complete byte validation and current-executable binding.
/// The serving scope keeps these bytes; later publication cannot replace them.
pub struct ReadyBuild {
    selected: SelectedSet,
    files: BTreeMap<String, bytes::Bytes>,
    pub(super) port: u16,
}

impl ReadyBuild {
    pub(super) fn set_id(&self) -> String {
        hex(&self.selected.id)
    }
    pub(super) fn web_root(&self) -> PathBuf {
        self.selected.directory.join("web")
    }
    pub(super) fn identity(&self) -> &BuildIdentity {
        &self.selected.manifest.identity
    }
    pub(super) fn asset(&self, id: &str, path: &str) -> Option<bytes::Bytes> {
        (id == self.set_id())
            .then(|| self.files.get(path).cloned())
            .flatten()
    }
}

fn ready(selected: SelectedSet, executable: &Path, source: &str, port: u16) -> Result<ReadyBuild> {
    if port == 0 {
        return Err(BuildSetError::Arguments);
    }
    selected.verify_native(executable, source)?;
    let mut files = BTreeMap::new();
    for part in &selected.manifest.parts {
        if let Some(path) = part.path.strip_prefix("web/") {
            let bytes = filesystem::read_checked(&selected.directory, part)?;
            files.insert(path.into(), bytes::Bytes::from(bytes));
        }
    }
    files.insert(
        "build-set.txt".into(),
        bytes::Bytes::copy_from_slice(&selected.manifest_bytes),
    );
    Ok(ReadyBuild {
        selected,
        files,
        port,
    })
}

pub enum CommandOutcome {
    Complete,
    Serve(Arc<ReadyBuild>),
}

fn path_argument(value: &OsString) -> Result<PathBuf> {
    let value = value.to_str().ok_or(BuildSetError::Arguments)?;
    if value.is_empty() || value.len() > 4096 {
        return Err(BuildSetError::Arguments);
    }
    Ok(value.into())
}

/// Explicit CLI settings; no library environment reads, default identities or raw mixed roots.
pub fn dispatch(arguments: &[OsString]) -> Result<CommandOutcome> {
    if arguments.len() > 5 {
        return Err(BuildSetError::Arguments);
    }
    for argument in arguments {
        path_argument(argument)?;
    }
    let args: Vec<_> = arguments
        .iter()
        .map(|x| x.to_str().ok_or(BuildSetError::Arguments))
        .collect::<Result<_>>()?;
    match args.as_slice() {
        ["--seal-build-set", input, expected, output] => {
            let input = Path::new(input);
            let expected = parse_identity(&filesystem::read_external(
                Path::new(expected),
                IDENTITY_LIMIT,
            )?)?;
            let manifest = filesystem::seal(input, &expected)?;
            filesystem::write_sealed(Path::new(output), &manifest.serialize()?)?;
            Ok(CommandOutcome::Complete)
        }
        ["--publish-build-set", root, input, manifest] => {
            let manifest = filesystem::read_external(Path::new(manifest), MANIFEST_LIMIT)?;
            verify_native(
                &Manifest::parse(&manifest)?,
                &std::env::current_exe()?,
                crate::BUILD_ID,
            )?;
            let owner = filesystem::Publisher::acquire(Path::new(root))?;
            let id = owner.publish(Path::new(input), &manifest, &mut |_| Ok(()))?;
            // This is CLI output, not an alternate runtime logging path.
            println!("build-set {} published", hex(&id));
            Ok(CommandOutcome::Complete)
        }
        ["--launch-build-set", root, port] => {
            let port = port.parse::<u16>().map_err(|_| BuildSetError::Arguments)?;
            if port == 0 {
                return Err(BuildSetError::Arguments);
            }
            let selected = filesystem::resolve(Path::new(root))?;
            let executable = selected.directory.join("native/df-transport-fixture");
            // The launcher holds no locks, unpublished state or serving futures
            // here: `resolve` retains only validated bytes, so handing over to
            // the selected executable unwinds nothing on either platform.
            let error = file_identity::launch_selected(
                &executable,
                &[
                    OsString::from("--serve-selected"),
                    selected.root.into_os_string(),
                    OsString::from(hex(&selected.id)),
                    OsString::from(port.to_string()),
                ],
            );
            Err(error.into())
        }
        ["--serve-selected", root, id, port] => {
            let selected = filesystem::select(Path::new(root), parse_digest(id)?)?;
            let port = port.parse::<u16>().map_err(|_| BuildSetError::Arguments)?;
            Ok(CommandOutcome::Serve(Arc::new(ready(
                selected,
                &std::env::current_exe()?,
                crate::BUILD_ID,
                port,
            )?)))
        }
        ["--inspect-build-set", root] => {
            println!(
                "build-set {} verified",
                hex(&filesystem::resolve(Path::new(root))?.id)
            );
            Ok(CommandOutcome::Complete)
        }
        _ => Err(BuildSetError::Arguments),
    }
}
