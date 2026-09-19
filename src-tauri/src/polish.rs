//! Turn a raw Deepgram transcript into the text we actually type.
//!
//! Rule-based on purpose: no model call sits between you and your words, so
//! dictation never waits on a second network round trip.

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Insert,
    /// "scratch that" — remove what the previous dictation typed.
    Scratch,
}

#[derive(Debug, Clone)]
pub struct Processed {
    pub text: String,
    pub action: Action,
    pub submit: bool,
    /// How many of the user's replacement rules fired. Surfaced only for the
    /// Insights page, which calls these the fixes made to your words.
    pub replacements: usize,
    /// Standalone filler words dropped.
    pub fillers: usize,
}

/// Words that are almost never intentional in dictation.
const FILLERS: &[&str] = &["um", "umm", "uhm", "uh", "uhh", "erm", "hmm", "mmm", "mhm", "hm"];

/// Spoken punctuation, longest phrase first so "question mark" wins over "mark".
///
/// Only unambiguous phrases live here. Ordinary nouns stay out on purpose —
/// "the period of this wave" must not turn into "the. of this wave", and
/// "comma", "colon" and "dash" are all everyday words. Anyone who wants symbol
/// dictation for those can add a replacement rule, which applies verbatim.
const SPOKEN: &[(&str, &str)] = &[
    ("new paragraph", "\n\n"),
    ("new line", "\n"),
    ("exclamation mark", "!"),
    ("exclamation point", "!"),
    ("question mark", "?"),
    ("open parenthesis", "("),
    ("close parenthesis", ")"),
    ("open paren", "("),
    ("close paren", ")"),
    ("full stop", "."),
    ("semicolon", ";"),
    ("hyphen", "-"),
];

/// The spoken word that marks the command after it as a command, for anyone who
/// wants to be explicit about it: "slash new line". The transcriber sometimes
/// writes that as "/" instead, and both spellings are accepted.
///
/// A bare command fires with or without it. Making the marker the *only* way in
/// would stop "new line" from working, which is a bigger change than it looks —
/// so the marked form is additive, and the cost of that is that "add a new line
/// of products" still becomes "add a\nof products".
const MARKER: &str = "slash";

const SCRATCH: &[&str] = &["scratch that", "delete that", "cancel that", "never mind that"];
const SUBMIT: &[&str] = &["press enter", "hit enter", "send it", "submit that"];

pub fn process(raw: &str, cfg: &Config) -> Processed {
    let mut text = raw.trim().to_string();

    // Checked before anything mutates the text, since it usually stands alone.
    if cfg.voice_commands {
        let bare = bare_sentence(&text).to_ascii_lowercase();
        if SCRATCH.contains(&bare.as_str()) {
            return Processed {
                text: String::new(),
                action: Action::Scratch,
                submit: false,
                replacements: 0,
                fillers: 0,
            };
        }
    }

    let mut fillers = 0;
    if cfg.remove_fillers {
        let (stripped, dropped) = strip_fillers(&text);
        text = stripped;
        fillers = dropped;
    }

    let mut submit = false;
    if cfg.voice_commands {
        if let Some(stripped) = strip_trailing_phrase(&text, SUBMIT) {
            text = stripped;
            submit = true;
        }
        text = apply_spoken_punctuation(&text);
    }

    // Counted around the user's own rules only. Spoken punctuation goes through
    // the same replacer, and turning "question mark" into "?" is not a fix.
    let mut replacements = 0;
    for rule in &cfg.replacements {
        if !rule.from.trim().is_empty() {
            let (replaced, hits) = replace_word_ci_counted(&text, rule.from.trim(), &rule.to);
            text = replaced;
            replacements += hits;
        }
    }

    text = tidy(&text);

    Processed { text, action: Action::Insert, submit, replacements, fillers }
}

/// Drop standalone filler words, taking a comma that hung off them with them.
fn strip_fillers(text: &str) -> (String, usize) {
    let mut kept: Vec<&str> = Vec::new();
    let mut dropped = 0;
    for token in text.split_whitespace() {
        let core = token.trim_matches(|c: char| !c.is_alphanumeric());
        if FILLERS.iter().any(|f| core.eq_ignore_ascii_case(f)) {
            dropped += 1;
            continue;
        }
        kept.push(token);
    }
    let mut out = kept.join(" ");
    // "so , I" / "so ,I" style leftovers after a filler was dropped mid-sentence.
    while out.contains(" ,") {
        out = out.replace(" ,", ",");
    }
    (out, dropped)
}

/// The text with the sentence-ending punctuation the transcriber added taken
/// off the end. A command usually arrives wearing it — "… press enter." — and a
/// command that fails to match is a command that gets typed out instead.
fn bare_sentence(text: &str) -> &str {
    text.trim().trim_end_matches(['.', ',', '!', '?', ';', ':']).trim_end()
}

fn strip_trailing_phrase(text: &str, phrases: &[&str]) -> Option<String> {
    let trimmed = bare_sentence(text);
    for p in phrases {
        // `trimmed` can be shorter than the phrase — that just rules this one out.
        let Some(cut) = trimmed.len().checked_sub(p.len()) else { continue };
        if cut == 0 || !trimmed.is_char_boundary(cut) {
            continue;
        }
        if !trimmed[cut..].eq_ignore_ascii_case(p) {
            continue;
        }
        let head = &trimmed[..cut];
        // The phrase must start at a word boundary, or "re-press enter" would
        // match too. Checked on `head` before trimming, since the separating
        // space is exactly the boundary we are testing for.
        if !head.is_empty() && !head.ends_with(|c: char| c.is_whitespace() || ",.;:!?".contains(c)) {
            continue;
        }
        let before = head.trim_end_matches([' ', ',']);
        if before.is_empty() {
            return Some(String::new());
        }
        return Some(format!("{before} "));
    }
    None
}

/// Replace spoken command phrases with their symbols.
///
/// Matched on word cores rather than on the literal phrase, because a spoken
/// command is not normal speech and the transcriber punctuates it as if it were.
/// "first line new line second line" comes back as any of
///
///     "First line, new line, second line."
///     "First line, new. Line, second line."
///
/// depending on where smart_format decided the sentence ended. A literal match
/// misses the second one entirely — the words are all there, just not adjacent —
/// which is why the command appeared to do nothing and got typed out instead.
/// The punctuation hung on the phrase goes with it when it is replaced, so the
/// comma that followed the command does not surface at the head of the new line.
fn apply_spoken_punctuation(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut at = 0;

    while at < words.len() {
        // "slash new line" is read as one, and the marker is stepped over so it
        // never reaches the typed text. A "/" the transcriber wrote instead is
        // not stepped over — it is punctuation on the word that follows, which
        // `core` strips when it matches "/new" as "new".
        let start = if core(words[at]) == MARKER { at + 1 } else { at };

        // SPOKEN is ordered longest phrase first, so "new paragraph" is tried
        // before "new line" and wins where both could start here.
        let hit = SPOKEN.iter().find_map(|(spoken, symbol)| {
            let phrase: Vec<&str> = spoken.split_whitespace().collect();
            matching(&words, start, &phrase).map(|len| (*symbol, start + len))
        });

        match hit {
            Some((symbol, end)) => {
                // A comma directly before the command is the pause the speaker
                // took to say it, not punctuation they dictated.
                if let Some(last) = out.last_mut() {
                    if last.ends_with(',') {
                        last.pop();
                        if last.is_empty() {
                            out.pop();
                        }
                    }
                }
                out.push(symbol.to_string());
                at = end;
            }
            None => {
                out.push(words[at].to_string());
                at += 1;
            }
        }
    }
    out.join(" ")
}

/// Whether `phrase` sits at `at`, ignoring any punctuation attached to those
/// words. Returns how many words it spans.
fn matching(words: &[&str], at: usize, phrase: &[&str]) -> Option<usize> {
    if at + phrase.len() > words.len() {
        return None;
    }
    let found = phrase
        .iter()
        .enumerate()
        .all(|(i, w)| core(words[at + i]).eq_ignore_ascii_case(w));
    found.then_some(phrase.len())
}

/// A word with whatever punctuation it is wearing stripped off: `"Line,"` →
/// `"line"`.
fn core(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase()
}

/// Replace `needle` when it appears as a whole word (or phrase), ignoring case,
/// reporting how many times it matched.
///
/// The count lives here rather than in a second matcher so the word-boundary
/// rules cannot drift apart between the two callers.
fn replace_word_ci_counted(hay: &str, needle: &str, to: &str) -> (String, usize) {
    let mut out = String::with_capacity(hay.len());
    let mut rest = hay;
    let mut hits = 0;
    // `rest` is consumed as matches are found, so the search re-runs on what is
    // left; the loop ends when nothing matches in the remainder.
    while let Some(i) = find_ci(rest, needle) {
        let before_ok = i == 0
            || !rest.as_bytes()[i - 1].is_ascii_alphanumeric()
            || needle.starts_with(|c: char| !c.is_alphanumeric());
        let end = i + needle.len();
        let after_ok = end >= rest.len()
            || !rest.as_bytes()[end].is_ascii_alphanumeric()
            || needle.ends_with(|c: char| !c.is_alphanumeric());
        if before_ok && after_ok {
            out.push_str(&rest[..i]);
            out.push_str(to);
            rest = &rest[end..];
            hits += 1;
        } else {
            // Not a boundary match — keep the char and carry on scanning.
            let step = rest[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            out.push_str(&rest[..i + step]);
            rest = &rest[i + step..];
        }
        if rest.is_empty() {
            break;
        }
    }
    out.push_str(rest);
    (out, hits)
}

/// Byte-wise ASCII-case-insensitive search.
///
/// ponytail: ASCII only. Non-ASCII triggers would need a char-indexed scan;
/// dictation triggers in practice are ASCII, so the byte scan is enough.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || n.len() > h.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// Collapse the whitespace damage the steps above can leave behind.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_space = false;
    for ch in text.chars() {
        if ch == '\n' {
            // Trim trailing spaces on the line we just closed.
            while out.ends_with(' ') {
                out.pop();
            }
            out.push('\n');
            prev_space = false;
            continue;
        }
        if ch == ' ' || ch == '\t' {
            prev_space = true;
            continue;
        }
        if prev_space && !out.is_empty() && !out.ends_with('\n') {
            out.push(' ');
        }
        prev_space = false;
        if ",.;:!?".contains(ch) {
            while out.ends_with(' ') {
                out.pop();
            }
        }
        out.push(ch);
    }
    while out.ends_with('\n') {
        out.pop();
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Rule};

    fn cfg() -> Config {
        Config::default()
    }

    #[test]
    fn drops_fillers_and_their_commas() {
        let p = process("So, um, I think uh we should ship it", &cfg());
        assert_eq!(p.text, "So, I think we should ship it");
    }

    #[test]
    fn filler_count_matches_what_was_dropped() {
        assert_eq!(process("So, um, I think uh we should ship it", &cfg()).fillers, 2);
        assert_eq!(process("no fillers in this one", &cfg()).fillers, 0);
        // "uh" inside a word is not a filler, so it must not be counted either.
        assert_eq!(process("the uhf antenna", &cfg()).fillers, 0);

        // Turning the feature off stops the count rather than reporting a
        // removal that did not happen.
        let mut c = cfg();
        c.remove_fillers = false;
        assert_eq!(process("um uh", &c).fillers, 0);
        assert_eq!(process("um uh", &c).text, "um uh");
    }

    #[test]
    fn replacement_count_counts_user_rules_only() {
        let mut c = cfg();
        c.replacements = vec![Rule { from: "or ra".into(), to: "Orra".into() }];

        let p = process("or ra and or ra again", &c);
        assert_eq!(p.text, "Orra and Orra again");
        assert_eq!(p.replacements, 2);

        // Spoken punctuation goes through the same replacer but is not a fix.
        let p = process("really question mark", &c);
        assert_eq!(p.text, "really?");
        assert_eq!(p.replacements, 0);

        // A rule that matches nothing contributes nothing.
        let mut c2 = cfg();
        c2.replacements = vec![Rule { from: "absent".into(), to: "x".into() }];
        assert_eq!(process("nothing here", &c2).replacements, 0);
    }

    #[test]
    fn replacement_count_respects_word_boundaries() {
        let mut c = cfg();
        c.replacements = vec![Rule { from: "cat".into(), to: "dog".into() }];

        // "concatenate" contains "cat" but not as a word, so it is not a fix.
        assert_eq!(process("concatenate the cat", &c).replacements, 1);
        assert_eq!(process("concatenate", &c).replacements, 0);
    }

    #[test]
    fn the_scratch_early_return_reports_no_fixes() {
        let mut c = cfg();
        c.replacements = vec![Rule { from: "cat".into(), to: "dog".into() }];
        let p = process("cat scratch that", &c);
        // "cat scratch that" is not the scratch phrase, so it is an insert...
        assert_eq!(p.action, Action::Insert);
        assert_eq!(p.replacements, 1);

        // ...whereas a bare "scratch that" short-circuits before any counting.
        let p = process("scratch that", &c);
        assert_eq!(p.action, Action::Scratch);
        assert_eq!(p.replacements, 0);
        assert_eq!(p.fillers, 0);
    }

    #[test]
    fn spoken_punctuation_does_not_eat_real_words() {
        // "period", "comma" and "colon" are ordinary nouns; a built-in command
        // must never rewrite them. Users who want symbols add a replacement rule.
        for sentence in [
            "the period of this wave is short",
            "add a comma before the closing tag",
            "the colon is the longest part",
        ] {
            assert_eq!(process(sentence, &cfg()).text, sentence);
        }
    }

    #[test]
    fn unambiguous_spoken_punctuation_still_converts() {
        assert_eq!(process("really question mark", &cfg()).text, "really?");
        assert_eq!(process("wait exclamation mark", &cfg()).text, "wait!");
    }

    #[test]
    fn submit_phrase_needs_a_word_boundary() {
        // A phrase buried inside a longer word must not be treated as a command.
        let p = process("repress enter", &cfg());
        assert_eq!(p.text, "repress enter");
        assert!(!p.submit);
    }

    #[test]
    fn scratch_that_is_detected_not_typed() {
        let p = process("Scratch that.", &cfg());
        assert_eq!(p.action, Action::Scratch);
        assert!(p.text.is_empty());
    }

    #[test]
    fn submit_phrase_is_stripped_and_flagged() {
        let p = process("ship it press enter", &cfg());
        assert_eq!(p.text, "ship it");
        assert!(p.submit);
    }

    #[test]
    fn new_line_becomes_a_real_newline() {
        let p = process("first line new line second line", &cfg());
        assert_eq!(p.text, "first line\nsecond line");
    }

    /// Both of these are what Deepgram returned, verbatim, for the same spoken
    /// sentence — the words arrive wrapped in the transcriber's punctuation, and
    /// the command has to fire anyway. The second one is the case that made the
    /// command silently do nothing and get typed out as words.
    #[test]
    fn a_command_survives_the_transcribers_punctuation() {
        let cfg = cfg();
        assert_eq!(process("First line, new line, second line.", &cfg).text, "First line\nsecond line.");
        assert_eq!(process("First line, new. Line, second line.", &cfg).text, "First line\nsecond line.");
    }

    #[test]
    fn replacing_a_command_takes_its_own_commas_with_it() {
        // Only the comma glued to the command goes: the one the user dictated
        // earlier in the sentence stays where they put it.
        let p = process("I said hello, new line and then this", &cfg());
        assert_eq!(p.text, "I said hello\nand then this");
    }

    /// The marked form, for anyone who wants to be unambiguous about it. Both
    /// spellings the transcriber might produce, and the marker itself is eaten.
    #[test]
    fn a_command_can_be_marked_with_slash() {
        let cfg = cfg();
        assert_eq!(process("first line slash new line second line", &cfg).text, "first line\nsecond line");
        assert_eq!(process("one slash new paragraph two", &cfg).text, "one\n\ntwo");
        assert_eq!(process("is it ready slash question mark", &cfg).text, "is it ready?");

        // When it hears the word as the symbol instead.
        assert_eq!(process("first line /new line second line", &cfg).text, "first line\nsecond line");

        // A "slash" with no command after it is just a word.
        assert_eq!(process("press slash to continue", &cfg).text, "press slash to continue");
    }

    #[test]
    fn a_command_at_the_end_still_fires_through_a_full_stop() {
        let p = process("ship it, press enter.", &cfg());
        assert_eq!(p.text, "ship it");
        assert!(p.submit);

        // "scratch that" is matched the same tolerant way.
        assert_eq!(process("Scratch that.", &cfg()).action, Action::Scratch);
        assert_eq!(process("scratch that!", &cfg()).action, Action::Scratch);
        assert_eq!(process("scratch that,", &cfg()).action, Action::Scratch);
    }

    #[test]
    fn replacements_respect_word_boundaries() {
        let mut c = cfg();
        c.replacements = vec![Rule { from: "or ra".into(), to: "Orra".into() }];
        assert_eq!(process("welcome to or ra", &c).text, "welcome to Orra");

        let mut c2 = cfg();
        c2.replacements = vec![Rule { from: "cat".into(), to: "dog".into() }];
        // "category" must not become "dogegory".
        assert_eq!(process("category theory", &c2).text, "category theory");
    }

    #[test]
    fn trailing_space_is_not_added_here() {
        let p = process("  hello world  ", &cfg());
        assert_eq!(p.text, "hello world");
    }

    #[test]
    fn voice_commands_can_be_disabled() {
        let mut c = cfg();
        c.voice_commands = false;
        c.remove_fillers = false;
        assert_eq!(process("new line", &c).text, "new line");
    }
}
