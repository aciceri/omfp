//! Line selectors on `omfp://` reads (DMN-378..380). omp 18.22.0 registers a host scheme with `selectors: "none"`, so
//! its read tool hands `omfp://docs/widgets:524` to the host as is and numbers whatever the host answers from line 1,
//! cutting it at 3000 lines / 50 KB with a `Use :N to continue` hint relative to that answer. The daemon therefore peels
//! omp's selector grammar itself, answers the selected lines and pages long answers below omp's limits, with its own
//! continuation notice in the document's line numbers.
use std::fmt::Write as _;

use omp_rpc::HostUriRead;

/// What one page of an answer holds at most, below omp's read limits (3000 lines, 50 KB), so omp never cuts it.
pub const PAGE_LINES: usize = 300;
/// See [`PAGE_LINES`].
pub const PAGE_BYTES: usize = 32 * 1024;

/// The selectors an `omfp://` read takes, for errors.
pub const ACCEPTED: &str = "`:N` (from line N), `:N-M`, `:N+K` (K lines from N), `:N-` (from N), `:-N` (the last N \
                            lines), a comma list (`:5-16,40-80`), `:raw` and a range with it (`:raw:N-M`)";

/// One inclusive range of 1-based lines; `end` `None` runs to the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: usize,
    pub end: Option<usize>,
}

/// A parsed selector: the ranges (empty: the whole text), the last `tail` lines when non-zero, and `raw`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selector {
    pub ranges: Vec<Range>,
    pub tail: usize,
    pub raw: bool,
}

/// Whether `chunk` is shaped like a selector part (so a malformed one is an error, not part of the URL).
fn looks_like_selector(chunk: &str) -> bool {
    !chunk.is_empty()
        && (chunk.eq_ignore_ascii_case("raw")
            || chunk.eq_ignore_ascii_case("conflicts")
            || chunk.eq_ignore_ascii_case("img")
            || chunk
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | ',' | '.' | 'L' | 'l'))
                && chunk.chars().any(|c| c.is_ascii_digit()))
}

/// Splits a trailing selector off `url` (up to two `:` chunks, as omp's `:raw:50-100`): `(base, selector)`.
#[must_use]
pub fn split(url: &str) -> (&str, Option<&str>) {
    let Some(path_start) = url.find("://").map(|i| i + 3) else {
        return (url, None);
    };
    let mut base = url.len();
    for _ in 0..2 {
        let Some(colon) = url[path_start..base].rfind(':').map(|i| i + path_start) else {
            break;
        };
        if !looks_like_selector(&url[colon + 1..base]) {
            break;
        }
        base = colon;
    }
    if base == url.len() {
        (url, None)
    } else {
        (&url[..base], Some(&url[base + 1..]))
    }
}

fn invalid(selector: &str, why: &str) -> String {
    format!("invalid selector `:{selector}`{why}: omfp:// reads take {ACCEPTED}")
}

fn number(text: &str, selector: &str) -> Result<usize, String> {
    let text = text.strip_prefix(['L', 'l']).unwrap_or(text);
    let value: usize = text.parse().map_err(|_| invalid(selector, ""))?;
    if value == 0 {
        return Err(invalid(selector, " (lines start at 1)"));
    }
    Ok(value)
}

/// One `N`, `N-`, `N-M`, `N..M` or `N+K` chunk; a bare `N` is line N alone when `single`, else from N on.
fn chunk(text: &str, single: bool, selector: &str) -> Result<Range, String> {
    let (start, rest) = match text.find(['-', '+', '.']) {
        Some(at) => (&text[..at], Some(&text[at..])),
        None => (text, None),
    };
    let start = number(start, selector)?;
    let end = match rest {
        None => single.then_some(start),
        Some("-" | "..") => None,
        Some(rest) => {
            if let Some(count) = rest.strip_prefix('+') {
                let end = start
                    .checked_add(number(count, selector)? - 1)
                    .ok_or_else(|| invalid(selector, " (the range is too long)"))?;
                Some(end)
            } else {
                let end = rest
                    .strip_prefix("..")
                    .or_else(|| rest.strip_prefix('-'))
                    .ok_or_else(|| invalid(selector, ""))?;
                let end = number(end, selector)?;
                if end < start {
                    return Err(invalid(selector, " (the end comes before the start)"));
                }
                Some(end)
            }
        }
    };
    Ok(Range { start, end })
}

/// Parses omp's read selector grammar (`tools/read-selector.ts` of omp 18.22.0) minus what needs a file (`:img`,
/// `:conflicts`).
///
/// # Errors
/// A selector outside the grammar, line 0 or a range ending before it starts, naming the accepted selectors.
pub fn parse(selector: &str) -> Result<Selector, String> {
    let chunks: Vec<&str> = selector.split(':').collect();
    let (lines, raw) = match chunks.as_slice() {
        [one] if one.eq_ignore_ascii_case("raw") => {
            return Ok(Selector {
                raw: true,
                ..Selector::default()
            });
        }
        [one] => (*one, false),
        [a, b] if a.eq_ignore_ascii_case("raw") && !b.eq_ignore_ascii_case("raw") => (*b, true),
        [a, b] if b.eq_ignore_ascii_case("raw") && !a.eq_ignore_ascii_case("raw") => (*a, true),
        _ => return Err(invalid(selector, "")),
    };
    let parts: Vec<&str> = lines.split(',').collect();
    let single = raw || parts.len() > 1;
    let mut parsed = Selector {
        raw,
        ..Selector::default()
    };
    for part in parts {
        if let Some(count) = part.strip_prefix('-') {
            parsed.tail = parsed.tail.max(number(count, selector)?);
        } else {
            parsed.ranges.push(chunk(part, single, selector)?);
        }
    }
    Ok(parsed)
}

/// `parsed` against a text of `total` lines: sorted, merged ranges, the tail pinned to the last lines; empty for the
/// whole text.
fn resolve(parsed: &Selector, total: usize) -> Vec<Range> {
    let mut ranges = parsed.ranges.clone();
    if parsed.tail > 0 {
        ranges.push(Range {
            start: total.saturating_sub(parsed.tail) + 1,
            end: Some(total),
        });
    }
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last)
                if last
                    .end
                    .is_none_or(|end| range.start <= end.saturating_add(1)) =>
            {
                last.end = match (last.end, range.end) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    _ => None,
                };
            }
            _ => merged.push(range),
        }
    }
    merged
}

/// The selector that reads `remaining` (the rest of the range paging stopped in, then the ranges after it), in the
/// form the caller used.
fn continuation(base: &str, remaining: &[Range], raw: bool) -> String {
    let lines = match remaining {
        [Range { start, end: None }] if !raw => start.to_string(),
        _ => remaining
            .iter()
            .map(|range| match range.end {
                Some(end) => format!("{}-{end}", range.start),
                None => format!("{}-", range.start),
            })
            .collect::<Vec<_>>()
            .join(","),
    };
    if raw {
        format!("{base}:raw:{lines}")
    } else {
        format!("{base}:{lines}")
    }
}

/// `read`'s answer to `base` with `selector` applied (DMN-378, DMN-379): the selected lines, paged at [`PAGE_LINES`] /
/// [`PAGE_BYTES`]. A whole text that fits comes back untouched; otherwise the answer ends with a notice giving the
/// lines it holds in the document's numbering and the selector that reads on.
///
/// # Errors
/// An invalid selector, a first line past the end, or `read`'s own error.
pub fn read(
    base: &str,
    selector: Option<&str>,
    read: impl FnOnce(&str) -> Result<HostUriRead, String>,
) -> Result<HostUriRead, String> {
    let parsed = selector.map(parse).transpose()?.unwrap_or_default();
    let mut answer = read(base)?;
    let lines: Vec<&str> = answer.content.lines().collect();
    let total = lines.len();
    let mut ranges = resolve(&parsed, total);
    let whole = ranges.is_empty();
    if whole {
        if total == 0 {
            return Ok(answer);
        }
        ranges.push(Range {
            start: 1,
            end: None,
        });
    }
    if let Some(range) = ranges.iter().find(|range| range.start > total) {
        return Err(format!(
            "line {} is past the end of {base} ({total} lines): read {base}:1 or {base}:-50",
            range.start
        ));
    }
    let multiple = ranges.len() > 1;
    let mut text = String::new();
    let mut count = 0;
    let mut stop = None;
    let mut shown = Vec::new();
    'ranges: for (index, range) in ranges.iter().enumerate() {
        let end = range.end.map_or(total, |end| end.min(total));
        let header = text.len();
        if multiple {
            let _ = writeln!(text, "[{base} lines {}-{end} of {total}]", range.start);
        }
        for line in range.start..=end {
            let content = lines[line - 1];
            // The page's bytes are everything written so far, range headers included.
            if count > 0 && (count == PAGE_LINES || text.len() + content.len() + 1 > PAGE_BYTES) {
                let mut remaining = vec![Range {
                    start: line,
                    end: range.end,
                }];
                remaining.extend_from_slice(&ranges[index + 1..]);
                stop = Some(remaining);
                if line > range.start {
                    shown.push((range.start, line - 1));
                } else {
                    text.truncate(header);
                }
                break 'ranges;
            }
            text.push_str(content);
            text.push('\n');
            count += 1;
        }
        shown.push((range.start, end));
    }
    if whole && stop.is_none() {
        return Ok(answer);
    }
    let mut notice = None;
    if let Some(remaining) = stop {
        let held: Vec<String> = shown.iter().map(|(a, b)| format!("{a}-{b}")).collect();
        notice = Some(format!(
            "[omfp: this answer holds lines {} of {total} of {base}{}; read {} to continue]",
            held.join(", "),
            if shown.first().is_some_and(|(a, _)| *a > 1) || multiple {
                " (omp numbers the answer from 1)"
            } else {
                ""
            },
            continuation(base, &remaining, parsed.raw)
        ));
    } else if !parsed.raw
        && !multiple
        && let Some((start, end)) = shown.first().filter(|(start, _)| *start > 1)
    {
        notice = Some(format!(
            "[omfp: this answer holds lines {start}-{end} of {total} of {base} (omp numbers the answer from 1)]"
        ));
    }
    if let Some(notice) = notice {
        text.push('\n');
        text.push_str(&notice);
        text.push('\n');
    }
    answer.content = text;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{DOCS, DOCS_URL, read_uri};

    fn docs(selector: Option<&str>) -> Result<String, String> {
        read(DOCS_URL, selector, |url| read_uri(url, None)).map(|read| read.content)
    }

    fn doc_lines() -> Vec<&'static str> {
        DOCS.lines().collect()
    }

    // spec: DMN-378
    #[test]
    fn splits_the_selectors_omp_suggests_off_the_url() {
        for (url, base, selector) in [
            ("omfp://docs/widgets", "omfp://docs/widgets", None),
            (
                "omfp://docs/widgets:524",
                "omfp://docs/widgets",
                Some("524"),
            ),
            (
                "omfp://docs/widgets:raw:5-9",
                "omfp://docs/widgets",
                Some("raw:5-9"),
            ),
            (
                "omfp://docs/widgets:5-9:raw",
                "omfp://docs/widgets",
                Some("5-9:raw"),
            ),
            (
                "omfp://docs/widgets:-40",
                "omfp://docs/widgets",
                Some("-40"),
            ),
            (
                "omfp://docs/widgets:5-16,40-80",
                "omfp://docs/widgets",
                Some("5-16,40-80"),
            ),
            ("omfp://docs/widgets:0", "omfp://docs/widgets", Some("0")),
            (
                "omfp://widgets/check/user:init",
                "omfp://widgets/check/user:init",
                None,
            ),
            (
                "omfp://widgets/check/user:init:9",
                "omfp://widgets/check/user:init",
                Some("9"),
            ),
        ] {
            assert_eq!(split(url), (base, selector), "{url}");
        }
    }

    // spec: DMN-378
    #[test]
    fn parses_omps_selector_grammar() {
        let range = |start, end| Range { start, end };
        assert_eq!(parse("524").unwrap().ranges, [range(524, None)]);
        assert_eq!(parse("5-9").unwrap().ranges, [range(5, Some(9))]);
        assert_eq!(parse("5..9").unwrap().ranges, [range(5, Some(9))]);
        assert_eq!(parse("5+3").unwrap().ranges, [range(5, Some(7))]);
        assert_eq!(parse("5-").unwrap().ranges, [range(5, None)]);
        assert_eq!(
            parse("19,59").unwrap().ranges,
            [range(19, Some(19)), range(59, Some(59))]
        );
        let tail = parse("1-3,-15").unwrap();
        assert_eq!((tail.ranges, tail.tail), (vec![range(1, Some(3))], 15));
        let raw = parse("raw:7").unwrap();
        assert!(raw.raw);
        assert_eq!(raw.ranges, [range(7, Some(7))]);
        assert!(parse("RAW").unwrap().raw && parse("RAW").unwrap().ranges.is_empty());
        for bad in [
            "0",
            "9-5",
            "5+0",
            "raw:raw",
            "conflicts",
            "img",
            "1:2",
            "x-y",
            "2+18446744073709551615",
        ] {
            let error = parse(bad).unwrap_err();
            assert!(
                error.contains(&format!("`:{bad}`")) && error.contains(":raw:N-M"),
                "{error}"
            );
        }
    }

    // spec: DMN-379
    #[test]
    fn pages_the_widget_docs_and_continues_where_the_notice_says() {
        let all = doc_lines();
        assert!(
            all.len() > PAGE_LINES || DOCS.len() > PAGE_BYTES,
            "the docs fit one page now"
        );
        let mut read_back = Vec::new();
        let mut selector: Option<String> = None;
        for _ in 0..all.len() {
            let page = docs(selector.as_deref()).unwrap();
            assert!(
                page.len() <= PAGE_BYTES + 512 && page.lines().count() <= PAGE_LINES + 2,
                "{}",
                page.len()
            );
            // The answer is the lines, each ending in `\n`, then `\n[omfp: …]\n`: a page may end on a blank line.
            let (body, notice) = match page.rsplit_once("\n[omfp: ") {
                Some((body, notice)) if notice.starts_with("this answer holds") => {
                    (body, Some(notice))
                }
                _ => (page.as_str(), None),
            };
            read_back.extend(body.lines().map(str::to_owned));
            let next = notice.and_then(|notice| notice.split(&format!("read {DOCS_URL}:")).nth(1));
            match next {
                Some(next) => {
                    selector = Some(next.trim_end().trim_end_matches(" to continue]").to_owned());
                }
                None => break,
            }
        }
        assert_eq!(read_back, all, "the pages put together are the docs");
        let first = docs(None).unwrap();
        assert!(first.starts_with(all[0]));
        assert!(
            first.contains("holds lines 1-") && !first.contains("numbers the answer from 1"),
            "a first page numbers like the docs: {}",
            &first[first.len() - 200..]
        );
    }

    // spec: DMN-379
    #[test]
    fn answers_ranges_tails_lists_and_raw() {
        let all = doc_lines();
        let range = docs(Some("10-12")).unwrap();
        assert!(
            range.starts_with(&format!("{}\n{}\n{}\n\n", all[9], all[10], all[11])),
            "{range}"
        );
        assert!(
            range.contains(&format!(
                "lines 10-12 of {} of {DOCS_URL} (omp numbers",
                all.len()
            )),
            "{range}"
        );
        assert_eq!(docs(Some("10+3")).unwrap(), range);
        assert_eq!(
            docs(Some("raw:10-12")).unwrap(),
            format!("{}\n{}\n{}\n", all[9], all[10], all[11])
        );
        assert_eq!(docs(Some("raw:10")).unwrap(), format!("{}\n", all[9]));
        let tail = docs(Some("-2")).unwrap();
        assert!(
            tail.starts_with(&format!("{}\n{}\n", all[all.len() - 2], all[all.len() - 1])),
            "{tail}"
        );
        let list = docs(Some("2,4-5")).unwrap();
        assert_eq!(
            list,
            format!(
                "[{DOCS_URL} lines 2-2 of {total}]\n{}\n[{DOCS_URL} lines 4-5 of {total}]\n{}\n{}\n",
                all[1],
                all[3],
                all[4],
                total = all.len()
            )
        );
        let raw_whole = docs(Some("raw")).unwrap();
        assert!(
            raw_whole.starts_with(all[0]) && raw_whole.contains(&format!("read {DOCS_URL}:raw:")),
            "{raw_whole}"
        );
        let past = docs(Some(&(all.len() + 1).to_string())).unwrap_err();
        assert!(past.contains("past the end"), "{past}");
        assert!(docs(Some("0")).unwrap_err().contains(ACCEPTED));
    }

    // spec: DMN-379
    #[test]
    fn a_paged_comma_list_continues_with_every_range_left() {
        let all = doc_lines();
        let later = all.len() - 5;
        let selector = format!("1-400,{later}-{}", later + 2);
        let first = docs(Some(&selector)).unwrap();
        let next = first
            .split(&format!("read {DOCS_URL}:"))
            .nth(1)
            .and_then(|rest| rest.strip_suffix(" to continue]\n"))
            .unwrap_or_else(|| panic!("no continuation: {first}"));
        assert!(next.ends_with(&format!(",{later}-{}", later + 2)), "{next}");
        let rest = docs(Some(next)).unwrap();
        assert!(
            rest.contains(&format!(
                "{}\n{}\n{}\n",
                all[later - 1],
                all[later],
                all[later + 1]
            )),
            "{rest}"
        );
        assert!(!rest.contains("to continue]"), "{rest}");
    }

    // spec: DMN-379
    #[test]
    fn a_short_text_comes_back_untouched() {
        let read = read("omfp://schemas/views.json", None, |url| read_uri(url, None)).unwrap();
        assert_eq!(
            read.content,
            read_uri("omfp://schemas/views.json", None).unwrap().content
        );
    }

    // spec: DMN-379
    #[test]
    fn an_empty_document_reads_whole() {
        let read = read("omfp://widgets/builtin/x/empty.ts", None, |_| {
            Ok(String::new().into())
        })
        .unwrap();
        assert_eq!(read.content, "");
    }

    // spec: DMN-379
    #[test]
    fn range_headers_count_in_the_page_budget() {
        let base = format!("omfp://docs/widgets{}", "/".repeat(2000));
        let text = (1..=600)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let odd: Vec<String> = (1..=600).step_by(2).map(|n| n.to_string()).collect();
        let page = read(&base, Some(&odd.join(",")), |_| Ok(text.clone().into()))
            .unwrap()
            .content;
        let body = page.split("\n[omfp: ").next().unwrap_or_default();
        assert!(body.len() <= PAGE_BYTES, "{} bytes", body.len());
        assert!(page.contains("to continue]"), "{page}");
    }

    // spec: DMN-380
    #[test]
    fn the_unknown_url_error_lists_the_selectors() {
        let error = read("omfp://docs/nothing", Some("5"), |url| read_uri(url, None)).unwrap_err();
        assert!(
            error.starts_with("unknown omfp URL omfp://docs/nothing:") && error.ends_with(ACCEPTED),
            "{error}"
        );
    }
}
