use df_locale::{
    ArgumentKind, ArgumentValue, CatalogEntry, CatalogLoadError, FormatError, FormattedPart,
    MessageError, MessagePart, TextKey, VersionedCatalog, lookup_chain,
};
use df_types::{Currency, LocaleTag, Money, RevisionLabel, RevisionLabelError, Usage, UsageUnit};
use std::collections::BTreeMap;

struct Translation {
    locale: LocaleTag,
    key: TextKey,
    text: String,
    parts: Vec<MessagePart>,
}

struct Fixture {
    locales: Vec<LocaleTag>,
    declarations: BTreeMap<TextKey, BTreeMap<String, ArgumentKind>>,
    translations: Vec<Translation>,
}

fn key(value: &str) -> TextKey {
    TextKey::parse(value).unwrap()
}

fn locale(value: &str) -> LocaleTag {
    LocaleTag::parse(value).unwrap()
}

fn revision(value: &str) -> RevisionLabel {
    RevisionLabel::new(Some(value)).unwrap()
}

fn name_part() -> MessagePart {
    MessagePart::Argument {
        name: "name".to_owned(),
        kind: ArgumentKind::Text,
    }
}

fn arguments() -> BTreeMap<String, ArgumentValue> {
    BTreeMap::from([(
        "name".to_owned(),
        ArgumentValue::Text("<script>{command}</script>".to_owned()),
    )])
}

impl Fixture {
    fn new() -> Self {
        let declarations = BTreeMap::from([
            (
                key("ui.greeting"),
                BTreeMap::from([("name".to_owned(), ArgumentKind::Text)]),
            ),
            (key("mechanics.action.attack"), BTreeMap::new()),
            (key("ui.empty"), BTreeMap::new()),
        ]);
        let mut translations = Vec::new();
        for (language, greeting, attack) in
            [("en", "Hello ", "Attack"), ("fr", "Bonjour ", "Attaque")]
        {
            for (entry_key, text, parts) in [
                (
                    key("ui.greeting"),
                    greeting,
                    vec![MessagePart::Literal(greeting.to_owned()), name_part()],
                ),
                (
                    key("mechanics.action.attack"),
                    attack,
                    vec![MessagePart::Literal(attack.to_owned())],
                ),
                (key("ui.empty"), "", vec![]),
            ] {
                translations.push(Translation {
                    locale: locale(language),
                    key: entry_key,
                    text: text.to_owned(),
                    parts,
                });
            }
        }
        Self {
            locales: vec![locale("en"), locale("fr")],
            declarations,
            translations,
        }
    }

    fn entries(&self) -> Vec<CatalogEntry<'_>> {
        self.translations
            .iter()
            .map(|entry| CatalogEntry {
                locale: &entry.locale,
                key: &entry.key,
                text: &entry.text,
                parts: &entry.parts,
            })
            .collect()
    }

    fn revisions(&self, value: &str) -> Vec<RevisionLabel> {
        vec![revision(value); self.translations.len()]
    }

    fn load(&self, value: &str) -> VersionedCatalog<RevisionLabel> {
        VersionedCatalog::load_revision(
            Some(value),
            &self.locales,
            &self.declarations,
            &self.entries(),
            &self.revisions(value),
        )
        .unwrap()
    }

    fn snapshot(&self, catalog: &VersionedCatalog<RevisionLabel>) -> (RevisionLabel, Vec<String>) {
        let mut values = Vec::new();
        for language in &self.locales {
            for entry_key in self.declarations.keys() {
                let chain = std::slice::from_ref(language);
                let lookup = catalog.lookup(entry_key, chain);
                assert_eq!(lookup.version, catalog.version());
                values.push(lookup.lookup.label().unwrap().to_owned());
                let arguments = if entry_key == &key("ui.greeting") {
                    arguments()
                } else {
                    BTreeMap::new()
                };
                let formatted = catalog.format(entry_key, chain, &arguments).unwrap();
                assert_eq!(formatted.version, catalog.version());
                values.push(formatted.message.plain_text());
            }
        }
        (catalog.version().clone(), values)
    }
}

#[test]
fn complete_source_set_emits_its_canonical_revision_with_plain_labels_and_messages() {
    let fixture = Fixture::new();
    let catalog = fixture.load("text:1");
    let found = catalog.lookup(&key("ui.greeting"), &[locale("fr")]);
    assert_eq!(found.version, &revision("text:1"));
    assert_eq!(found.lookup.label(), Some("Bonjour "));
    assert_eq!(found.lookup.source_locale(), Some(&locale("fr")));
    let formatted = catalog
        .format(&key("ui.greeting"), &[locale("fr")], &arguments())
        .unwrap();
    assert_eq!(formatted.version, found.version);
    assert_eq!(formatted.message.key, key("ui.greeting"));
    assert_eq!(
        formatted.message.plain_text(),
        "Bonjour <script>{command}</script>"
    );
}

#[test]
fn absent_empty_overlong_and_malformed_revisions_refuse_initial_load_and_reload() {
    let fixture = Fixture::new();
    let mut catalog = fixture.load("text:1");
    let before = fixture.snapshot(&catalog);
    let too_long = "x".repeat(129);
    for (value, error) in [
        (None, RevisionLabelError::Missing),
        (Some(""), RevisionLabelError::Empty),
        (
            Some(too_long.as_str()),
            RevisionLabelError::TooLong { actual: 129 },
        ),
        (
            Some("text private"),
            RevisionLabelError::InvalidCharacter { byte_index: 4 },
        ),
    ] {
        let expected = CatalogLoadError::InvalidRevision(error);
        assert_eq!(
            catalog.reload_revision(
                value,
                &fixture.locales,
                &fixture.declarations,
                &fixture.entries(),
                &fixture.revisions("text:2")
            ),
            Err(expected)
        );
        assert_eq!(fixture.snapshot(&catalog), before);
        assert!(matches!(
            VersionedCatalog::load_revision(
                value,
                &fixture.locales,
                &fixture.declarations,
                &fixture.entries(),
                &fixture.revisions("text:2")
            ),
            Err(CatalogLoadError::InvalidRevision(_))
        ));
    }
}

#[test]
fn mixed_source_revisions_refuse_every_entry_position_before_replacement() {
    let fixture = Fixture::new();
    let mut catalog = fixture.load("text:1");
    let before = fixture.snapshot(&catalog);
    for index in 0..fixture.translations.len() {
        let mut revisions = fixture.revisions("text:2");
        revisions[index] = revision("other:private");
        let entry = &fixture.translations[index];
        let error = catalog
            .reload_revision(
                Some("text:2"),
                &fixture.locales,
                &fixture.declarations,
                &fixture.entries(),
                &revisions,
            )
            .unwrap_err();
        assert_eq!(
            error,
            CatalogLoadError::MixedRevision {
                locale: entry.locale.clone(),
                key: entry.key.clone()
            }
        );
        assert!(!format!("{error:?}").contains("other:private"));
        assert_eq!(fixture.snapshot(&catalog), before);
    }
}

#[test]
fn source_revision_inventory_requires_exact_one_to_one_entry_binding() {
    let fixture = Fixture::new();
    let mut catalog = fixture.load("text:1");
    let before = fixture.snapshot(&catalog);
    for count in [
        0,
        fixture.translations.len() - 1,
        fixture.translations.len() + 1,
    ] {
        let revisions = vec![revision("text:2"); count];
        assert_eq!(
            catalog.reload_revision(
                Some("text:2"),
                &fixture.locales,
                &fixture.declarations,
                &fixture.entries(),
                &revisions
            ),
            Err(CatalogLoadError::RevisionInventoryMismatch)
        );
        assert_eq!(fixture.snapshot(&catalog), before);
    }
}

#[test]
fn every_missing_translation_refuses_atomically_even_when_fallback_exists() {
    let fixture = Fixture::new();
    let mut catalog = fixture.load("text:1");
    let before = fixture.snapshot(&catalog);
    for index in 0..fixture.translations.len() {
        let mut entries = fixture.entries();
        entries.remove(index);
        let revisions = vec![revision("text:2"); entries.len()];
        let missing = &fixture.translations[index];
        assert_eq!(
            catalog.reload_revision(
                Some("text:2"),
                &fixture.locales,
                &fixture.declarations,
                &entries,
                &revisions
            ),
            Err(CatalogLoadError::MissingEntry {
                locale: missing.locale.clone(),
                key: missing.key.clone()
            })
        );
        assert_eq!(fixture.snapshot(&catalog), before);
    }
    assert!(matches!(
        VersionedCatalog::load_revision(
            Some("text:2"),
            &fixture.locales,
            &fixture.declarations,
            &[],
            &[]
        ),
        Err(CatalogLoadError::MissingEntry { .. })
    ));
}

#[test]
fn empty_duplicate_and_unexpected_inventory_never_changes_the_active_set() {
    let fixture = Fixture::new();
    let mut catalog = fixture.load("text:1");
    let before = fixture.snapshot(&catalog);
    let entries = fixture.entries();
    let revisions = fixture.revisions("text:2");
    for (locales, declarations) in [
        (&[][..], &fixture.declarations),
        (fixture.locales.as_slice(), &BTreeMap::new()),
    ] {
        assert_eq!(
            catalog.reload_revision(Some("text:2"), locales, declarations, &entries, &revisions),
            Err(CatalogLoadError::EmptyManifest)
        );
        assert_eq!(fixture.snapshot(&catalog), before);
    }
    let duplicated = [locale("en"), locale("en")];
    assert_eq!(
        catalog.reload_revision(
            Some("text:2"),
            &duplicated,
            &fixture.declarations,
            &entries,
            &revisions
        ),
        Err(CatalogLoadError::DuplicateLocale {
            locale: locale("en")
        })
    );
    let mut duplicate_entries = fixture.entries();
    duplicate_entries.push(CatalogEntry {
        locale: entries[0].locale,
        key: entries[0].key,
        text: entries[0].text,
        parts: entries[0].parts,
    });
    let duplicate_versions = vec![revision("text:2"); duplicate_entries.len()];
    assert!(matches!(
        catalog.reload_revision(
            Some("text:2"),
            &fixture.locales,
            &fixture.declarations,
            &duplicate_entries,
            &duplicate_versions
        ),
        Err(CatalogLoadError::DuplicateEntry { .. })
    ));
    let extra_locale = locale("de");
    let extra_key = key("ui.extra");
    for (entry, expected) in [
        (
            CatalogEntry {
                locale: &extra_locale,
                key: entries[0].key,
                text: "private",
                parts: &[],
            },
            CatalogLoadError::UnexpectedLocale {
                locale: extra_locale.clone(),
            },
        ),
        (
            CatalogEntry {
                locale: entries[0].locale,
                key: &extra_key,
                text: "private",
                parts: &[],
            },
            CatalogLoadError::UnexpectedKey {
                key: extra_key.clone(),
            },
        ),
    ] {
        assert_eq!(
            catalog.reload_revision(
                Some("text:2"),
                &fixture.locales,
                &fixture.declarations,
                &[entry],
                &[revision("text:2")]
            ),
            Err(expected)
        );
        assert_eq!(fixture.snapshot(&catalog), before);
    }
}

#[test]
fn invalid_source_declarations_and_translation_slots_refuse_atomically() {
    let fixture = Fixture::new();
    let mut catalog = fixture.load("text:1");
    let before = fixture.snapshot(&catalog);
    let mut invalid = fixture.declarations.clone();
    invalid.insert(
        key("ui.greeting"),
        BTreeMap::from([("Bad.Name".to_owned(), ArgumentKind::Text)]),
    );
    assert!(matches!(
        catalog.reload_revision(
            Some("text:2"),
            &fixture.locales,
            &invalid,
            &fixture.entries(),
            &fixture.revisions("text:2")
        ),
        Err(CatalogLoadError::InvalidDeclaration {
            error: MessageError::InvalidArgumentName,
            ..
        })
    ));
    assert_eq!(fixture.snapshot(&catalog), before);
    for invalid_parts in [
        vec![],
        vec![MessagePart::Argument {
            name: "name".to_owned(),
            kind: ArgumentKind::Quantity(UsageUnit::Token),
        }],
        vec![
            name_part(),
            MessagePart::Argument {
                name: "extra".to_owned(),
                kind: ArgumentKind::Text,
            },
        ],
    ] {
        let mut entries = fixture.entries();
        entries[3].parts = &invalid_parts;
        assert!(matches!(
            catalog.reload_revision(
                Some("text:2"),
                &fixture.locales,
                &fixture.declarations,
                &entries,
                &fixture.revisions("text:2")
            ),
            Err(CatalogLoadError::InvalidMessage {
                error: MessageError::SlotMismatch,
                ..
            })
        ));
        assert_eq!(fixture.snapshot(&catalog), before);
    }
}

#[test]
fn admitted_reload_replaces_inventory_slot_declarations_and_text_revision_together() {
    let fixture = Fixture::new();
    let mut catalog = fixture.load("text:1");
    let german = locale("de");
    let greeting = key("ui.greeting");
    let kind = ArgumentKind::Quantity(UsageUnit::Token);
    let declarations = BTreeMap::from([(
        greeting.clone(),
        BTreeMap::from([("name".to_owned(), kind)]),
    )]);
    let parts = [MessagePart::Argument {
        name: "name".to_owned(),
        kind,
    }];
    let entries = [CatalogEntry {
        locale: &german,
        key: &greeting,
        text: "Neu",
        parts: &parts,
    }];
    catalog
        .reload_revision(
            Some("text:2"),
            std::slice::from_ref(&german),
            &declarations,
            &entries,
            &[revision("text:2")],
        )
        .unwrap();
    assert_eq!(catalog.version(), &revision("text:2"));
    assert_eq!(
        catalog.lookup(&greeting, &[locale("en")]).lookup.label(),
        None
    );
    assert_eq!(
        catalog
            .lookup(&key("ui.empty"), std::slice::from_ref(&german))
            .lookup
            .label(),
        None
    );
    let quantity = Usage::new(u128::MAX, UsageUnit::Token);
    let arguments = BTreeMap::from([("name".to_owned(), ArgumentValue::Quantity(quantity))]);
    let formatted = catalog.format(&greeting, &[german], &arguments).unwrap();
    assert_eq!(formatted.version, &revision("text:2"));
    assert_eq!(
        formatted.message.parts,
        vec![FormattedPart::Quantity(quantity)]
    );
}

#[test]
fn fallback_source_locale_changes_without_changing_the_admitted_text_revision() {
    let fixture = Fixture::new();
    let catalog = fixture.load("text:1");
    let chain = lookup_chain(&locale("de"), &locale("en"));
    let found = catalog.lookup(&key("mechanics.action.attack"), &chain);
    assert_eq!(found.version, &revision("text:1"));
    assert_eq!(found.lookup.source_locale(), Some(&locale("en")));
    assert_eq!(found.lookup.searched_locales(), chain);
    let formatted = catalog
        .format(&key("mechanics.action.attack"), &chain, &BTreeMap::new())
        .unwrap();
    assert_eq!(formatted.version, found.version);
    assert_eq!(formatted.message.key, key("mechanics.action.attack"));
}

#[test]
fn empty_translations_are_explicit_entries_with_the_set_revision() {
    let fixture = Fixture::new();
    let catalog = fixture.load("text:1");
    let empty = catalog.lookup(&key("ui.empty"), &[locale("fr")]);
    assert_eq!(empty.version, &revision("text:1"));
    assert_eq!(empty.lookup.label(), Some(""));
    let formatted = catalog
        .format(&key("ui.empty"), &[locale("fr")], &BTreeMap::new())
        .unwrap();
    assert_eq!(formatted.version, empty.version);
    assert!(formatted.message.parts.is_empty());
}

#[test]
fn missing_keys_and_invalid_arguments_emit_no_fabricated_label_or_message() {
    let fixture = Fixture::new();
    let catalog = fixture.load("text:1");
    let missing_key = key("ui.missing");
    let chain = [locale("fr"), locale("en")];
    let missing = catalog.lookup(&missing_key, &chain);
    assert_eq!(missing.version, &revision("text:1"));
    assert_eq!(missing.lookup.key(), &missing_key);
    assert_eq!(missing.lookup.label(), None);
    assert_eq!(missing.lookup.source_locale(), None);
    assert_eq!(missing.lookup.searched_locales(), chain);
    assert_eq!(
        catalog.format(&missing_key, &chain, &BTreeMap::new()).err(),
        Some(FormatError::MissingKey {
            key: missing_key,
            searched_locales: chain.to_vec()
        })
    );
    assert!(matches!(
        catalog.format(&key("ui.greeting"), &chain, &BTreeMap::new()),
        Err(FormatError::MissingArgument { .. })
    ));
}

#[test]
fn revision_admission_preserves_exact_money_quantity_and_stable_keys() {
    let locale = locale("en");
    let entry_key = key("mechanics.resource.cost");
    let currency = Currency::parse("USD").unwrap();
    let kinds = BTreeMap::from([
        ("amount".to_owned(), ArgumentKind::Money(currency)),
        (
            "quantity".to_owned(),
            ArgumentKind::Quantity(UsageUnit::Byte),
        ),
    ]);
    let declarations = BTreeMap::from([(entry_key.clone(), kinds)]);
    let parts = [
        MessagePart::Argument {
            name: "quantity".to_owned(),
            kind: ArgumentKind::Quantity(UsageUnit::Byte),
        },
        MessagePart::Argument {
            name: "amount".to_owned(),
            kind: ArgumentKind::Money(currency),
        },
    ];
    let entries = [CatalogEntry {
        locale: &locale,
        key: &entry_key,
        text: "Cost",
        parts: &parts,
    }];
    let catalog = VersionedCatalog::load_revision(
        Some("text:1"),
        std::slice::from_ref(&locale),
        &declarations,
        &entries,
        &[revision("text:1")],
    )
    .unwrap();
    for value in [0, 1, u128::MAX] {
        let amount = Money::new(currency, value);
        let quantity = Usage::new(value, UsageUnit::Byte);
        let arguments = BTreeMap::from([
            ("amount".to_owned(), ArgumentValue::Money(amount)),
            ("quantity".to_owned(), ArgumentValue::Quantity(quantity)),
        ]);
        let formatted = catalog
            .format(&entry_key, std::slice::from_ref(&locale), &arguments)
            .unwrap();
        assert_eq!(formatted.version, &revision("text:1"));
        assert_eq!(formatted.message.key, entry_key);
        assert_eq!(
            formatted.message.parts,
            vec![
                FormattedPart::Quantity(quantity),
                FormattedPart::Money(amount)
            ]
        );
    }
}
