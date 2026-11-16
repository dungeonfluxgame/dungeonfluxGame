//! The page's role: read from the address, the devices it draws and the names it keeps.

use crate::controls::Device;
use crate::role::PageRole;

#[test]
fn each_role_key_is_its_serde_name_and_reads_back() {
    for role in PageRole::ALL {
        let serde_name = serde_json::to_string(&role).unwrap();
        assert_eq!(serde_name, format!("\"{}\"", role.key()));
        assert_eq!(PageRole::from_key(role.key()), Some(role));
    }
}

#[test]
fn the_play_address_is_a_phone_and_every_playground_address_is_not() {
    assert_eq!(PageRole::of_path("/play"), PageRole::Phone);
    assert_eq!(PageRole::of_path("/play/"), PageRole::Phone);
    assert_eq!(PageRole::of_path("/playground"), PageRole::Playground);
    assert_eq!(PageRole::of_path("/playground/"), PageRole::Playground);
    assert_eq!(PageRole::of_path("/player"), PageRole::Playground);
    assert_eq!(PageRole::of_path("/"), PageRole::Playground);
}

#[test]
fn a_phone_draws_only_the_phone_whatever_the_control_says() {
    for chosen in Device::ALL {
        assert_eq!(PageRole::Phone.shown_device(chosen), Device::Phone);
        assert_eq!(PageRole::Playground.shown_device(chosen), chosen);
    }
    assert!(!PageRole::Phone.reads_canvas());
    assert!(PageRole::Playground.reads_canvas());
}

#[test]
fn a_kept_phone_name_is_used_only_when_it_is_one_the_phone_makes() {
    let name = PageRole::Phone.client_name(0x00c0_ffee);
    assert_eq!(name, "phone-00c0ffee");
    assert!(PageRole::Phone.is_client_name(&name));
    assert!(!PageRole::Playground.is_client_name(&name));
    assert!(!PageRole::Phone.is_client_name("phone-00c0ffe"));
    assert!(!PageRole::Phone.is_client_name("phone-00c0ffeg"));
    assert!(!PageRole::Phone.is_client_name("phone-00c0ffee0"));
    assert!(!PageRole::Phone.is_client_name("phone00c0ffee"));
    assert!(!PageRole::Phone.is_client_name(""));
}
