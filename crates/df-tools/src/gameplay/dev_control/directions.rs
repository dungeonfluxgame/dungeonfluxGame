//! Session-owner development directions. These never enter mechanical game state.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::gameplay) enum Target {
    Plate,
}

#[derive(Clone)]
struct Direction {
    text: String,
    keep: bool,
}

#[derive(Default)]
pub(in crate::gameplay) struct Directions {
    scene: String,
    targets: BTreeMap<Target, Direction>,
}

impl Directions {
    pub(in crate::gameplay) fn enter(&mut self, scene: &str) {
        if self.scene != scene {
            self.targets.clear();
            self.scene = scene.to_owned();
        }
    }

    pub(in crate::gameplay) fn set(&mut self, target: Target, text: String, keep: bool) {
        self.targets.insert(target, Direction { text, keep });
    }

    /// Consumed at admitted render dispatch; kept directions remain until clear or scene exit.
    pub(in crate::gameplay) fn take(&mut self, target: Target) -> Option<String> {
        let direction = self.targets.get(&target)?;
        if direction.keep {
            Some(direction.text.clone())
        } else {
            self.targets.remove(&target).map(|direction| direction.text)
        }
    }

    pub(in crate::gameplay) fn clear(&mut self) {
        self.targets.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_direction_is_cleared_after_one_use() {
        let mut directions = Directions::default();
        directions.enter("scene-a");
        directions.set(Target::Plate, "Draw the sword".to_owned(), false);
        assert_eq!(
            directions.take(Target::Plate).as_deref(),
            Some("Draw the sword")
        );
        assert_eq!(directions.take(Target::Plate), None);
    }

    #[test]
    fn kept_direction_lasts_until_clear_or_scene_exit() {
        let mut directions = Directions::default();
        directions.enter("scene-a");
        directions.set(Target::Plate, "Draw the sword".to_owned(), true);
        assert!(directions.take(Target::Plate).is_some());
        assert!(directions.take(Target::Plate).is_some());
        directions.clear();
        assert_eq!(directions.take(Target::Plate), None);
        directions.set(Target::Plate, "Draw the sword".to_owned(), true);
        directions.enter("scene-b");
        assert_eq!(directions.take(Target::Plate), None);
    }
}
