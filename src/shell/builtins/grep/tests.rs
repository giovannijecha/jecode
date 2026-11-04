use super::search::Lines;
use super::*;

#[test]
fn max_count_zero_does_not_wait_for_standard_input() {
    let (reader, writer) = io::pipe().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let child = std::thread::spawn(move || {
        let options = Options {
            max: Some(0),
            ..Options::default()
        };
        let regex = Regex::new(&["foo"], Syntax::Basic, Edges::Any, false).unwrap();
        let mut output = Vec::new();
        sender
            .send(search(
                &options,
                &regex,
                Box::new(reader),
                "-",
                false,
                &mut output,
                &mut false,
            ))
            .unwrap();
    });
    let completed = receiver.recv_timeout(std::time::Duration::from_secs(1));
    drop(writer);
    child.join().unwrap();
    assert!(!completed.unwrap().unwrap());
}

#[test]
fn binary_input_without_newlines_stays_bounded() {
    let mut input = vec![b'a'; 10];
    input.push(0);
    input.extend(vec![0; 4 * 1024 * 1024]);
    input.extend([b'b'; 10]);
    input.extend("\0c".repeat(1024 * 1024).bytes());
    let mut lines = Lines::new(Box::new(&input[..]), true, true);
    let mut line = Vec::new();
    let mut total = 0;
    while lines.next(&mut line).unwrap() {
        total += 1;
        assert!(line.capacity() <= 1024);
    }
    assert!(lines.binary());
    assert_eq!(total, 2 + 1024 * 1024);
}
