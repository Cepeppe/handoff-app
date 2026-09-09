//! The diff behind the **Show** control of the consent screen (INST-01, §7.15).
//!
//! INST-01 says the user is shown "each exact modification first (file, lines)". This is
//! the *lines* half: the value at one location of one file, as it is now and as it will be,
//! rendered so that what changes is obvious and what does not is visibly untouched.
//!
//! It is deliberately not a general diff algorithm. The three changes an adapter makes are
//! "a key appears", "an entry is appended to an array" and "an entry is removed from an
//! array", and for all three the common prefix and the common suffix are exactly the
//! context the user wants: the hooks they already had stay as unmarked context lines, and
//! only ours carry a `+`. An LCS diff would produce the same output for these shapes and
//! bring an algorithm nobody can check by eye.
//!
//! The output is text, not markup: it goes through the channel to a webview that renders it
//! in a `<pre>`, so the prefixes are the whole vocabulary. Line endings are `\n` on both
//! platforms — this is a rendering, never a file.

/// The unified-style rendering of `before` → `after` at `location`.
///
/// `before` is `None` when the value is not there at all, which is the ordinary case of a
/// first installation; the header says so, because an empty "before" pane and a missing one
/// are different facts.
#[must_use]
pub fn render(location: &str, before: Option<&str>, after: &str) -> String {
    let mut out = String::new();
    match before {
        Some(_) => out.push_str(&format!("--- {location}\n+++ {location}\n")),
        None => out.push_str(&format!("--- {location} (absent)\n+++ {location}\n")),
    }

    let before_lines: Vec<&str> = before
        .map(str::lines)
        .map(Iterator::collect)
        .unwrap_or_default();
    let after_lines: Vec<&str> = after.lines().collect();

    let common_start = before_lines
        .iter()
        .zip(after_lines.iter())
        .take_while(|(left, right)| left == right)
        .count();
    // The suffix may not overlap the prefix, or a line would be printed twice.
    let remaining = before_lines
        .len()
        .min(after_lines.len())
        .saturating_sub(common_start);
    let common_end = before_lines
        .iter()
        .rev()
        .zip(after_lines.iter().rev())
        .take(remaining)
        .take_while(|(left, right)| left == right)
        .count();

    for line in &before_lines[..common_start] {
        out.push_str(&format!(" {line}\n"));
    }
    for line in &before_lines[common_start..before_lines.len() - common_end] {
        out.push_str(&format!("-{line}\n"));
    }
    for line in &after_lines[common_start..after_lines.len() - common_end] {
        out.push_str(&format!("+{line}\n"));
    }
    for line in &after_lines[after_lines.len() - common_end..] {
        out.push_str(&format!(" {line}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_that_is_not_there_yet_is_all_addition() {
        assert_eq!(
            render("a.json · key", None, "{\n  \"a\": 1\n}"),
            "--- a.json · key (absent)\n+++ a.json · key\n+{\n+  \"a\": 1\n+}\n"
        );
    }

    #[test]
    fn an_entry_appended_to_an_array_leaves_the_existing_ones_as_context() {
        let before = "[\n  \"theirs\"\n]";
        let after = "[\n  \"theirs\",\n  \"ours\"\n]";
        assert_eq!(
            render("settings.json · hooks.Stop", Some(before), after),
            concat!(
                "--- settings.json · hooks.Stop\n",
                "+++ settings.json · hooks.Stop\n",
                " [\n",
                "-  \"theirs\"\n",
                "+  \"theirs\",\n",
                "+  \"ours\"\n",
                " ]\n",
            )
        );
    }

    #[test]
    fn an_entry_removed_from_an_array_is_the_mirror() {
        let before = "[\n  \"theirs\",\n  \"ours\"\n]";
        let after = "[\n  \"theirs\"\n]";
        assert_eq!(
            render("settings.json · hooks.Stop", Some(before), after),
            concat!(
                "--- settings.json · hooks.Stop\n",
                "+++ settings.json · hooks.Stop\n",
                " [\n",
                "-  \"theirs\",\n",
                "-  \"ours\"\n",
                "+  \"theirs\"\n",
                " ]\n",
            )
        );
    }

    #[test]
    fn an_unchanged_value_produces_only_context() {
        let value = "{\n  \"a\": 1\n}";
        assert_eq!(
            render("a.json · key", Some(value), value),
            "--- a.json · key\n+++ a.json · key\n {\n   \"a\": 1\n }\n"
        );
    }

    #[test]
    fn the_prefix_and_the_suffix_never_claim_the_same_line() {
        // "a" is both a common prefix and a common suffix of the two sides; printing it
        // twice would show the user a value that does not exist.
        let rendered = render("x", Some("a"), "a\nb\na");
        assert_eq!(rendered, "--- x\n+++ x\n a\n+b\n+a\n");
    }
}
