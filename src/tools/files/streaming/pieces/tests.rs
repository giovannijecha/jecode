use super::*;
use crate::test_support::Directory;

fn contents(view: &View<'_>, file: &mut File, chunk: usize) -> String {
    let mut reader = view.reader(file);
    let mut bytes = Vec::new();
    let mut buffer = vec![0; chunk];
    loop {
        let count = reader.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    assert_eq!(bytes.len() as u64, view.len());
    String::from_utf8(bytes).unwrap()
}

#[test]
fn sequential_ranges_match_in_memory_edits_across_original_and_inserted_pieces() {
    let directory = Directory::new();
    let path = directory.path().join("source.txt");
    let original = "αbeta🐟gamma\n";
    std::fs::write(&path, original).unwrap();
    let mut file = File::open(path).unwrap();
    let mut view = View::new(original.len() as u64);
    let mut expected = original.to_string();
    let mut seed = 17u64;
    for step in 0..300 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let boundaries: Vec<_> = expected
            .char_indices()
            .map(|(position, _)| position)
            .chain(std::iter::once(expected.len()))
            .collect();
        let first = seed as usize % boundaries.len();
        let last = first + (seed >> 32) as usize % (boundaries.len() - first);
        let start = boundaries[first];
        let end = boundaries[last];
        let inserted = ["", "new", "🦀é", "\r\n", "longer replacement"][step % 5];
        view.replace(start as u64, (end - start) as u64, inserted.as_bytes())
            .unwrap();
        expected.replace_range(start..end, inserted);
        assert_eq!(contents(&view, &mut file, step % 7 + 1), expected);
    }
    assert_eq!(
        std::fs::read_to_string(directory.path().join("source.txt")).unwrap(),
        original
    );
}

#[test]
fn deletion_of_every_piece_and_insertion_into_an_empty_view_are_readable() {
    let directory = Directory::new();
    let path = directory.path().join("source.txt");
    std::fs::write(&path, "abc").unwrap();
    let mut file = File::open(path).unwrap();
    let mut view = View::new(3);
    view.replace(1, 1, b"insert").unwrap();
    view.replace(0, view.len(), b"").unwrap();
    assert_eq!(contents(&view, &mut file, 1), "");
    view.replace(0, 0, "é".as_bytes()).unwrap();
    assert_eq!(contents(&view, &mut file, 1), "é");
}

#[test]
fn a_truncated_original_is_an_error_instead_of_a_partial_replacement() {
    let directory = Directory::new();
    let path = directory.path().join("source.txt");
    std::fs::write(&path, "original").unwrap();
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let mut view = View::new(8);
    view.replace(0, 1, b"changed").unwrap();
    file.set_len(2).unwrap();
    let mut bytes = Vec::new();
    assert_eq!(
        view.reader(&mut file)
            .read_to_end(&mut bytes)
            .unwrap_err()
            .kind(),
        io::ErrorKind::UnexpectedEof
    );
}
