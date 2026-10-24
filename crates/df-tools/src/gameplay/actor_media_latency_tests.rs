use super::*;

#[test]
fn hs53_measure_six_hero_reply_projection() {
    let loaded = super::super::pack::load_from_bytes(
        include_bytes!("../../../../content/free-roam/pack.json"),
        include_bytes!("../../../../content/free-roam/tuning.json"),
    ).unwrap();
    super::super::pack::with_test_pack(loaded, || {
        let started = Instant::now();
        let current = super::super::beat_script_runtime::opening_social_tests::opening(6);
        println!("HS53 six-hero setup_ms={}", started.elapsed().as_millis());
        let member = current.state().characters[0].owner;
        let started = Instant::now();
        assert!(audit_combat_view(&current, member).is_none());
        println!("HS53 audit_combat_ms={}", started.elapsed().as_millis());
        let started = Instant::now();
        assert!(audit_hero(&current, member).is_some());
        println!("HS53 audit_hero_ms={}", started.elapsed().as_millis());
    });
}
