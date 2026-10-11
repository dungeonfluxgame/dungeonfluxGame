//! Conservative rendered slot bounds, with coherent opening combinations where maxima correlate.
use super::{
    Tables, render,
    tables::{TablesError, group, intersects},
};
use std::collections::BTreeMap;
fn longest(values: impl Iterator<Item = String>) -> String {
    values.max_by_key(|s| s.chars().count()).unwrap_or_default()
}
fn rendered(text: &str, context: &BTreeMap<String, String>) -> Result<String, TablesError> {
    render::render(text, context).map_err(|e| TablesError::Invalid {
        file: "prose-patterns.json".into(),
        path: "tables".into(),
        reason: e.to_string(),
    })
}
fn bound(file: &str, path: &str, text: &str, maximum: usize) -> Result<(), TablesError> {
    let actual = text.chars().count();
    if actual > maximum {
        return Err(TablesError::Invalid {
            file: file.into(),
            path: path.into(),
            reason: format!("longest rendered value has {actual} characters; maximum {maximum}"),
        });
    }
    Ok(())
}
pub(super) fn validate(tables: &Tables) -> Result<(), TablesError> {
    let mut c = BTreeMap::new();
    for (k, values) in [
        (
            "loc.name",
            tables
                .locations
                .entries
                .iter()
                .map(|e| e.name.clone())
                .collect::<Vec<_>>(),
        ),
        (
            "loc.title",
            tables
                .locations
                .entries
                .iter()
                .map(|e| e.title.clone())
                .collect(),
        ),
        (
            "loc.short",
            tables
                .locations
                .entries
                .iter()
                .map(|e| e.short.clone())
                .collect(),
        ),
        (
            "stake.short",
            tables
                .stakes
                .entries
                .iter()
                .map(|e| e.short.clone())
                .collect(),
        ),
        (
            "stake.full",
            tables
                .stakes
                .entries
                .iter()
                .map(|e| e.full.clone())
                .collect(),
        ),
        (
            "stake.title",
            tables
                .stakes
                .entries
                .iter()
                .map(|e| e.title.clone())
                .collect(),
        ),
        (
            "ant.the",
            tables
                .antagonists
                .entries
                .iter()
                .map(|e| e.the.clone())
                .collect(),
        ),
    ] {
        c.insert(k.into(), longest(values.into_iter()));
    }
    let given = longest(
        tables
            .names
            .iter()
            .filter(|(k, _)| *k == "he" || *k == "she")
            .flat_map(|(_, t)| t.entries.iter().map(|e| e.text.clone())),
    );
    let surname = longest(
        tables
            .names
            .get("surnames")
            .into_iter()
            .flat_map(|t| t.entries.iter().map(|e| e.text.clone())),
    );
    for (prefix, npcs) in [
        ("patron", &tables.patrons.entries),
        ("contact", &tables.contacts.entries),
    ] {
        for (k, values) in [
            ("first", vec![given.clone()]),
            ("full", vec![format!("{given} {surname}")]),
            ("role", npcs.iter().map(|e| e.role.clone()).collect()),
            ("desc", npcs.iter().map(|e| e.desc.clone()).collect()),
        ] {
            c.insert(format!("{prefix}.{k}"), longest(values.into_iter()));
        }
        for (k, value) in [
            ("sub", "she"),
            ("obj", "him"),
            ("pos", "his"),
            ("self", "himself"),
        ] {
            c.insert(format!("{prefix}.{k}"), value.into());
        }
        let mannerism = longest(
            tables
                .traits
                .get("mannerisms")
                .into_iter()
                .flat_map(|t| t.entries.iter().map(|e| e.text.replace("{pos}", "his"))),
        );
        c.insert(format!("{prefix}.mannerism"), mannerism);
        bound(
            "names.json",
            &format!("{prefix}.full"),
            &format!("{given} {surname}"),
            28,
        )?;
    }
    for k in ["name", "title", "short"] {
        let values = tables.destinations.values().flat_map(|t| {
            t.entries.iter().map(|e| match k {
                "name" => e.name.clone(),
                "title" => e.title.clone(),
                _ => e.short.clone(),
            })
        });
        c.insert(format!("dest.{k}"), longest(values));
    }
    let atmosphere = longest(
        tables
            .locations
            .entries
            .iter()
            .flat_map(|l| l.atmosphere.entries.iter().map(|e| e.text.clone())),
    );
    for letter in ["a", "b", "c"] {
        c.insert(format!("loc.atmos_{letter}"), atmosphere.clone());
    }
    c.insert("ant.short".into(), "x".repeat(29));
    c.insert("ant.full".into(), "x".repeat(29));
    for (key, value) in [
        ("ant.sub", "she"),
        ("ant.obj", "him"),
        ("ant.pos", "his"),
        ("ant.self", "himself"),
        ("pos", "his"),
        ("name", "{name}"),
    ] {
        c.insert(key.into(), value.into());
    }
    let entrances = tables
        .hooks
        .entries
        .iter()
        .map(|e| rendered(&e.entrance, &c))
        .collect::<Result<Vec<_>, _>>()?;
    c.insert("entrance".into(), longest(entrances.into_iter()));
    let fore = tables
        .prose
        .iter()
        .filter(|(k, _)| k.starts_with("foreshadow_"))
        .flat_map(|(_, t)| &t.variants)
        .map(|e| rendered(&e.text, &c))
        .collect::<Result<Vec<_>, _>>()?;
    c.insert("foreshadow".into(), longest(fore.into_iter()));
    let arrivals = tables
        .antagonists
        .entries
        .iter()
        .flat_map(|e| &e.arrivals.variants)
        .map(|e| rendered(&e.text, &c))
        .collect::<Result<Vec<_>, _>>()?;
    c.insert("arrival".into(), longest(arrivals.into_iter()));
    let routes = tables
        .prose
        .iter()
        .filter(|(k, _)| k.starts_with("route_"))
        .flat_map(|(_, t)| &t.variants)
        .map(|e| rendered(&e.text, &c))
        .collect::<Result<Vec<_>, _>>()?;
    c.insert("route".into(), longest(routes.into_iter()));
    for key in ["fact", "confirm"] {
        let values = tables
            .secrets
            .entries
            .iter()
            .map(|e| rendered(if key == "fact" { &e.fact } else { &e.confirm }, &c))
            .collect::<Result<Vec<_>, _>>()?;
        c.insert(format!("secret.{key}"), longest(values.into_iter()));
    }
    c.insert(
        "reward.line".into(),
        longest(tables.rewards.entries.iter().map(|e| e.text.clone())),
    );
    for (key, table) in &tables.prose {
        if table.die.is_none() || key == "opening_narration" {
            continue;
        }
        let maximum = if key.ends_with("title") {
            32
        } else if key.ends_with("label") {
            40
        } else if key == "clue" {
            280
        } else if key.ends_with("dialogue") {
            160
        } else {
            220
        };
        for (index, e) in table.variants.iter().enumerate() {
            bound(
                "prose-patterns.json",
                &format!("tables.{key}.variants[{index}].text"),
                &rendered(&e.text, &c)?,
                maximum,
            )?;
        }
    }
    for hook in &tables.hooks.entries {
        for (field, text, maximum) in [
            ("title", &hook.title, 32),
            ("pitch", &hook.pitch, 160),
            ("answer", &hook.answer, 160),
            ("accept", &hook.accept, 160),
        ] {
            bound(
                "hooks.json",
                &format!("tables.hooks.{}.{}", hook.id, field),
                &rendered(text, &c)?,
                maximum,
            )?;
        }
    }
    for ant in &tables.antagonists.entries {
        if let Some(warnings) = &ant.warnings {
            for warning in &warnings.variants {
                bound(
                    "antagonists.json",
                    &format!("{}.warnings", ant.id),
                    &rendered(&warning.text, &c)?,
                    160,
                )?;
            }
        }
    }
    // Composed story fields also have a single bound, not one bound per component.
    let intro = longest(
        tables
            .prose
            .get("inn_intro")
            .into_iter()
            .flat_map(|t| &t.variants)
            .map(|e| rendered(&e.text, &c))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter(),
    );
    let dest_arrival = longest(
        tables
            .destinations
            .values()
            .flat_map(|t| t.entries.iter().map(|e| e.arrival.clone())),
    );
    bound(
        "destinations.json",
        "arrival + inn_intro",
        &format!("{dest_arrival} {intro}"),
        220,
    )?;
    let lead = longest(
        tables
            .prose
            .get("victory_lead")
            .into_iter()
            .flat_map(|t| &t.variants)
            .map(|e| rendered(&e.text, &c))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter(),
    );
    for complication in &tables.complications.entries {
        bound(
            "complications.json",
            &complication.id,
            &format!("{lead} {}", rendered(&complication.text, &c)?),
            220,
        )?;
    }
    for loc in &tables.locations.entries {
        for patron in tables
            .patrons
            .entries
            .iter()
            .filter(|e| intersects(&e.fits, &loc.tags))
        {
            for hook in tables
                .hooks
                .entries
                .iter()
                .filter(|e| intersects(&e.fits, &loc.tags))
            {
                for stake in tables
                    .stakes
                    .entries
                    .iter()
                    .filter(|e| intersects(&e.tags, &hook.stake_tags))
                {
                    let mut coherent = c.clone();
                    for (k, v) in [
                        ("loc.name", &loc.name),
                        ("patron.desc", &patron.desc),
                        ("stake.full", &stake.full),
                        ("stake.short", &stake.short),
                    ] {
                        coherent.insert(k.into(), v.clone());
                    }
                    coherent.insert(
                        "loc.atmos_c".into(),
                        longest(loc.atmosphere.entries.iter().map(|e| e.text.clone())),
                    );
                    coherent.insert(
                        "pos".into(),
                        if patron.gender == "he" { "his" } else { "her" }.into(),
                    );
                    let entrance = rendered(&hook.entrance, &coherent)?;
                    coherent.insert("entrance".into(), entrance);
                    for ant in tables
                        .antagonists
                        .entries
                        .iter()
                        .filter(|e| intersects(&e.fits, &loc.tags))
                    {
                        let key = format!("foreshadow_{}", group(ant));
                        let fore = longest(
                            tables
                                .prose
                                .get(&key)
                                .into_iter()
                                .flat_map(|t| &t.variants)
                                .map(|e| rendered(&e.text, &coherent))
                                .collect::<Result<Vec<_>, _>>()?
                                .into_iter(),
                        );
                        coherent.insert("foreshadow".into(), fore);
                        for e in tables
                            .prose
                            .get("opening_narration")
                            .into_iter()
                            .flat_map(|t| &t.variants)
                        {
                            bound(
                                "prose-patterns.json",
                                "tables.opening_narration",
                                &rendered(&e.text, &coherent)?,
                                220,
                            )?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
