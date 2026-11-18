use crate::limits::MEDIA_LINK_LIFETIME_S;
use crate::path::{forget_media_links, keeps_link, media_file, remember_media_links};

fn link(expires: u64) -> String {
    format!("{expires}.abcdef")
}

#[test]
fn a_kept_link_stays_until_a_newer_one_lasts_half_a_lifetime_longer() {
    let kept = link(10_000);
    let half = MEDIA_LINK_LIFETIME_S / 2;
    assert!(keeps_link(&kept, &link(10_000)));
    assert!(keeps_link(&kept, &link(10_000 + half - 1)));
    assert!(!keeps_link(&kept, &link(10_000 + half)));
    // An older link never replaces a newer one.
    assert!(keeps_link(&kept, &link(9_000)));
    // A link that cannot be read is replaced, and an unreadable offer is not kept over nothing.
    assert!(!keeps_link("garbage", &link(10_000)));
    assert!(!keeps_link(&kept, "garbage"));
}

#[test]
fn a_files_address_stays_the_same_across_answers_and_a_new_host_forgets_it() {
    forget_media_links();
    remember_media_links(&format!("plate.png={}", link(1_000)));
    let first = media_file("plate.png");
    assert_eq!(
        first,
        format!("/playground/media/plate.png?ml={}", link(1_000))
    );
    remember_media_links(&format!(
        "plate.png={};hero.png={}",
        link(1_001),
        link(1_001)
    ));
    assert_eq!(
        media_file("plate.png"),
        first,
        "a poll later the address is the same"
    );
    assert_eq!(
        media_file("hero.png"),
        format!("/playground/media/hero.png?ml={}", link(1_001))
    );
    let later = link(1_000 + MEDIA_LINK_LIFETIME_S / 2);
    remember_media_links(&format!("plate.png={later}"));
    assert_eq!(
        media_file("plate.png"),
        format!("/playground/media/plate.png?ml={later}")
    );
    forget_media_links();
    assert_eq!(media_file("plate.png"), "/playground/media/plate.png");
}
