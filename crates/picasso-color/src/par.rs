//! Running a pass over a picture on every core, where the platform has threads.

/// Splits `buf` (`rows` rows of `row_len` bytes) into bands and calls `f(first_row, band)` for
/// each, on every core. On one core, or where threads do not exist (the browser build), `f` is
/// called once for the whole buffer, on the calling thread.
pub fn par_rows(
    buf: &mut [u8],
    row_len: usize,
    rows: usize,
    f: &(dyn Fn(usize, &mut [u8]) + Sync),
) {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(16);
    if threads <= 1 || rows < 2 || row_len == 0 {
        f(0, buf);
        return;
    }
    let per = rows.div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (i, chunk) in buf.chunks_mut(per * row_len).enumerate() {
            scope.spawn(move || f(i * per, chunk));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_visited_once_with_its_number() {
        let (w, h) = (7usize, 101usize);
        let mut buf = vec![0u8; w * h];
        par_rows(&mut buf, w, h, &|first, band| {
            for (r, row) in band.chunks_mut(w).enumerate() {
                row.fill(((first + r) % 251) as u8);
            }
        });
        for y in 0..h {
            assert!(
                buf[y * w..(y + 1) * w]
                    .iter()
                    .all(|&v| v == (y % 251) as u8),
                "row {y}"
            );
        }
    }

    #[test]
    fn empty_and_tiny_buffers_are_fine() {
        par_rows(&mut [], 4, 0, &|_, _| {});
        par_rows(&mut [1, 2, 3, 4], 4, 1, &|f, b| {
            assert_eq!((f, b.len()), (0, 4));
        });
        par_rows(&mut [1, 2], 0, 0, &|_, _| {});
    }
}
