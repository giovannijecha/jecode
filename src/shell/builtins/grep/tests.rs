use super::search::Lines;
use super::*;
use crate::shell::{Sink, Source};

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

#[test]
fn buffered_output_error_sets_failure_status() {
    let directory = crate::test_support::Directory::new();
    fs::write(directory.path().join("input"), "hit\n").unwrap();
    let mut shell = Shell::new().unwrap();
    shell.cwd = directory.path().to_path_buf();

    for silent in [false, true] {
        let io = Io {
            stdin: Source::Null,
            stdout: Sink::File(fs::File::open(directory.path().join("input")).unwrap()),
            stderr: Sink::Null,
        };
        let mut args = vec!["grep".to_owned(), "hit".to_owned(), "input".to_owned()];
        if silent {
            args.insert(1, "-s".to_owned());
        }
        let (status, _, stderr) = grep(&mut shell, &args, &io).unwrap();
        assert_eq!(status, 2);
        if silent {
            assert!(stderr.is_empty());
        } else {
            assert!(
                String::from_utf8(stderr)
                    .unwrap()
                    .starts_with("grep: output: ")
            );
        }
        assert!(!shell.broken_output);
    }
}

#[test]
fn buffered_broken_pipe_marks_output_closed() {
    let directory = crate::test_support::Directory::new();
    fs::write(directory.path().join("input"), "hit\n").unwrap();
    let mut shell = Shell::new().unwrap();
    shell.cwd = directory.path().to_path_buf();
    let (reader, writer) = io::pipe().unwrap();
    drop(reader);
    let io = Io {
        stdin: Source::Null,
        stdout: Sink::Pipe(writer),
        stderr: Sink::Null,
    };

    let (status, _, stderr) = grep(
        &mut shell,
        &["grep".into(), "hit".into(), "input".into()],
        &io,
    )
    .unwrap();
    assert_eq!(status, 0);
    assert!(stderr.is_empty());
    assert!(shell.broken_output);
}
