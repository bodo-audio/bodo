//! `pack` and `unpack` turn a `.bodo` file into a `.bodoproj` folder and back without losing anything.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

#[test]
fn pack_and_unpack_round_trip() {
    let dir = common::temp_dir("pack");
    let (session, file) = common::saved_beat(&dir, "Beat.bodo");
    let folder = dir.join("Beat.bodoproj");
    let packed = dir.join("Packed.bodo");
    bodo_io::convert(&file, &folder).unwrap();
    bodo_io::convert(&folder, &packed).unwrap();
    assert!(folder.join("project.json").is_file());
    for path in [&folder, &packed] {
        assert_eq!(bodo_io::load(path).unwrap(), **session.project(), "{}", path.display());
    }
    assert!(bodo_io::convert(&file, &folder).is_err(), "never overwrites");
    std::fs::remove_dir_all(dir).unwrap();
}
