//! Perception: what a character sees on the screen, and what they are thinking about. The players
//! and the characters of the scene can all see what is on screen, so the host feeds the picture
//! that is shown (with what the scan knows of it) and the character's own state through the
//! language model, and the model tells what the character sees and what is on their mind. The API
//! is standard and story-neutral: nothing here knows a story, a place or a painter.
//!
//! - `POST /dev/persona/perceive` (`persona perceive <who> ..`): one character. The model is given
//!   the picture itself as an image (the Responses backend sends it as an `input_image`, the Codex
//!   backend as an `--image`), the scan's regions with each person only as the character knows
//!   them, the persona, state, what they carry, what they know, where they are, the things in sight
//!   and the last lines of the scene. The answer is one closed object: `sees` (each with a region
//!   of the scan and an item of the place or of what they carry, both checked), `thinking_about`
//!   (the inner thought, first person, in their own voice), `attention` (a region or an item,
//!   checked), an optional `feeling` (an emotion of the campaign's list and its change, which goes
//!   into the state only through the line pipeline's state path) and `wants`. `vision` in the
//!   answer says whether the picture itself was given; `vision: false` (asked for, or the picture
//!   is not a readable library picture) is an answer from the scan's layout and the scene's words
//!   alone, marked so, never presented as sight.
//! - `POST /dev/persona/perceive/scene` (`persona perceive scene ..`): the same for a list of
//!   characters, or for everyone in the picture's cast who has a persona, a bounded number at a
//!   time, each from their own point of view. A character who is not in the picture sees what the
//!   screen shows a viewer (`viewpoint`). One character's refusal is their row's and stops no one;
//!   nothing of one character's answer is in another's facts.
//! - `GET /dev/persona/thoughts` (`persona thoughts <who>`): the thoughts a character kept. Each
//!   answer is kept as a memory of kind `thought` (`world memory`), owned by the character.
//!
//! The model judges what is seen and thought; code checks every id and number against the lists it
//! offered, and an answer that fails a check changes nothing and is asked once more with why
//! (`answer`). A name the character has not learned is never in the facts, and a handle that comes
//! back in an answer is put as how the person looks, or the answer is refused (`facts`).

mod answer;
mod attending;
pub(super) mod cli;
mod facts;
pub mod request;
pub mod routes;
mod screen;
mod thoughts;

pub(crate) use self::attending::attending;
pub(crate) use self::routes::router_with;
