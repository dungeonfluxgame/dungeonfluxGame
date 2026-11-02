use crate::ecs::{Entities, Entity, Store};

#[test]
fn entities_are_distinct_alive_and_counted() {
    let mut entities = Entities::default();
    let a = entities.spawn();
    let b = entities.spawn();
    assert_ne!(a, b);
    assert!(entities.is_alive(a) && entities.is_alive(b));
    assert_eq!(entities.len(), 2);
    assert!(!entities.is_empty());
}

#[test]
fn a_removed_entity_is_not_alive_and_removing_it_again_does_nothing() {
    let mut entities = Entities::default();
    let a = entities.spawn();
    assert!(entities.despawn(a));
    assert!(!entities.is_alive(a));
    assert!(!entities.despawn(a));
    assert_eq!(entities.len(), 0);
}

#[test]
fn a_reused_index_gets_a_new_generation_so_a_stale_handle_never_reaches_the_new_entity() {
    let mut entities = Entities::default();
    let old = entities.spawn();
    entities.despawn(old);
    let new = entities.spawn();
    assert_ne!(old, new);
    assert!(!entities.is_alive(old));
    assert!(entities.is_alive(new));
    assert!(!entities.despawn(old), "the stale handle removes nothing");
    assert!(entities.is_alive(new));
}

#[test]
fn a_handle_that_was_never_handed_out_is_not_alive() {
    let mut other = Entities::default();
    let far = (0..5).map(|_| other.spawn()).last().expect("spawned");
    assert!(!Entities::default().is_alive(far));
}

fn store_with(items: &[(Entity, &'static str)]) -> Store<&'static str> {
    let mut store = Store::default();
    for (entity, item) in items {
        store.insert(*entity, *item);
    }
    store
}

#[test]
fn a_store_gives_back_what_it_was_given_and_replaces_on_a_second_insert() {
    let mut entities = Entities::default();
    let a = entities.spawn();
    let mut store = Store::default();
    assert_eq!(store.insert(a, "first"), None);
    assert_eq!(store.get(a), Some(&"first"));
    assert_eq!(store.insert(a, "second"), Some("first"));
    assert_eq!(store.get(a), Some(&"second"));
    assert_eq!(store.len(), 1);
    *store.get_mut(a).expect("has it") = "third";
    assert_eq!(store.get(a), Some(&"third"));
}

#[test]
fn removing_from_the_middle_keeps_every_other_component_reachable() {
    let mut entities = Entities::default();
    let [a, b, c] = [entities.spawn(), entities.spawn(), entities.spawn()];
    let mut store = store_with(&[(a, "a"), (b, "b"), (c, "c")]);
    assert_eq!(store.remove(a), Some("a"));
    assert_eq!(store.get(a), None);
    assert_eq!((store.get(b), store.get(c)), (Some(&"b"), Some(&"c")));
    assert_eq!(store.len(), 2);
    assert_eq!(store.remove(a), None);
    assert_eq!(store.remove(c), Some("c"));
    assert_eq!(store.get(b), Some(&"b"));
}

#[test]
fn a_store_does_not_answer_for_a_stale_handle_whose_index_was_reused() {
    let mut entities = Entities::default();
    let old = entities.spawn();
    let mut store = store_with(&[(old, "old")]);
    entities.despawn(old);
    let new = entities.spawn();
    assert_eq!(store.get(new), None);
    assert!(!store.contains(new));
    store.insert(new, "new");
    assert_eq!(
        store.get(old),
        None,
        "the old handle does not see the new component"
    );
    assert_eq!(store.get(new), Some(&"new"));
}

#[test]
fn iteration_visits_every_component_once_and_can_change_them() {
    let mut entities = Entities::default();
    let [a, b] = [entities.spawn(), entities.spawn()];
    let mut store = Store::default();
    store.insert(a, 1);
    store.insert(b, 2);
    for (_, item) in store.iter_mut() {
        *item *= 10;
    }
    let mut seen: Vec<(Entity, i32)> = store.iter().map(|(entity, item)| (entity, *item)).collect();
    seen.sort();
    assert_eq!(seen, vec![(a, 10), (b, 20)]);
    store.clear();
    assert!(store.is_empty() && store.get(a).is_none());
}

#[test]
fn a_handle_prints_its_index_and_generation() {
    let mut entities = Entities::default();
    let first = entities.spawn();
    entities.despawn(first);
    assert_eq!(entities.spawn().to_string(), "#0v1");
}
