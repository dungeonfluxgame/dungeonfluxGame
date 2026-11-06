use crate::atomic_file::{temporary_path, write_atomically};

fn scratch_directory(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "df-table-host-atomic-file-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

fn hidden_leftovers(directory: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.'))
        .collect()
}

#[test]
fn two_temporary_paths_for_the_same_name_differ() {
    let directory = std::path::Path::new("layouts");
    assert_ne!(
        temporary_path(directory, "plate.json"),
        temporary_path(directory, "plate.json")
    );
}

#[test]
fn a_write_creates_the_directory_and_leaves_only_the_target() {
    let directory = scratch_directory("creates");
    write_atomically(&directory, "plate.json", b"{}").unwrap();
    assert_eq!(std::fs::read(directory.join("plate.json")).unwrap(), b"{}");
    assert!(hidden_leftovers(&directory).is_empty());
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn concurrent_writes_of_one_name_each_land_whole_and_leave_no_temporary_file() {
    let directory = scratch_directory("concurrent");
    let texts: Vec<Vec<u8>> = (0..8_u8).map(|index| vec![b'a' + index; 4096]).collect();
    std::thread::scope(|scope| {
        for text in &texts {
            let directory = &directory;
            scope.spawn(move || write_atomically(directory, "plate.json", text));
        }
    });
    let landed = std::fs::read(directory.join("plate.json")).unwrap();
    assert!(
        texts.contains(&landed),
        "the file is one of the texts, whole"
    );
    assert!(hidden_leftovers(&directory).is_empty());
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_failed_rename_removes_the_temporary_file() {
    let directory = scratch_directory("failed");
    let occupied = directory.join("plate.json");
    std::fs::create_dir_all(occupied.join("inside")).unwrap();
    assert!(write_atomically(&directory, "plate.json", b"{}").is_err());
    assert!(hidden_leftovers(&directory).is_empty());
    std::fs::remove_dir_all(&directory).unwrap();
}
