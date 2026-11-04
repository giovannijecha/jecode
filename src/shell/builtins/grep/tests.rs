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
            ))
            .unwrap();
    });
    let completed = receiver.recv_timeout(std::time::Duration::from_secs(1));
    drop(writer);
    child.join().unwrap();
    assert!(!completed.unwrap().unwrap());
}
