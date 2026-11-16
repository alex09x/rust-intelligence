/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The unchanged lines between two edited regions, matched as patience diff matches them: lines that occur exactly
//! once in each text are anchors, the longest run of them in the same order in both is kept, and identical lines
//! next to an anchor extend it. O(n log n), no quadratic table: a long file with many edits stays cheap.

use std::collections::HashMap;

/// Pairs `(baseline index, live index)` of the lines of `b` and `l` that stand for each other, in increasing order.
pub(crate) fn matched_lines(b: &[&str], l: &[&str]) -> Vec<(usize, usize)> {
    if b.is_empty() || l.is_empty() {
        return Vec::new();
    }
    // Lines once in each text, by baseline order.
    let mut seen: HashMap<&str, (u32, u32, usize, usize)> = HashMap::new();
    for (i, s) in b.iter().enumerate() {
        let e = seen.entry(s).or_insert((0, 0, i, 0));
        e.0 += 1;
    }
    for (j, s) in l.iter().enumerate() {
        if let Some(e) = seen.get_mut(s) {
            e.1 += 1;
            e.3 = j;
        }
    }
    let mut unique: Vec<(usize, usize)> = seen.values().filter(|e| e.0 == 1 && e.1 == 1).map(|e| (e.2, e.3)).collect();
    unique.sort_unstable();
    let anchors = longest_increasing_by_live(&unique);

    // Each anchor, and the identical lines after and before it up to the neighbouring anchors.
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut prev = (0usize, 0usize); // first indices not yet matched
    let bounds: Vec<(usize, usize)> = anchors.iter().copied().chain(std::iter::once((b.len(), l.len()))).collect();
    for &(ab, al) in &bounds {
        // Forward from the previous anchor.
        let (mut i, mut j) = prev;
        while i < ab && j < al && b[i] == l[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        }
        // Backward from this anchor, not past what the forward run took.
        let mut back = Vec::new();
        let (mut x, mut y) = (ab, al);
        while x > i && y > j && b[x - 1] == l[y - 1] {
            x -= 1;
            y -= 1;
            back.push((x, y));
        }
        out.extend(back.into_iter().rev());
        if ab < b.len() && al < l.len() {
            out.push((ab, al));
        }
        prev = (ab + 1, al + 1);
    }
    out
}

/// The longest subsequence of `pairs` (sorted by baseline index) whose live indices increase too.
fn longest_increasing_by_live(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut tails: Vec<usize> = Vec::new(); // index into `pairs` of the smallest tail of each length
    let mut parent = vec![usize::MAX; pairs.len()];
    for (k, &(_, live)) in pairs.iter().enumerate() {
        let pos = tails.partition_point(|&t| pairs[t].1 < live);
        if pos > 0 {
            parent[k] = tails[pos - 1];
        }
        if pos == tails.len() {
            tails.push(k);
        } else {
            tails[pos] = k;
        }
    }
    let mut out = Vec::with_capacity(tails.len());
    let mut k = tails.last().copied().unwrap_or(usize::MAX);
    while k != usize::MAX {
        out.push(pairs[k]);
        k = parent[k];
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::matched_lines;

    #[test]
    fn test_unchanged_lines_between_two_edits_match() {
        // Baseline: an import in `mod tests` and a line in a test that the live text no longer has.
        let b = ["    use std::collections::HashSet;", "", "    #[test]", "    fn a() {", "        x();", "    }", "        let _s = HashSet::new();", "    }"];
        let l = ["", "    #[test]", "    fn a() {", "        x();", "    }", "    }"];
        let m = matched_lines(&b, &l);
        assert!(m.contains(&(3, 2)) && m.contains(&(4, 3)), "`fn a` and its body: {m:?}");
        assert!(!m.iter().any(|&(i, _)| i == 0 || i == 6), "removed lines map to nothing: {m:?}");
        assert!(m.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 < w[1].1), "in order: {m:?}");
    }

    #[test]
    fn test_moved_and_repeated_lines() {
        let b = ["a", "}", "b", "}", "c"];
        let l = ["c", "a", "}", "b", "}"];
        let m = matched_lines(&b, &l);
        assert!(m.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 < w[1].1), "a monotone match: {m:?}");
        assert!(m.contains(&(0, 1)) && m.contains(&(2, 3)), "the longer ordered run wins over the moved `c`: {m:?}");
    }
}
