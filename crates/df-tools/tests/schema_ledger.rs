use std::collections::BTreeSet;

use prost::Message;
use prost_types::{DescriptorProto, EnumDescriptorProto, FileDescriptorSet};

#[derive(Debug, PartialEq, Eq)]
enum LedgerError {
    MalformedDescriptor,
    DuplicateAllocation,
    ReservedAllocation,
    ContractChanged,
}

fn enum_records(
    name: &str,
    value: &EnumDescriptorProto,
    records: &mut BTreeSet<String>,
) -> Result<(), LedgerError> {
    records.insert(format!("enum {name}"));
    let mut numbers = BTreeSet::new();
    let mut names = BTreeSet::new();
    for item in &value.value {
        let number = item.number.unwrap_or_default();
        let item_name = item.name.as_deref().unwrap_or_default();
        if !numbers.insert(number) || !names.insert(item_name) {
            return Err(LedgerError::DuplicateAllocation);
        }
        if value.reserved_name.iter().any(|name| name == item_name)
            || value.reserved_range.iter().any(|range| {
                range.start.is_some_and(|start| number >= start)
                    && range.end.is_some_and(|end| number <= end)
            })
        {
            return Err(LedgerError::ReservedAllocation);
        }
        records.insert(format!("enum-value {name} {number} {item_name}"));
    }
    for range in &value.reserved_range {
        records.insert(format!(
            "enum-reserved-range {name} {:?} {:?}",
            range.start, range.end
        ));
    }
    for reserved in &value.reserved_name {
        records.insert(format!("enum-reserved-name {name} {reserved}"));
    }
    Ok(())
}

fn message_records(
    name: &str,
    value: &DescriptorProto,
    records: &mut BTreeSet<String>,
) -> Result<(), LedgerError> {
    records.insert(format!("message {name}"));
    let mut numbers = BTreeSet::new();
    let mut names = BTreeSet::new();
    for field in &value.field {
        let number = field.number.unwrap_or_default();
        let field_name = field.name.as_deref().unwrap_or_default();
        if !numbers.insert(number) || !names.insert(field_name) {
            return Err(LedgerError::DuplicateAllocation);
        }
        if value.reserved_name.iter().any(|name| name == field_name)
            || value.reserved_range.iter().any(|range| {
                range.start.is_some_and(|start| number >= start)
                    && range.end.is_some_and(|end| number < end)
            })
        {
            return Err(LedgerError::ReservedAllocation);
        }
        let oneof = field
            .oneof_index
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| value.oneof_decl.get(index))
            .and_then(|oneof| oneof.name.as_deref())
            .unwrap_or("-");
        records.insert(format!("field {name} {number} {field_name} type={} label={} presence={} target={} oneof={oneof}", field.r#type.unwrap_or_default(),field.label.unwrap_or_default(),field.proto3_optional.unwrap_or_default(),field.type_name.as_deref().unwrap_or("-")));
    }
    for oneof in &value.oneof_decl {
        records.insert(format!(
            "oneof {name} {}",
            oneof.name.as_deref().unwrap_or_default()
        ));
    }
    for range in &value.reserved_range {
        records.insert(format!(
            "reserved-range {name} {:?} {:?}",
            range.start, range.end
        ));
    }
    for reserved in &value.reserved_name {
        records.insert(format!("reserved-name {name} {reserved}"));
    }
    for nested in &value.nested_type {
        message_records(
            &format!("{name}.{}", nested.name.as_deref().unwrap_or_default()),
            nested,
            records,
        )?;
    }
    for nested in &value.enum_type {
        enum_records(
            &format!("{name}.{}", nested.name.as_deref().unwrap_or_default()),
            nested,
            records,
        )?;
    }
    Ok(())
}

fn descriptor_records(bytes: &[u8]) -> Result<BTreeSet<String>, LedgerError> {
    let descriptor =
        FileDescriptorSet::decode(bytes).map_err(|_| LedgerError::MalformedDescriptor)?;
    let mut records = BTreeSet::new();
    for file in descriptor.file {
        let package = file.package.as_deref().unwrap_or_default();
        records.insert(format!(
            "file {} {package} {}",
            file.name.as_deref().unwrap_or_default(),
            file.syntax.as_deref().unwrap_or_default()
        ));
        for message in &file.message_type {
            message_records(
                &format!("{package}.{}", message.name.as_deref().unwrap_or_default()),
                message,
                &mut records,
            )?;
        }
        for value in &file.enum_type {
            enum_records(
                &format!("{package}.{}", value.name.as_deref().unwrap_or_default()),
                value,
                &mut records,
            )?;
        }
        for service in &file.service {
            let name = format!("{package}.{}", service.name.as_deref().unwrap_or_default());
            records.insert(format!("service {name}"));
            for method in &service.method {
                records.insert(format!(
                    "method {name} {} {} {} client-stream={} server-stream={}",
                    method.name.as_deref().unwrap_or_default(),
                    method.input_type.as_deref().unwrap_or_default(),
                    method.output_type.as_deref().unwrap_or_default(),
                    method.client_streaming.unwrap_or_default(),
                    method.server_streaming.unwrap_or_default()
                ));
            }
        }
    }
    Ok(records)
}

fn check_ledger(bytes: &[u8], ledger: &BTreeSet<String>) -> Result<(), LedgerError> {
    if descriptor_records(bytes)? != *ledger {
        return Err(LedgerError::ContractChanged);
    }
    Ok(())
}

fn ledger() -> BTreeSet<String> {
    include_str!("../../df-protocol/proto/field-ledger.txt")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

#[test]
fn ledger_matches_actual_protoc_descriptor_for_all_allocated_names_numbers_and_presence() {
    let records = descriptor_records(df_protocol::FILE_DESCRIPTOR_SET).unwrap();
    assert_eq!(records, ledger());
}

fn common_revision(descriptor: &mut FileDescriptorSet) -> &mut DescriptorProto {
    descriptor
        .file
        .iter_mut()
        .find(|file| file.package.as_deref() == Some("dungeonflux.public.v1"))
        .unwrap()
        .message_type
        .iter_mut()
        .find(|message| message.name.as_deref() == Some("SessionRevision"))
        .unwrap()
}

#[test]
fn actual_descriptor_mutations_reject_field_reuse_removal_renaming_type_and_presence_changes() {
    for mutation in 0..6 {
        let mut descriptor = FileDescriptorSet::decode(df_protocol::FILE_DESCRIPTOR_SET).unwrap();
        let revision = common_revision(&mut descriptor);
        match mutation {
            0 => revision.field[1].number = Some(1),
            1 => {
                revision.field.remove(1);
            }
            2 => revision.field[1].name = Some("reused_sequence".to_owned()),
            3 => revision.field[1].r#type = Some(5),
            4 => revision.field[1].proto3_optional = Some(false),
            5 => revision.field[1].number = Some(3),
            _ => unreachable!(),
        }
        assert!(
            check_ledger(&descriptor.encode_to_vec(), &ledger()).is_err(),
            "mutation {mutation}"
        );
    }
    assert_eq!(
        check_ledger(&[10, 255][..], &ledger()),
        Err(LedgerError::MalformedDescriptor)
    );
}

#[test]
fn retired_number_and_name_remain_reserved_and_cannot_be_reallocated() {
    // Hypothetical future retirement exercises protoc descriptor ranges, not a cloned schema.
    let mut retired = FileDescriptorSet::decode(df_protocol::FILE_DESCRIPTOR_SET).unwrap();
    let revision = common_revision(&mut retired);
    revision.field.remove(1);
    revision.oneof_decl.clear();
    revision
        .reserved_range
        .push(prost_types::descriptor_proto::ReservedRange {
            start: Some(2),
            end: Some(3),
        });
    revision.reserved_name.push("sequence".to_owned());
    let retired_ledger = descriptor_records(&retired.encode_to_vec()).unwrap();
    assert!(check_ledger(&retired.encode_to_vec(), &retired_ledger).is_ok());
    for (number, name) in [(2, "different"), (3, "sequence")] {
        let mut reused = retired.clone();
        common_revision(&mut reused)
            .field
            .push(prost_types::FieldDescriptorProto {
                name: Some(name.to_owned()),
                number: Some(number),
                r#type: Some(4),
                label: Some(1),
                ..Default::default()
            });
        assert_eq!(
            check_ledger(&reused.encode_to_vec(), &retired_ledger),
            Err(LedgerError::ReservedAllocation)
        );
    }
    let mut forgotten = retired;
    common_revision(&mut forgotten).reserved_range.clear();
    assert_eq!(
        check_ledger(&forgotten.encode_to_vec(), &retired_ledger),
        Err(LedgerError::ContractChanged)
    );
}

/// Maintenance, not a check. `cargo test -p df-tools --test schema_ledger -- --ignored --nocapture
/// print_ledger_drift` lists what the descriptor has that the ledger lacks (`+`) and what the ledger
/// has that the descriptor no longer does (`-`, a removal that needs a `reserved` entry).
#[test]
#[ignore = "prints the drift between the descriptor and the ledger"]
fn print_ledger_drift() {
    let actual = descriptor_records(df_protocol::FILE_DESCRIPTOR_SET).unwrap();
    let ledger = ledger();
    for line in actual.difference(&ledger) {
        println!("+ {line}");
    }
    for line in ledger.difference(&actual) {
        println!("- {line}");
    }
}
