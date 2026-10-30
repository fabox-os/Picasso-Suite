//! Picasso Suite change: reading the `text/uri-list` a file manager offers for a drag and drop.
//!
//! Upstream winit read the whole list or none of it: one line it could not use (a name that is not
//! UTF-8, a `file://localhost/` address, a stray NUL at the end, a path that is gone) made the drop
//! silent, with no event at all. Here each line stands alone: the usable ones are kept and the
//! others are named in the second part of the answer, so they can be logged.
//!
//! This file depends on nothing of X11, so it can be tested alone.

use percent_encoding::percent_decode;

/// The paths (as bytes, which Unix paths are) of the `file:` lines of `data`, and why each line
/// that was left out was left out.
pub fn uri_list_paths(data: &[u8]) -> (Vec<Vec<u8>>, Vec<String>) {
    let mut paths = Vec::new();
    let mut left_out = Vec::new();
    // Lines end with CR LF; a bare LF is read the same, and a NUL some sources add is dropped.
    for line in data.split(|b| *b == b'\n') {
        let line = trim(line);
        // RFC 2483: an empty line or a line starting with # is not an address.
        if line.is_empty() || line.starts_with(b"#") {
            continue;
        }
        let Some(rest) = line.strip_prefix(b"file://") else {
            left_out.push(format!("not a file address: {}", String::from_utf8_lossy(line)));
            continue;
        };
        // `file:///path`, or `file://localhost/path`; another host is a file on another computer.
        let path = if rest.starts_with(b"/") {
            rest
        } else {
            match rest.iter().position(|b| *b == b'/') {
                Some(slash) if rest[..slash].eq_ignore_ascii_case(b"localhost") => &rest[slash..],
                _ => {
                    left_out.push(format!(
                        "on another computer: {}",
                        String::from_utf8_lossy(line)
                    ));
                    continue;
                },
            }
        };
        let decoded: Vec<u8> = percent_decode(path).collect();
        if decoded.contains(&0) {
            left_out.push(format!("a NUL in the name: {}", String::from_utf8_lossy(line)));
            continue;
        }
        paths.push(decoded);
    }
    (paths, left_out)
}

/// `line` without the CR, NUL and spaces around it.
fn trim(mut line: &[u8]) -> &[u8] {
    let junk = |b: &u8| matches!(*b, b'\r' | 0 | b' ' | b'\t');
    while line.first().is_some_and(junk) {
        line = &line[1..];
    }
    while line.last().is_some_and(junk) {
        line = &line[..line.len() - 1];
    }
    line
}

#[cfg(test)]
mod tests {
    use super::uri_list_paths;

    fn paths(data: &[u8]) -> Vec<String> {
        uri_list_paths(data).0.iter().map(|p| String::from_utf8_lossy(p).into_owned()).collect()
    }

    #[test]
    fn the_usual_list_is_read_as_before() {
        let data = b"file:///home/a/one.mp4\r\nfile:///home/a/two%20clips.mov\r\n";
        assert_eq!(paths(data), ["/home/a/one.mp4", "/home/a/two clips.mov"]);
        assert!(uri_list_paths(data).1.is_empty());
    }

    #[test]
    fn a_nul_at_the_end_and_a_bare_lf_do_not_spoil_the_list() {
        assert_eq!(paths(b"file:///a/b.mp4\0"), ["/a/b.mp4"]);
        assert_eq!(paths(b"file:///a/b.mp4\nfile:///a/c.mp4\n"), ["/a/b.mp4", "/a/c.mp4"]);
        assert_eq!(paths(b"file:///a/b.mp4\r\n\0"), ["/a/b.mp4"]);
    }

    #[test]
    fn localhost_is_this_computer_and_another_host_is_left_out() {
        assert_eq!(paths(b"file://localhost/a/b.mp4\r\n"), ["/a/b.mp4"]);
        assert_eq!(paths(b"file://LOCALHOST/a/b.mp4\r\n"), ["/a/b.mp4"]);
        let (found, left_out) = uri_list_paths(b"file://nas/a/b.mp4\r\nfile:///a/c.mp4\r\n");
        assert_eq!(found, vec![b"/a/c.mp4".to_vec()]);
        assert_eq!(left_out.len(), 1);
        assert!(left_out[0].contains("another computer"), "{left_out:?}");
    }

    #[test]
    fn one_line_that_cannot_be_used_does_not_take_the_others_with_it() {
        let data = b"file:///a/1.mp4\r\nsmb://nas/x.mp4\r\nfile:///a/2.mp4\r\nfile:///a/3%00.mp4\r\nfile:///a/4.mp4\r\n";
        let (found, left_out) = uri_list_paths(data);
        assert_eq!(found, vec![b"/a/1.mp4".to_vec(), b"/a/2.mp4".to_vec(), b"/a/4.mp4".to_vec()]);
        assert_eq!(left_out.len(), 2, "{left_out:?}");
    }

    #[test]
    fn a_name_that_is_not_utf8_is_kept_byte_for_byte() {
        let (found, left_out) = uri_list_paths(b"file:///a/caf%E9.mp4\r\n");
        assert_eq!(found, vec![b"/a/caf\xE9.mp4".to_vec()]);
        assert!(left_out.is_empty());
    }

    #[test]
    fn comments_blank_lines_and_an_empty_list_give_nothing() {
        assert!(uri_list_paths(b"").0.is_empty());
        assert!(uri_list_paths(b"\r\n\r\n").0.is_empty());
        let (found, left_out) = uri_list_paths(b"# a comment\r\n");
        assert!(found.is_empty() && left_out.is_empty());
    }

    #[test]
    fn a_big_batch_is_read_whole() {
        let mut data = Vec::new();
        for i in 0..500 {
            data.extend_from_slice(format!("file:///media/drive/Footage%20{i}/clip_{i:04}.mov\r\n").as_bytes());
        }
        let (found, left_out) = uri_list_paths(&data);
        assert_eq!(found.len(), 500);
        assert!(left_out.is_empty());
        assert_eq!(found[499], b"/media/drive/Footage 499/clip_0499.mov".to_vec());
    }
}
