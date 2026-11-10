//! The people check over a fake scanner: a picture that shows more people than its cast is made
//! once more, the second attempt that is within the cast is the picture (with its own scan), and
//! two that are not are refused.

use std::cell::RefCell;
use std::time::Duration;

use super::people_check::{Attempts, Verified, more_than_cast, verified};

/// The attempts at a picture whose scans find `counts` people, one per attempt (`None`: the scan
/// could not say). The plate of an attempt is its own name, standing for its picture and layout.
async fn attempts_at(
    cast: usize,
    counts: &[Option<usize>],
) -> (Result<Attempts<String>, String>, Vec<u8>) {
    let made = RefCell::new(Vec::new());
    let result = verified(cast, |attempt| {
        made.borrow_mut().push(attempt);
        let people = counts[usize::from(attempt) - 1];
        async move {
            Ok::<_, String>((
                format!("plate-{attempt}"),
                people,
                Duration::from_secs(u64::from(attempt) * 10),
            ))
        }
    })
    .await;
    (result, made.into_inner())
}

#[test]
fn a_picture_shows_more_than_its_cast_only_with_more_people_than_the_cast() {
    assert!(more_than_cast(2, 1));
    assert!(!more_than_cast(1, 1));
    assert!(!more_than_cast(0, 1));
    assert!(!more_than_cast(3, 3));
}

#[tokio::test]
async fn a_picture_within_its_cast_is_accepted_at_once_with_its_own_scan() {
    let (result, made) = attempts_at(1, &[Some(1), Some(9)]).await;
    let result = result.unwrap();
    assert_eq!(
        result.verdict,
        Verified::Accepted {
            plate: "plate-1".to_owned(),
            attempt: 1
        }
    );
    assert_eq!(made, [1], "no second attempt is made");
    assert_eq!(result.makings, [Duration::from_secs(10)]);
}

#[tokio::test]
async fn a_picture_with_more_people_than_the_cast_is_made_again_and_the_second_is_the_picture() {
    let (result, made) = attempts_at(1, &[Some(2), Some(1)]).await;
    let result = result.unwrap();
    assert_eq!(made, [1, 2]);
    assert_eq!(
        result.verdict,
        Verified::Accepted {
            plate: "plate-2".to_owned(),
            attempt: 2
        },
        "the second attempt's own plate, and so its own layout, is kept; the first is not"
    );
    assert_eq!(
        result.makings,
        [Duration::from_secs(10), Duration::from_secs(20)],
        "how long each took"
    );
}

#[tokio::test]
async fn a_picture_that_shows_more_people_than_the_cast_twice_is_refused() {
    let (result, made) = attempts_at(1, &[Some(2), Some(3)]).await;
    let result = result.unwrap();
    assert_eq!(made, [1, 2], "one more attempt and no more");
    assert_eq!(result.verdict, Verified::Rejected { found: 3 });
    assert_eq!(result.makings.len(), 2);
}

#[tokio::test]
async fn a_cast_of_several_allows_as_many_people() {
    let (result, made) = attempts_at(3, &[Some(3)]).await;
    assert_eq!(made, [1]);
    assert!(matches!(
        result.unwrap().verdict,
        Verified::Accepted { attempt: 1, .. }
    ));
}

#[tokio::test]
async fn a_picture_whose_people_could_not_be_counted_is_accepted() {
    let (result, made) = attempts_at(1, &[None]).await;
    assert_eq!(made, [1]);
    assert!(matches!(
        result.unwrap().verdict,
        Verified::Accepted { attempt: 1, .. }
    ));
}

#[tokio::test]
async fn a_failure_to_make_a_picture_ends_the_attempts_with_it() {
    let calls = RefCell::new(0_u8);
    let result: Result<Attempts<String>, String> = verified(1, |_| {
        *calls.borrow_mut() += 1;
        async { Err::<(String, Option<usize>, Duration), _>("the scan failed".to_owned()) }
    })
    .await;
    assert_eq!(result.unwrap_err(), "the scan failed");
    assert_eq!(calls.into_inner(), 1);
}
