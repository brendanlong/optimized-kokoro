//! Abbreviations read in full where the words around them show what they stand for, and left as
//! written where they don't (`docs/divergences.md`, "Text: abbreviations"):
//!
//! - "St." and "Dr." before a name are "Saint" and "Doctor"; after a capitalised word or a
//!   number ("Main St.", "Mulholland Dr.", "42nd St.") "Street" and "Drive".
//! - "No." before a number is "number" ("No. 5"), and a month's abbreviation before a day or a
//!   year is the month ("Dec. 2025", "Oct 20").
//! - In citations: "p." and "pp." before a number are "page" and "pages", "Vol." "volume", "ca."
//!   "circa", "esp." "especially", and "v." between two capitalised words "versus" ("Roe v.
//!   Wade").
//! - "tl;dr" and "tldr" are the letters "TLDR".

/// Words that start sentences more often than they are names: after "St." or "Dr.", they say the
/// abbreviation ended a sentence ("on Main St. The next day").
const SENTENCE_STARTERS: &[&str] = &[
    "The", "A", "An", "In", "On", "At", "He", "She", "It", "They", "We", "I", "You", "This", "That", "These", "Those",
    "There", "His", "Her", "Its", "Our", "Their", "My", "Your", "But", "And", "Or", "So", "If", "When", "While",
    "After", "Before", "Since", "As", "For", "From", "To", "With", "By", "Then", "Now", "Was", "Is", "Are", "Were",
    "What", "Who", "Where", "Why", "How", "Some", "Many", "Most", "All", "One", "Two", "Its",
];

const MONTHS: &[(&str, &str)] = &[
    ("Jan", "January"),
    ("Feb", "February"),
    ("Mar", "March"),
    ("Apr", "April"),
    ("Jun", "June"),
    ("Jul", "July"),
    ("Aug", "August"),
    ("Sep", "September"),
    ("Sept", "September"),
    ("Oct", "October"),
    ("Nov", "November"),
    ("Dec", "December"),
];

struct Text<'a> {
    t: &'a [char],
}

impl Text<'_> {
    fn get(&self, i: usize) -> Option<char> {
        self.t.get(i).copied()
    }

    fn letters_end(&self, i: usize) -> usize {
        let mut j = i;
        while self.get(j).is_some_and(|c| c.is_alphabetic()) {
            j += 1;
        }
        j
    }

    fn digits_end(&self, i: usize) -> usize {
        let mut j = i;
        while self.get(j).is_some_and(|c| c.is_ascii_digit()) {
            j += 1;
        }
        j
    }

    fn free_at(&self, i: usize) -> bool {
        !self.get(i).is_some_and(char::is_alphanumeric)
    }

    /// The token (characters up to a space) after the space at `i`, if there is one.
    fn next_token(&self, i: usize) -> Option<&[char]> {
        if self.get(i) != Some(' ') {
            return None;
        }
        let end = (i + 1..self.t.len()).find(|&k| self.t[k] == ' ').unwrap_or(self.t.len());
        (end > i + 1).then(|| &self.t[i + 1..end])
    }

    /// The token before the space before `i`, if there is one.
    fn previous_token(&self, i: usize) -> Option<&[char]> {
        if i < 2 || self.t[i - 1] != ' ' {
            return None;
        }
        let start = self.t[..i - 1].iter().rposition(|&c| c == ' ').map_or(0, |k| k + 1);
        (start < i - 1).then(|| &self.t[start..i - 1])
    }

    /// Whether a number of the kind a citation, list or date has comes after the space at `i`.
    fn number_after(&self, i: usize) -> bool {
        self.get(i) == Some(' ') && self.get(i + 1).is_some_and(|c| c.is_ascii_digit())
    }

    /// A day (1 to 31, maybe with an ordinal suffix) or a four-digit year after the space at `i`.
    fn day_or_year_after(&self, i: usize) -> bool {
        if !self.number_after(i) {
            return false;
        }
        let end = self.digits_end(i + 1);
        let value: usize = self.t[i + 1..end].iter().collect::<String>().parse().unwrap_or(0);
        let suffix = ["st", "nd", "rd", "th"]
            .iter()
            .any(|s| s.chars().enumerate().all(|(k, c)| self.get(end + k) == Some(c)) && self.free_at(end + 2));
        let ends = self.free_at(end) && !(matches!(self.get(end), Some('.' | ':' | ',')) && self.digit(end + 1));
        match end - i - 1 {
            1 | 2 => (1..=31).contains(&value) && (ends || suffix),
            4 => ends,
            _ => false,
        }
    }

    fn digit(&self, i: usize) -> bool {
        self.get(i).is_some_and(|c| c.is_ascii_digit())
    }
}

/// A capitalised word ("Louis", "Paul's", "Jr."), or an initial ("J.").
fn is_name(token: &[char]) -> bool {
    let word: String = token.iter().take_while(|c| c.is_alphabetic() || matches!(c, '\'' | '-')).collect();
    let initial = token.len() == 2 && token[0].is_uppercase() && token[1] == '.';
    initial
        || token.first().is_some_and(|c| c.is_uppercase())
            && word.chars().count() >= 2
            && !SENTENCE_STARTERS.contains(&word.as_str())
}

/// The word before a street's "St." or "Dr.": a capitalised word ("Main", "King Jr."), or a
/// number ("42nd", "5th"). A longer word ending in a period ended a sentence ("in Paris.").
fn is_street_name(token: &[char]) -> bool {
    let (body, period) = match token.split_last() {
        Some(('.', body)) => (body, true),
        _ => (token, false),
    };
    let capitalised = body.first().is_some_and(|c| c.is_uppercase())
        && body.iter().all(|c| c.is_alphabetic() || matches!(c, '\'' | '-'))
        && body.len() >= 2;
    let number = body.first().is_some_and(char::is_ascii_digit)
        && (body.iter().all(char::is_ascii_digit) || {
            let s: String = body.iter().collect::<String>().to_ascii_lowercase();
            ["st", "nd", "rd", "th"].iter().any(|x| s.ends_with(x))
                && s[..s.len() - 2].bytes().all(|b| b.is_ascii_digit())
        });
    (capitalised && (!period || body.len() <= 3)) || (number && !period)
}

/// The abbreviation pass.
pub fn abbreviations(text: &str) -> String {
    let t: Vec<char> = text.chars().collect();
    let x = Text { t: &t };
    let mut out = String::with_capacity(text.len() + 16);
    let mut i = 0;
    while i < t.len() {
        let c = t[i];
        let word_start = c.is_alphabetic() && (i == 0 || !(t[i - 1].is_alphanumeric() || t[i - 1] == '.'));
        if word_start {
            let end = x.letters_end(i);
            let word: String = t[i..end].iter().collect();
            if let Some((reading, next)) = reading(&x, i, end, &word) {
                out.push_str(&reading);
                i = next;
                continue;
            }
            out.push_str(&word);
            i = end;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// What the word `t[start..end]` reads as, and where the text after it resumes.
fn reading(x: &Text, start: usize, end: usize, word: &str) -> Option<(String, usize)> {
    let dot = x.get(end) == Some('.');
    let after = end + usize::from(dot);
    let lower = word.to_ascii_lowercase();
    // "tl;dr", "TL;DR", "tldr": the letters.
    if lower == "tl" && x.get(end) == Some(';') {
        let dr = x.letters_end(end + 1);
        if dr == end + 3 && x.t[end + 1..dr].iter().collect::<String>().eq_ignore_ascii_case("dr") {
            return Some(("TLDR".to_owned(), dr));
        }
    }
    if lower == "tldr" && word != "TLDR" {
        return Some(("TLDR".to_owned(), end));
    }
    match word {
        "St" | "Dr" | "ST" | "DR" => {
            let (title, street) =
                if word.eq_ignore_ascii_case("st") { ("Saint", "Street") } else { ("Doctor", "Drive") };
            let next = x.next_token(after);
            let caps = word.chars().all(|c| c.is_uppercase());
            let street_before = !caps && x.previous_token(start).is_some_and(is_street_name);
            // Before a capital, the title, as the reference read "Dr." ("Dr. Who"); but after a
            // street's name, a word that starts sentences starts the next one ("on Main St. The").
            // Without the period, only before a name ("St Paul", "Dr Smith").
            let starter = next.is_some_and(|n| {
                let word: String = n.iter().take_while(|c| c.is_alphabetic()).collect();
                SENTENCE_STARTERS.contains(&word.as_str())
            });
            let capital = next.is_some_and(|n| n.first().is_some_and(|c| c.is_uppercase()));
            if dot && capital && !(street_before && starter) || !dot && !caps && next.is_some_and(is_name) {
                return Some((title.to_owned(), after));
            }
            // "DR. SMITH", whose name the caps pass has lowercased.
            if word == "DR" && dot && next.is_some_and(|n| n.iter().all(|c| c.is_lowercase())) {
                return Some((title.to_owned(), after));
            }
            // A street's name ends there: the text ends, or a space or closing punctuation follows
            // ("Neville St.—" is a name cut short).
            let ends = x.get(after).is_none_or(|c| c == ' ' || ",;:)!?\"'”’".contains(c));
            if !street_before || !ends {
                return None;
            }
            // The period also ends a sentence before a capital or at the end of the text.
            let ends_sentence =
                dot && (after == x.t.len() || next.is_some_and(|n| n.first().is_some_and(|c| c.is_uppercase())));
            return Some((format!("{street}{}", if ends_sentence { "." } else { "" }), after));
        }
        "No" | "no" | "Nos" | "nos" if dot => {
            let glued = x.digit(after);
            if glued || x.number_after(after) {
                let number = if word.starts_with('N') { "Number" } else { "number" };
                let plural = if word.ends_with('s') { "s" } else { "" };
                return Some((format!("{number}{plural}{}", if glued { " " } else { "" }), after));
            }
        }
        "p" | "pp" | "Vol" | "vol" | "ca" if dot && x.number_after(after) => {
            let full = match word {
                "p" => "page",
                "pp" => "pages",
                "Vol" => "Volume",
                "vol" => "volume",
                _ => "circa",
            };
            return Some((full.to_owned(), after));
        }
        "esp" if dot && x.free_at(after) => return Some(("especially".to_owned(), after)),
        "v" if dot => {
            let names = x.previous_token(start).is_some_and(|p| {
                p.first().is_some_and(|c| c.is_uppercase()) && p.iter().all(|c| c.is_alphabetic() || *c == '\'')
            }) && x.next_token(after).is_some_and(|n| n.first().is_some_and(|c| c.is_uppercase()));
            if names {
                return Some(("versus".to_owned(), after));
            }
        }
        _ => {}
    }
    if let Some(&(_, month)) = MONTHS.iter().find(|(abbreviation, _)| *abbreviation == word)
        && x.day_or_year_after(after)
    {
        return Some((month.to_owned(), after));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saint_and_doctor_before_names_street_and_drive_after_them() {
        for (text, expected) in [
            ("St. Louis and St Paul's", "Saint Louis and Saint Paul's"),
            ("Dr. Smith and Dr Jones and DR. WHO", "Doctor Smith and Doctor Jones and Doctor WHO"),
            ("Mount St. Helens", "Mount Saint Helens"),
            ("Exile on Main St. was ranked", "Exile on Main Street was ranked"),
            ("at Commerce St. and then", "at Commerce Street and then"),
            (
                "Martin Luther King Jr. Dr. (formerly Hunter St.) and",
                "Martin Luther King Jr. Drive (formerly Hunter Street) and",
            ),
            ("David Lynch's Mulholland Dr.", "David Lynch's Mulholland Drive."),
            ("on 42nd St. near", "on 42nd Street near"),
            ("on Main St. The next day", "on Main Street. The next day"),
            ("He lives on Elm St", "He lives on Elm Street"),
            // No name after it and no street before it: as written.
            ("the dr. said and the St. was", "the dr. said and the St. was"),
            ("Ask the Dr.", "Ask the Dr."),
            ("He ended in Paris. St. was", "He ended in Paris. St. was"),
            ("MAIN ST. here", "MAIN ST. here"),
        ] {
            assert_eq!(abbreviations(text), expected, "{text}");
        }
    }

    #[test]
    fn numbers_months_and_citations() {
        for (text, expected) in [
            ("No. 5 and no. 12 and Nos. 3 and 4, No.7", "Number 5 and number 12 and Numbers 3 and 4, Number 7"),
            ("No. I won't. No. We said no.", "No. I won't. No. We said no."),
            (
                "in Dec. 2025, on Jan. 5, Oct 20, 2022 and 26 Oct 2023",
                "in December 2025, on January 5, October 20, 2022 and 26 October 2023",
            ),
            ("Sept. 3rd and Aug. 4)", "September 3rd and August 4)"),
            ("Jan said 5:30, Mar 3.5 and Dec. 45", "Jan said 5:30, Mar 3.5 and Dec. 45"),
            (
                "see pp. 230-232 and p. 183, Vol. 35, vol. 2, ca. 1700",
                "see pages 230-232 and page 183, Volume 35, volume 2, circa 1700",
            ),
            ("Amadis esp. o.c. and esp.", "Amadis especially o.c. and especially"),
            ("Kunath v. Smith and Roe v. Wade", "Kunath versus Smith and Roe versus Wade"),
            ("a v. b and vitamin v. C and p. q", "a v. b and vitamin v. C and p. q"),
        ] {
            assert_eq!(abbreviations(text), expected, "{text}");
        }
    }

    #[test]
    fn tldr_is_letters() {
        assert_eq!(
            abbreviations("tl;dr: fine. TL;DR it works, tldr and Tl;Dr"),
            "TLDR: fine. TLDR it works, TLDR and TLDR"
        );
        assert_eq!(abbreviations("tl;drs, xtldr and TLDR"), "tl;drs, xtldr and TLDR");
    }
}
