//! The range checks a scenario file passes when it is read, with the line and column of the value
//! that breaks one.
//!
//! The rules are the ones `checks` applies to a live change, so a fixture that breaks a range is
//! refused with the same words as the command line would use. This module only walks the fixture
//! and finds where in the text a value is; it never changes a value and never draws.

use crate::checks;
use crate::fixture::{
    Camera, Choice, ElementSource, Fixture, FixtureError, ImageRef, PartyMember, Slide,
};
use crate::limits::{MAXIMUM_CAPTION_CHARACTERS, MAXIMUM_PARTY_MEMBERS, MAXIMUM_PIVOT_PERCENT};
use crate::scene::SceneError;

/// One step into the JSON of a fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Key(&'static str),
    Index(usize),
}

/// A value that is out of range, where it is and why.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Problem {
    path: Vec<Step>,
    error: SceneError,
}

impl Problem {
    fn at(path: &[Step], error: SceneError) -> Self {
        Self {
            path: path.to_vec(),
            error,
        }
    }

    fn new(path: &[Step], last: Step, error: SceneError) -> Self {
        let mut path = path.to_vec();
        path.push(last);
        Self { path, error }
    }

    fn path_text(&self) -> String {
        let mut text = String::new();
        for step in &self.path {
            match step {
                Step::Key(key) => {
                    if !text.is_empty() {
                        text.push('.');
                    }
                    text.push_str(key);
                }
                Step::Index(index) => text.push_str(&format!("[{index}]")),
            }
        }
        text
    }

    fn into_error(self, text: &str) -> FixtureError {
        let (line, column) = locate(text, &self.path);
        FixtureError {
            line,
            column,
            message: format!("{}: {}", self.path_text(), self.error),
        }
    }
}

const KEY_ROTATION: Step = Step::Key("rotation_degrees");
const KEY_SLIDE: Step = Step::Key("slide");

/// What a failed text check was about, so the problem points at the right key.
fn push(found: &mut Vec<Problem>, at: &[Step], key: &'static str, result: Result<(), SceneError>) {
    if let Err(error) = result {
        found.push(Problem::new(at, Step::Key(key), error));
    }
}

/// A number the host sets (the serial of a move, when it began and how long ago) must be left out
/// of a file, so the page cannot mistake a file's move for one the host numbered.
fn host_owned(found: &mut Vec<Problem>, at: &[Step], fields: [(&'static str, u64); 3]) {
    for (key, value) in fields {
        if value != 0 {
            found.push(Problem::new(at, Step::Key(key), SceneError::HostOwned(key)));
        }
    }
}

fn slide_problems(found: &mut Vec<Problem>, at: &[Step], slide: &Slide, may_slide: bool) {
    let here: Vec<Step> = at.iter().copied().chain([KEY_SLIDE]).collect();
    host_owned(
        found,
        &here,
        [
            ("serial", slide.serial),
            ("began_ms", slide.began_ms),
            ("elapsed_ms", slide.elapsed_ms),
        ],
    );
    let error = if may_slide {
        checks::slide(slide).err()
    } else {
        Some(SceneError::SlideNeedsTheCard)
    };
    if let Some(error) = error {
        found.push(Problem::at(&here, error));
    }
}

fn image_problems(found: &mut Vec<Problem>, at: &[Step], image: &ImageRef, may_slide: bool) {
    push(
        found,
        at,
        "media",
        checks::labelled("a media name", &image.media),
    );
    if let Some(caption) = &image.caption {
        push(
            found,
            at,
            "caption",
            checks::labelled("a picture caption", caption),
        );
    }
    if let Err(error) = checks::rotation(image.rotation_degrees) {
        found.push(Problem::new(at, KEY_ROTATION, error));
    }
    if let Some(slide) = &image.slide {
        slide_problems(found, at, slide, may_slide);
    }
}

fn camera_problems(found: &mut Vec<Problem>, camera: &Camera, elements: &ElementSource) {
    let at = [Step::Key("tv"), Step::Key("camera")];
    host_owned(
        found,
        &at,
        [
            ("serial", camera.serial),
            ("began_ms", camera.began_ms),
            ("elapsed_ms", camera.elapsed_ms),
        ],
    );
    if let Err(error) = checks::camera(
        camera.target.as_deref(),
        camera.zoom_percent,
        camera.duration_ms,
    ) {
        let key = match error {
            SceneError::BadZoom(_) => "zoom_percent",
            SceneError::BadCameraTime(_) => "duration_ms",
            _ => "target",
        };
        found.push(Problem::new(&at, Step::Key(key), error));
        return;
    }
    if let (Some(target), ElementSource::Inline(inline)) = (camera.target.as_deref(), elements)
        && !inline.iter().any(|element| element.id == target)
    {
        found.push(Problem::new(
            &at,
            Step::Key("target"),
            SceneError::UnknownCameraTarget(target.to_owned()),
        ));
    }
}

fn member_problems(found: &mut Vec<Problem>, index: usize, member: &PartyMember) {
    let at = [Step::Key("tv"), Step::Key("party"), Step::Index(index)];
    push(
        found,
        &at,
        "name",
        checks::labelled("a party member's name", &member.name),
    );
    if member.maximum_hit_points == 0 {
        found.push(Problem::new(
            &at,
            Step::Key("maximum_hit_points"),
            SceneError::BadMaximum,
        ));
    }
    for (position, condition) in member.conditions.iter().enumerate() {
        let list = [at[0], at[1], at[2], Step::Key("conditions")];
        if let Err(error) = checks::labelled("a condition", condition) {
            found.push(Problem::new(&list, Step::Index(position), error));
        }
    }
}

fn choice_problems(found: &mut Vec<Problem>, list: &[Step], choices: &[Choice]) {
    if let Err(error) = checks::count(choices.len()) {
        found.push(Problem::at(list, error));
    }
    for (index, choice) in choices.iter().enumerate() {
        let at: Vec<Step> = list.iter().copied().chain([Step::Index(index)]).collect();
        push(
            found,
            &at,
            "label",
            checks::labelled("a choice label", &choice.label),
        );
        if let Some(explain) = &choice.explain {
            push(
                found,
                &at,
                "explain",
                checks::labelled("a choice explanation", explain),
            );
        }
    }
}

fn phone_problems(found: &mut Vec<Problem>, fixture: &Fixture) {
    let phone = [Step::Key("phone")];
    choice_problems(
        found,
        &[phone[0], Step::Key("suggested")],
        &fixture.phone.suggested,
    );
    for (index, group) in fixture.phone.groups.iter().enumerate() {
        let at = [phone[0], Step::Key("groups"), Step::Index(index)];
        push(
            found,
            &at,
            "name",
            checks::labelled("a group name", &group.name),
        );
        let list = [at[0], at[1], at[2], Step::Key("choices")];
        choice_problems(found, &list, &group.choices);
    }
    let unavailable = [phone[0], Step::Key("unavailable")];
    if let Err(error) = checks::count(fixture.phone.unavailable.len()) {
        found.push(Problem::at(&unavailable, error));
    }
    for (index, choice) in fixture.phone.unavailable.iter().enumerate() {
        let at = [unavailable[0], unavailable[1], Step::Index(index)];
        push(
            found,
            &at,
            "label",
            checks::labelled("a choice label", &choice.label),
        );
        push(
            found,
            &at,
            "reason",
            checks::labelled("a reason", &choice.reason),
        );
    }
}

fn tv_problems(found: &mut Vec<Problem>, fixture: &Fixture) {
    let tv = [Step::Key("tv")];
    if let Some(caption) = &fixture.tv.caption {
        let at = [tv[0], Step::Key("caption")];
        push(
            found,
            &at,
            "text",
            checks::text("the caption", &caption.text, MAXIMUM_CAPTION_CHARACTERS),
        );
        if let Some(speaker) = &caption.speaker {
            push(
                found,
                &at,
                "speaker",
                checks::labelled("the speaker", speaker),
            );
        }
    }
    if fixture.tv.party.len() > MAXIMUM_PARTY_MEMBERS {
        found.push(Problem::new(
            &tv,
            Step::Key("party"),
            SceneError::TooManyMembers,
        ));
    }
    for (index, member) in fixture.tv.party.iter().enumerate() {
        member_problems(found, index, member);
    }
    if let Some(camera) = &fixture.tv.camera {
        camera_problems(found, camera, &fixture.tv.elements);
    }
    if let ElementSource::Inline(elements) = &fixture.tv.elements {
        for (index, element) in elements.iter().enumerate() {
            let at = [tv[0], Step::Key("elements"), Step::Index(index)];
            if let Err(error) = checks::rotation(element.rotation_degrees) {
                found.push(Problem::new(&at, KEY_ROTATION, error));
            }
            for (key, percent) in [
                ("x_percent", element.pivot.x_percent),
                ("y_percent", element.pivot.y_percent),
            ] {
                if percent > MAXIMUM_PIVOT_PERCENT {
                    let pivot = [at[0], at[1], at[2], Step::Key("pivot")];
                    found.push(Problem::new(
                        &pivot,
                        Step::Key(key),
                        SceneError::BadPivot(percent),
                    ));
                }
            }
        }
    }
}

fn problems(fixture: &Fixture) -> Vec<Problem> {
    let mut found = Vec::new();
    let tv = [Step::Key("tv")];
    let phone = [Step::Key("phone")];
    for (parent, key, image, may_slide) in [
        (&tv, "backdrop", &fixture.tv.backdrop, false),
        (&tv, "card", &fixture.tv.card, true),
        (&phone, "image", &fixture.phone.image, false),
    ] {
        if let Some(image) = image {
            let at = [parent[0], Step::Key(key)];
            image_problems(&mut found, &at, image, may_slide);
        }
    }
    tv_problems(&mut found, fixture);
    phone_problems(&mut found, fixture);
    if let Some(track) = &fixture.music {
        let at = [Step::Key("music")];
        push(
            &mut found,
            &at,
            "media",
            checks::labelled("a media name", &track.media),
        );
        push(&mut found, &at, "volume", checks::volume(track.volume));
    }
    found
}

/// The mistake nearest the top of `text`, when the fixture it was read from has a value out of
/// range. Columns count characters, as `serde_error` does, so both kinds of mistake point the same
/// way.
pub fn first_problem(fixture: &Fixture, text: &str) -> Option<FixtureError> {
    problems(fixture)
        .into_iter()
        .map(|problem| problem.into_error(text))
        .min_by_key(|error| (error.line, error.column))
}

/// A mistake `serde_json` found, with its column in characters. `serde_json` counts bytes, which
/// puts the mark in another place on a line with a non-ASCII character before it.
pub fn serde_error(text: &str, error: &serde_json::Error) -> FixtureError {
    let line = error.line();
    let column = text
        .lines()
        .nth(line.saturating_sub(1))
        .map_or(error.column(), |content| {
            let bytes = error.column().saturating_sub(1).min(content.len());
            let mut end = bytes;
            while !content.is_char_boundary(end) {
                end -= 1;
            }
            content[..end].chars().count() + 1
        });
    let full = error.to_string();
    let message = full
        .rsplit_once(" at line ")
        .map_or(full.as_str(), |(words, _)| words)
        .to_owned();
    FixtureError {
        line,
        column,
        message,
    }
}

/// Where the value at `path` starts in `text`, as a line and a column (both counted from 1). A path
/// that is not found points at the deepest place that is, and at the start of the file at worst.
fn locate(text: &str, path: &[Step]) -> (usize, usize) {
    let mut keep = path.len();
    let offset = loop {
        if let Some(offset) = Scanner::new(text).find(&path[..keep]) {
            break offset;
        }
        if keep == 0 {
            break 0;
        }
        keep -= 1;
    };
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map_or(0, |last| last.chars().count())
        + 1;
    (line, column)
}

/// A reader of JSON text that has already been read once, so it finds a place and does not
/// complain about what it meets.
struct Scanner<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            position: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.position += 1;
        }
    }

    /// Reads a string and returns what is between its quotes, escapes left as written.
    fn string(&mut self) -> Option<&'a [u8]> {
        if self.peek()? != b'"' {
            return None;
        }
        self.position += 1;
        let start = self.position;
        while let Some(byte) = self.peek() {
            match byte {
                b'\\' => self.position += 2,
                b'"' => {
                    let inside = &self.bytes[start..self.position];
                    self.position += 1;
                    return Some(inside);
                }
                _ => self.position += 1,
            }
        }
        None
    }

    fn skip_value(&mut self) -> Option<()> {
        self.skip_space();
        match self.peek()? {
            b'"' => self.string().map(|_| ()),
            b'{' | b'[' => {
                let mut depth = 0_usize;
                while let Some(byte) = self.peek() {
                    match byte {
                        b'"' => {
                            self.string()?;
                            continue;
                        }
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => depth -= 1,
                        _ => {}
                    }
                    self.position += 1;
                    if depth == 0 {
                        return Some(());
                    }
                }
                None
            }
            _ => {
                while self
                    .peek()
                    .is_some_and(|byte| !matches!(byte, b',' | b'}' | b']'))
                {
                    self.position += 1;
                }
                Some(())
            }
        }
    }

    /// The byte offset where the value at `path` starts.
    fn find(&mut self, path: &[Step]) -> Option<usize> {
        self.skip_space();
        let Some((step, rest)) = path.split_first() else {
            return Some(self.position);
        };
        match (step, self.peek()?) {
            (Step::Key(wanted), b'{') => {
                self.position += 1;
                loop {
                    self.skip_space();
                    let name = self.string()?;
                    self.skip_space();
                    if self.peek()? != b':' {
                        return None;
                    }
                    self.position += 1;
                    if name == wanted.as_bytes() {
                        return self.find(rest);
                    }
                    self.skip_value()?;
                    self.skip_space();
                    if self.peek()? != b',' {
                        return None;
                    }
                    self.position += 1;
                }
            }
            (Step::Index(wanted), b'[') => {
                self.position += 1;
                for index in 0.. {
                    self.skip_space();
                    if index == *wanted {
                        return self.find(rest);
                    }
                    self.skip_value()?;
                    self.skip_space();
                    if self.peek()? != b',' {
                        return None;
                    }
                    self.position += 1;
                }
                None
            }
            _ => None,
        }
    }
}
