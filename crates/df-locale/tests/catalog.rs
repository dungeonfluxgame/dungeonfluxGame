use df_locale::{
    Catalog, LocaleConfigurationError, Lookup, TextKey, TextKeyError, lookup_chain, settle,
};
use df_types::LocaleTag;

fn parse_locales(spellings: &[&str]) -> Vec<LocaleTag> {
    spellings
        .iter()
        .map(|spelling| LocaleTag::parse(spelling).unwrap())
        .collect()
}

#[test]
fn negotiated_caller_preserves_source_identity_across_translations_fallback_and_miss() {
    let key = TextKey::parse("mechanics.action.attack").unwrap();
    let supported = parse_locales(&["en", "fr"]);
    let default = LocaleTag::parse("en").unwrap();
    let source_context = ("source-revision-7", "action.attack");
    let mut translated = Catalog::default();
    translated.insert(&default, key.clone(), "Attack");
    translated.insert(&supported[1], key.clone(), "Attaque");

    for (requested, label) in [("en", "Attack"), ("fr", "Attaque")] {
        let selection = settle(&parse_locales(&[requested]), &supported, &default).unwrap();
        let chain = lookup_chain(selection.selected(), &default);
        let presentation = (source_context, translated.lookup(&key, &chain));
        assert_eq!(presentation.0, source_context);
        assert_eq!(presentation.1.key(), &key);
        assert_eq!(presentation.1.label(), Some(label));
        assert_eq!(presentation.1.source_locale(), Some(selection.selected()));
        assert_eq!(
            presentation.1.searched_locales(),
            &[selection.selected().clone()]
        );
        assert!(!selection.used_fallback());
    }

    let selection = settle(&parse_locales(&["fr"]), &supported, &default).unwrap();
    let chain = lookup_chain(selection.selected(), &default);
    let mut default_only = Catalog::default();
    default_only.insert(&default, key.clone(), "Attack");
    let fallback = (source_context, default_only.lookup(&key, &chain));
    assert_eq!(fallback.0, source_context);
    assert_eq!(fallback.1.key(), &key);
    assert_eq!(fallback.1.label(), Some("Attack"));
    assert_eq!(fallback.1.source_locale(), Some(&default));
    assert_eq!(fallback.1.searched_locales(), &chain);
    assert!(!selection.used_fallback());

    let missing_key = TextKey::parse("mechanics.action.help").unwrap();
    let missing = (source_context, translated.lookup(&missing_key, &chain));
    assert_eq!(missing.0, source_context);
    assert_eq!(missing.1.key().as_str(), "mechanics.action.help");
    assert_eq!(missing.1.label(), None);
    assert_eq!(missing.1.source_locale(), None);
    assert_eq!(missing.1.searched_locales(), &chain);
    assert_eq!(
        missing.1,
        Lookup::Missing {
            key: missing_key,
            searched_locales: chain,
        }
    );
}

#[test]
fn parsed_key_grammar_accepts_exact_spelling_and_refuses_malformed_segments() {
    for spelling in ["a", "mechanics.action.attack", "a0.b_c2", "a_", "abc123"] {
        let key = TextKey::parse(spelling).unwrap();
        assert_eq!(key.as_str(), spelling);
        assert_eq!(TextKey::parse(spelling), Ok(key.clone()));
        assert_eq!(key, key.clone());
    }
    for spelling in [
        "",
        ".attack",
        "mechanics..attack",
        "mechanics.attack.",
        "Mechanics.attack",
        "mechanics.ünknown",
        "mechanics.action attack",
        " attack",
        "attack\n",
        "a.1b",
        "_attack",
        "a._b",
        "a-b",
        "a/b",
        "a\0b",
    ] {
        assert_eq!(TextKey::parse(spelling), Err(TextKeyError::InvalidSyntax));
    }
}

#[test]
fn chain_deduplicates_only_equal_inputs_and_empty_chain_is_an_explicit_miss() {
    let selected = LocaleTag::parse("FR").unwrap();
    let default = LocaleTag::parse("en").unwrap();
    assert_eq!(
        lookup_chain(&selected, &default),
        parse_locales(&["fr", "en"])
    );
    assert_eq!(lookup_chain(&default, &default), parse_locales(&["en"]));
    assert_eq!(
        lookup_chain(&selected, &LocaleTag::parse("fr").unwrap()),
        parse_locales(&["fr"])
    );
    let key = TextKey::parse("mechanics.action.attack").unwrap();
    let mut catalog = Catalog::default();
    catalog.insert(&default, key.clone(), "Attack");
    assert_eq!(
        catalog.lookup(&key, &[]),
        Lookup::Missing {
            key,
            searched_locales: vec![],
        }
    );
}

#[test]
fn exact_tags_and_key_isolation_preserve_caller_inputs_and_search_order() {
    let key = TextKey::parse("mechanics.action.attack").unwrap();
    let other = TextKey::parse("mechanics.action.dodge").unwrap();
    let locale = LocaleTag::parse("en-US").unwrap();
    let chain = parse_locales(&["en", "fr", "en-US"]);
    let before = (key.clone(), chain.clone());
    let mut catalog = Catalog::default();
    catalog.insert(&locale, key.clone(), "Attack");
    catalog.insert(&chain[0], other, "Dodge");
    let found = catalog.lookup(&key, &chain);
    assert_eq!(found.label(), Some("Attack"));
    assert_eq!(found.source_locale(), Some(&locale));
    assert_eq!(found.searched_locales(), &chain);
    assert!(matches!(
        catalog.lookup(&key, &chain[..2]),
        Lookup::Missing { .. }
    ));
    assert_eq!((key, chain), before);

    for (stored, requested) in [("he", "iw"), ("tlh", "i-klingon"), ("zh-Hant", "zh-Hans")] {
        let key = TextKey::parse("mechanics.action.attack").unwrap();
        let mut catalog = Catalog::default();
        catalog.insert(&LocaleTag::parse(stored).unwrap(), key.clone(), "label");
        assert!(matches!(
            catalog.lookup(&key, &parse_locales(&[requested])),
            Lookup::Missing { .. }
        ));
    }
}

#[test]
fn catalog_owns_inserted_text_and_found_text_borrows_storage_including_empty_entries() {
    let key = TextKey::parse("mechanics.action.attack").unwrap();
    let chain = parse_locales(&["en"]);
    let mut catalog = Catalog::default();
    {
        let text = String::from("Attack");
        catalog.insert(&chain[0], key.clone(), &text);
    }
    let first = catalog.lookup(&key, &chain);
    let second = catalog.lookup(&key, &chain);
    assert_eq!(first.label(), Some("Attack"));
    assert!(std::ptr::eq(
        first.label().unwrap(),
        second.label().unwrap()
    ));
    catalog.insert(&chain[0], key.clone(), "");
    assert_eq!(
        catalog.lookup(&key, &chain),
        Lookup::Found {
            key,
            text: "",
            source_locale: chain[0].clone(),
            searched_locales: chain.clone(),
        }
    );
}

#[test]
fn unsupported_default_refuses_the_negotiated_lookup_caller() {
    let requested = parse_locales(&["fr"]);
    let supported = parse_locales(&["fr"]);
    let default = LocaleTag::parse("en").unwrap();
    let key = TextKey::parse("mechanics.action.attack").unwrap();
    let mut catalog = Catalog::default();
    catalog.insert(&requested[0], key.clone(), "Attaque");
    let lookup = settle(&requested, &supported, &default)
        .map(|selection| catalog.lookup(&key, &lookup_chain(selection.selected(), &default)));
    assert_eq!(lookup, Err(LocaleConfigurationError::UnsupportedDefault));
}
