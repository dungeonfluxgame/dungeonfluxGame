//! Designer inspection uses the same generator and includes DM-only content by explicit command.
use super::super::{model, startup_configuration};
use super::{ForcedEntries, GenerateOptions, Tables, generate};
use std::{ffi::OsString, path::Path};
pub(crate) fn run(arguments: &[OsString]) -> Result<(), Box<dyn std::error::Error>> {
    if !(1..=2).contains(&arguments.len()) {
        return Err("usage: df-transport-fixture --roll-adventure <tables_dir> [seed]".into());
    }
    let directory = arguments.first().ok_or("tables directory required")?;
    let seed = match arguments.get(1) {
        Some(value) => {
            startup_configuration::adventure_seed(value.to_str().ok_or("seed must be Unicode")?)?
        }
        None => u64::from_le_bytes(model::random::<8>().map_err(|_| "OS seed unavailable")?),
    };
    let tables = Tables::load(Path::new(directory))?;
    let generated = generate(
        &tables,
        seed,
        GenerateOptions {
            rules_mode: startup_configuration::adventure_creature_mode(std::env::var(
                "DF_GAMEPLAY_ADVENTURE_CREATURES",
            ))?,
        },
        &ForcedEntries::new(),
    )?;
    println!(
        "ADVENTURE seed=0x{seed:016x} tables={} generator=splitmix64-patterns-1",
        generated.tables_revision
    );
    println!(
        "Facts: location={} hook={} patron={} stake={} antagonist={} complication={} secret={} destination={} contact={}",
        generated.facts["loc.id"],
        generated.facts["hook.id"],
        generated.facts["patron.full"],
        generated.facts["stake.id"],
        generated.facts["ant.full"],
        generated.facts["complication.id"],
        generated.facts["secret.id"],
        generated.facts["dest.id"],
        generated.facts["contact.full"]
    );
    println!("DM's rolls ({}):", generated.rolls.len());
    for roll in &generated.rolls {
        println!(
            "{:02} {} d{}:{} {} [{:?}]",
            roll.n, roll.table, roll.die, roll.natural, roll.entry_id, roll.status
        );
    }
    let labels = &generated.pack.offer_labels;
    println!(
        "Offers: begin=\"{}\"; ask=\"{}\"; help=\"{}\"; stand=\"{}\"; continue=\"{}\"; destination=\"{}\"",
        labels.begin_story,
        labels.ask_courier,
        labels.escort_courier,
        labels.defend_courier,
        labels.choose_harbor_scene,
        generated
            .pack
            .destination_label
            .as_deref()
            .ok_or("generated destination missing")?
    );
    println!("Private clue: {}", generated.pack.courier_clue);
    let enemy = &generated.tuning.bandit;
    println!(
        "Antagonist: {} ({}) HP {} AC {} attack {:+}; {} {}d{}{:+}; abilities {:?}",
        generated.pack.bandit.name,
        generated
            .pack
            .bandit
            .srd_id
            .as_deref()
            .ok_or("generated SRD id missing")?,
        enemy.hit_points,
        enemy.armor_class,
        enemy.attack_bonus,
        enemy.weapon.item,
        enemy.weapon.damage_dice,
        enemy.weapon.damage_sides,
        enemy.damage_modifier,
        enemy.abilities
    );
    for scene in &generated.pack.scenes {
        println!("\n{:?}: {}\n{}", scene.key, scene.title, scene.narration);
        if !scene.dialogue.is_empty() {
            println!("{}: \"{}\"", scene.speaker, scene.dialogue);
        }
        if let Some(rested) = &scene.narration_rested {
            println!("After rest: {rested}");
        }
    }
    Ok(())
}
