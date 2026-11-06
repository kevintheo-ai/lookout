//! Regular expressions for the word lists. Watcher.swift's patterns are copied verbatim and run here with the
//! semantics of `NSRegularExpression` (ICU): case-insensitive option, `\b` that skips combining marks, `\s` as
//! Unicode White_Space, lookahead, lazy and bounded quantifiers, the inline flag `(?-i:…)`, `^` with optional
//! multi-line anchors. Only what those patterns use; anything else is a parse error.
//!
//! A backtracking matcher over code points like ICU. A quantifier over a node that always takes exactly one character
//! counts its run in a loop, so the stack depth depends on the pattern, not on the length of the text.
//!
//! ponytail: character properties come from Rust's `char` methods plus hand-picked ranges (combining marks, format
//! characters), not ICU's tables. Case-insensitive matching folds single characters; ICU folds fully ("strasse"
//! finds "Straße", measured), which the patterns do not need: they spell out "(ß|ss)". Checked against
//! NSRegularExpression on 41,533 real answer texts, all 27 patterns, identical matches (2026-11-06). Upgrade path: a
//! regex crate with lookaround, if a dependency becomes acceptable here.

use std::ops::Range;

#[derive(Debug)]
pub struct Regex {
    node: Node,
}

#[derive(Debug)]
enum Node {
    /// Literal; `true` = case-insensitive.
    Char(char, bool),
    Class(Class),
    /// `^`; `true` = also after a line terminator (`.anchorsMatchLines`).
    Start(bool),
    /// `$`: end of text, or before a line terminator that ends it.
    End,
    /// `\b`
    Boundary,
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat {
        node: Box<Node>,
        min: usize,
        max: usize,
        lazy: bool,
    },
    /// `(?=…)`, or `(?!…)` with `negate`. Atomic, as in ICU.
    Ahead {
        node: Box<Node>,
        negate: bool,
    },
}

#[derive(Debug)]
struct Class {
    items: Vec<Item>,
    negated: bool,
    caseless: bool,
}

#[derive(Debug, Clone, Copy)]
enum Item {
    Range(char, char),
    /// `\s`, or `\S` with `true`.
    Space(bool),
    /// `\w` / `\W`.
    Word(bool),
    /// `\d` / `\D`.
    Digit(bool),
    /// `\p{L}`
    Letter,
    /// `\p{Lu}`
    Upper,
    /// `\p{Lt}`, `\p{Lm}`, `\p{Lo}`: letters without case.
    Uncased,
    /// `\p{N}`
    Number,
}

impl Regex {
    /// `NSRegularExpression(pattern:options:)` with `.caseInsensitive` and `.anchorsMatchLines` as flags.
    pub fn new(pattern: &str, caseless: bool, lines: bool) -> Result<Regex, String> {
        let mut p = Parser {
            chars: pattern.chars().collect(),
            pos: 0,
            caseless,
            lines,
        };
        let node = p.alt()?;
        if p.pos < p.chars.len() {
            return Err(format!("unbalanced ')' at {} in {pattern}", p.pos));
        }
        Ok(Regex { node })
    }

    /// Leftmost match starting at or after `from` (character indices).
    pub fn find_at(&self, text: &[char], from: usize) -> Option<Range<usize>> {
        (from..=text.len()).find_map(|start| {
            let mut end = None;
            self.node
                .at(text, start, &mut |e| {
                    end = Some(e);
                    true
                })
                .then(|| start..end.unwrap_or(start))
        })
    }

    /// All non-overlapping matches from left to right (`matches(in:)`); after an empty match the next search starts
    /// one character later.
    pub fn find_all(&self, text: &[char]) -> Vec<Range<usize>> {
        let mut out = vec![];
        let mut at = 0;
        while at <= text.len() {
            let Some(m) = self.find_at(text, at) else {
                break;
            };
            at = if m.is_empty() { m.end + 1 } else { m.end };
            out.push(m);
        }
        out
    }

    pub fn is_match(&self, text: &str) -> bool {
        let t: Vec<char> = text.chars().collect();
        self.find_at(&t, 0).is_some()
    }
}

/// `NSRegularExpression.escapedPattern(for:)` for a name: ASCII punctuation escaped, so it matches literally.
pub fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    caseless: bool,
    lines: bool,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn next(&mut self) -> Result<char, String> {
        let c = self.peek().ok_or("pattern ends early")?;
        self.pos += 1;
        Ok(c)
    }

    fn eat(&mut self, c: char) -> bool {
        let hit = self.peek() == Some(c);
        if hit {
            self.pos += 1;
        }
        hit
    }

    fn alt(&mut self) -> Result<Node, String> {
        let mut branches = vec![self.concat()?];
        while self.eat('|') {
            branches.push(self.concat()?);
        }
        Ok(if branches.len() == 1 {
            branches.remove(0)
        } else {
            Node::Alt(branches)
        })
    }

    fn concat(&mut self) -> Result<Node, String> {
        let mut items = vec![];
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            items.push(self.quantified(atom)?);
        }
        Ok(Node::Concat(items))
    }

    fn atom(&mut self) -> Result<Node, String> {
        Ok(match self.next()? {
            '(' => self.group()?,
            '[' => Node::Class(self.class()?),
            '^' => Node::Start(self.lines),
            '$' if !self.lines => Node::End,
            '\\' => match self.escape()? {
                Escaped::Char(c) => Node::Char(c, self.caseless),
                Escaped::Item(item) => Node::Class(Class {
                    items: vec![item],
                    negated: false,
                    caseless: self.caseless,
                }),
                Escaped::Boundary => Node::Boundary,
            },
            c @ ('*' | '+' | '?' | '{' | '$') => {
                return Err(format!("unsupported '{c}' at {}", self.pos - 1));
            }
            c => Node::Char(c, self.caseless),
        })
    }

    fn group(&mut self) -> Result<Node, String> {
        let saved = self.caseless;
        let node = if self.eat('?') {
            match self.next()? {
                ':' => self.alt()?,
                c @ ('=' | '!') => Node::Ahead {
                    node: Box::new(self.alt()?),
                    negate: c == '!',
                },
                '-' if self.eat('i') && self.eat(':') => {
                    self.caseless = false;
                    self.alt()?
                }
                'i' if self.eat(':') => {
                    self.caseless = true;
                    self.alt()?
                }
                c => return Err(format!("unsupported group '(?{c}' at {}", self.pos)),
            }
        } else {
            // Capturing group: nothing reads its captures here.
            self.alt()?
        };
        self.caseless = saved;
        if !self.eat(')') {
            return Err(format!("missing ')' at {}", self.pos));
        }
        Ok(node)
    }

    fn quantified(&mut self, atom: Node) -> Result<Node, String> {
        let (min, max) = match self.peek() {
            Some('*') => (0, usize::MAX),
            Some('+') => (1, usize::MAX),
            Some('?') => (0, 1),
            Some('{') => {
                self.pos += 1;
                let min = self.number()?;
                let max = if self.eat(',') {
                    if self.peek() == Some('}') {
                        usize::MAX
                    } else {
                        self.number()?
                    }
                } else {
                    min
                };
                if self.peek() != Some('}') || max < min {
                    return Err(format!("bad bounds at {}", self.pos));
                }
                (min, max)
            }
            _ => return Ok(atom),
        };
        self.pos += 1;
        let lazy = self.eat('?');
        if self.peek() == Some('+') {
            return Err("possessive quantifiers are not supported".into());
        }
        Ok(Node::Repeat {
            node: Box::new(atom),
            min,
            max,
            lazy,
        })
    }

    fn number(&mut self) -> Result<usize, String> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        self.chars[start..self.pos]
            .iter()
            .collect::<String>()
            .parse()
            .map_err(|_| format!("bad number at {start}"))
    }

    fn escape(&mut self) -> Result<Escaped, String> {
        Ok(match self.next()? {
            'b' => Escaped::Boundary,
            's' => Escaped::Item(Item::Space(false)),
            'S' => Escaped::Item(Item::Space(true)),
            'w' => Escaped::Item(Item::Word(false)),
            'W' => Escaped::Item(Item::Word(true)),
            'd' => Escaped::Item(Item::Digit(false)),
            'D' => Escaped::Item(Item::Digit(true)),
            'n' => Escaped::Char('\n'),
            't' => Escaped::Char('\t'),
            'r' => Escaped::Char('\r'),
            'f' => Escaped::Char('\u{C}'),
            'p' => {
                if !self.eat('{') {
                    return Err(format!("expected '{{' at {}", self.pos));
                }
                let start = self.pos;
                while self.peek().is_some_and(|c| c != '}') {
                    self.pos += 1;
                }
                let name: String = self.chars[start..self.pos].iter().collect();
                self.pos += 1;
                Escaped::Item(match name.as_str() {
                    "L" => Item::Letter,
                    "Lu" => Item::Upper,
                    "Lt" | "Lm" | "Lo" => Item::Uncased,
                    "N" => Item::Number,
                    _ => return Err(format!("unsupported property {name}")),
                })
            }
            c if c.is_alphanumeric() => return Err(format!("unsupported escape '\\{c}'")),
            c => Escaped::Char(c),
        })
    }

    fn class(&mut self) -> Result<Class, String> {
        let negated = self.eat('^');
        let mut items = vec![];
        loop {
            let lo = match self.next()? {
                ']' if !items.is_empty() => break,
                '[' => return Err("nested sets are not supported".into()),
                '\\' => match self.escape()? {
                    Escaped::Char(c) => c,
                    Escaped::Item(item) => {
                        items.push(item);
                        continue;
                    }
                    Escaped::Boundary => return Err("'\\b' in a set".into()),
                },
                c => c,
            };
            // A '-' between two characters makes a range; at the end of the set it is literal.
            if self.peek() == Some('-') && self.chars.get(self.pos + 1).is_some_and(|&c| c != ']') {
                self.pos += 1;
                let hi = match self.next()? {
                    '\\' => match self.escape()? {
                        Escaped::Char(c) => c,
                        _ => return Err("range to a class".into()),
                    },
                    c => c,
                };
                items.push(Item::Range(lo, hi));
            } else {
                items.push(Item::Range(lo, lo));
            }
        }
        Ok(Class {
            items,
            negated,
            caseless: self.caseless,
        })
    }
}

enum Escaped {
    Char(char),
    Item(Item),
    Boundary,
}

impl Node {
    /// Does the node match at `i`, with a continuation `k` for what follows (`k` gets the end and decides)?
    fn at(&self, t: &[char], i: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        match self {
            Node::Char(c, caseless) => {
                t.get(i).is_some_and(|&x| same_char(x, *c, *caseless)) && k(i + 1)
            }
            Node::Class(class) => t.get(i).is_some_and(|&x| class.has(x)) && k(i + 1),
            Node::Start(lines) => {
                (i == 0 || (*lines && i < t.len() && is_line_end(t[i - 1]))) && k(i)
            }
            Node::End => at_end(t, i) && k(i),
            Node::Boundary => is_boundary(t, i) && k(i),
            Node::Concat(items) => concat(items, t, i, k),
            Node::Alt(branches) => branches.iter().any(|b| b.at(t, i, k)),
            Node::Ahead { node, negate } => node.at(t, i, &mut |_| true) != *negate && k(i),
            Node::Repeat {
                node,
                min,
                max,
                lazy,
            } => {
                if node.single() {
                    // Exactly one character per round: count the run, then try the lengths in order.
                    let limit = (*max).min(t.len() - i);
                    let mut n = 0;
                    while n < limit && node.at(t, i + n, &mut |_| true) {
                        n += 1;
                    }
                    if n < *min {
                        return false;
                    }
                    if *lazy {
                        (*min..=n).any(|c| k(i + c))
                    } else {
                        (*min..=n).rev().any(|c| k(i + c))
                    }
                } else {
                    repeat(node, *min, *max, *lazy, t, i, 0, k)
                }
            }
        }
    }

    /// Takes exactly one character whenever it matches (zero-width assertions around it allowed).
    fn single(&self) -> bool {
        match self {
            Node::Char(..) | Node::Class(_) => true,
            Node::Alt(branches) => branches.iter().all(Node::single),
            Node::Concat(items) => {
                items.iter().filter(|n| !n.zero_width()).count() == 1
                    && items.iter().all(|n| n.zero_width() || n.single())
            }
            _ => false,
        }
    }

    fn zero_width(&self) -> bool {
        matches!(
            self,
            Node::Start(_) | Node::End | Node::Boundary | Node::Ahead { .. }
        )
    }
}

fn concat(items: &[Node], t: &[char], i: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
    match items.split_first() {
        None => k(i),
        Some((first, rest)) => first.at(t, i, &mut |j| concat(rest, t, j, k)),
    }
}

#[allow(clippy::too_many_arguments)]
fn repeat(
    node: &Node,
    min: usize,
    max: usize,
    lazy: bool,
    t: &[char],
    i: usize,
    count: usize,
    k: &mut dyn FnMut(usize) -> bool,
) -> bool {
    if count < min {
        return node.at(t, i, &mut |j| {
            repeat(node, min, max, lazy, t, j, count + 1, k)
        });
    }
    if count >= max {
        return k(i);
    }
    // A round that takes nothing ends the loop (no endless empty rounds).
    let more = |k: &mut dyn FnMut(usize) -> bool| {
        node.at(t, i, &mut |j| {
            j != i && repeat(node, min, max, lazy, t, j, count + 1, k)
        })
    };
    if lazy {
        k(i) || more(k)
    } else {
        more(k) || k(i)
    }
}

impl Class {
    fn has(&self, c: char) -> bool {
        let any = |c: char| self.items.iter().any(|item| item.has(c));
        let hit = any(c) || (self.caseless && (any(lower(c)) || any(upper(c)) || any(fold(c))));
        hit != self.negated
    }
}

impl Item {
    fn has(self, c: char) -> bool {
        match self {
            Item::Range(lo, hi) => (lo..=hi).contains(&c),
            Item::Space(not) => is_space(c) != not,
            Item::Word(not) => is_word(c) != not,
            Item::Digit(not) => is_decimal(c) != not,
            Item::Letter => c.is_alphabetic(),
            Item::Upper => c.is_uppercase(),
            Item::Uncased => is_uncased_letter(c),
            Item::Number => c.is_numeric(),
        }
    }
}

/// The character when a case mapping gives exactly one.
fn single(mut it: impl Iterator<Item = char>) -> Option<char> {
    let first = it.next()?;
    it.next().is_none().then_some(first)
}

fn lower(c: char) -> char {
    single(c.to_lowercase()).unwrap_or(c)
}

fn upper(c: char) -> char {
    single(c.to_uppercase()).unwrap_or(c)
}

/// Simple case folding, close enough: "ſ" and "s", "ς" and "σ" fold together.
fn fold(c: char) -> char {
    lower(upper(c))
}

fn same_char(x: char, c: char, caseless: bool) -> bool {
    x == c || (caseless && fold(x) == fold(c))
}

/// ICU `\s`: Unicode White_Space (measured: vertical tab and U+0085 count, U+200B does not).
fn is_space(c: char) -> bool {
    c.is_whitespace()
}

/// ICU `\w`: `[\p{Alphabetic}\p{M}\p{Nd}\p{Pc}\u200c\u200d]` (measured: "²" and "½" are not word characters).
fn is_word(c: char) -> bool {
    c.is_alphabetic()
        || is_decimal(c)
        || is_mark(c)
        || matches!(
            c,
            '_' | '\u{200C}'
                | '\u{200D}'
                | '\u{203F}'
                | '\u{2040}'
                | '\u{2054}'
                | '\u{FE33}'
                | '\u{FE34}'
                | '\u{FE4D}'..='\u{FE4F}' | '\u{FF3F}'
        )
}

/// `\p{Nd}`: numbers that are neither letters (Roman numerals) nor other numbers (superscripts, fractions, circled).
fn is_decimal(c: char) -> bool {
    c.is_ascii_digit()
        || (!c.is_ascii()
            && c.is_numeric()
            && !c.is_alphabetic()
            && !matches!(c as u32, 0xB2 | 0xB3 | 0xB9 | 0xBC..=0xBE | 0x2070..=0x209F | 0x2150..=0x218F
                | 0x2460..=0x24FF | 0x2776..=0x2793 | 0x3248..=0x325F | 0x3280..=0x32BF | 0x1F100..=0x1F10C))
}

/// `\p{Lt}`, `\p{Lm}`, `\p{Lo}`: letters without case, plus the Latin modifier letters (Lm, but lowercase for Rust).
fn is_uncased_letter(c: char) -> bool {
    (c.is_alphabetic() && !c.is_uppercase() && !c.is_lowercase())
        || matches!(c as u32, 0x02B0..=0x02C1 | 0x02C6..=0x02D1 | 0x02E0..=0x02E4 | 0x02EC | 0x02EE
            | 0x1D2C..=0x1D6A | 0x1D78 | 0x1D9B..=0x1DBF | 0x2071 | 0x207F | 0x2090..=0x209C)
}

/// Combining marks and variation selectors (the ranges Latin text uses; other scripts' marks are mostly alphabetic).
fn is_mark(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF
        | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F | 0xE0100..=0xE01EF)
}

/// What ICU's `\b` looks through: grapheme extenders and format characters.
fn is_extend_or_format(c: char) -> bool {
    is_mark(c)
        || matches!(c as u32, 0xAD | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064
            | 0x2066..=0x206F | 0xFEFF | 0xE0001 | 0xE0020..=0xE007F)
}

/// ICU's line terminators.
fn is_line_end(c: char) -> bool {
    matches!(
        c,
        '\n' | '\u{B}' | '\u{C}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// ICU `\b` without UREGEX_UWORD: a word character on exactly one side, combining marks belong to the character
/// before them.
fn is_boundary(t: &[char], i: usize) -> bool {
    let here = match t.get(i) {
        Some(&c) if is_extend_or_format(c) => return false,
        Some(&c) => is_word(c),
        None => false,
    };
    let before = t[..i]
        .iter()
        .rev()
        .find(|&&c| !is_extend_or_format(c))
        .is_some_and(|&c| is_word(c));
    here != before
}

/// ICU `$` without multi-line mode: end of text, or before a final line terminator (CR LF counts as one).
fn at_end(t: &[char], i: usize) -> bool {
    let rest = &t[i..];
    match rest {
        [] => true,
        ['\r', '\n'] => true,
        [c] => is_line_end(*c) && !(*c == '\n' && i > 0 && t[i - 1] == '\r'),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, caseless: bool, text: &str) -> Vec<String> {
        let r = Regex::new(pattern, caseless, false).unwrap();
        let t: Vec<char> = text.chars().collect();
        r.find_all(&t)
            .into_iter()
            .map(|m| t[m].iter().collect())
            .collect()
    }

    /// Expected values measured with NSRegularExpression (Swift 6.4.2, macOS 27), 2026-11-06.
    #[test]
    fn behaves_like_icu() {
        assert_eq!(
            matches(r"\bdone\b", true, "Done, undone; DONE."),
            ["Done", "DONE"]
        );
        assert_eq!(matches(r"a{0,2}?b", false, "aaab"), ["aab"]);
        assert_eq!(matches(r"x(?!y)", false, "xy xz x"), ["x", "x"]);
        assert_eq!(
            matches(r"(?:[^.!?]|[.!?](?=\S))+", false, "a.b c. d! e"),
            ["a.b c", " d", " e"]
        );
        assert_eq!(
            matches(r"au(ß|ss)er", true, "AUSSER außer Außer"),
            ["AUSSER", "außer", "Außer"]
        );
        assert_eq!(
            matches(r"\b(du|(?-i:Will))\b", true, "will Will DU"),
            ["Will", "DU"]
        );
        assert_eq!(matches(r"[a-c]", true, "xBz"), ["B"]);
        assert_eq!(matches(r"s", true, "ſ S"), ["ſ", "S"]);
        assert_eq!(matches(r"\bI'll", true, "I'll i'll"), ["I'll", "i'll"]);
        let sock = r"/cc-socks/(\d+)\.sock$";
        assert_eq!(
            matches(sock, false, "uds:/tmp/cc-socks/55.sock\n"),
            ["/cc-socks/55.sock"],
            "before a final newline"
        );
        assert_eq!(matches(r"x$", false, "x\r\n"), ["x"]);
        assert!(
            matches(r"x$", false, "x\ny").is_empty() && matches(r"x$", false, "x\n\n").is_empty()
        );
        let heading = Regex::new(r"^[ \t]*(done)[ \t]*:", true, true).unwrap();
        let t: Vec<char> = "a\n  Done: x\nDONE:".chars().collect();
        assert_eq!(heading.find_all(&t), [2..9, 12..17], "multi-line '^'");
        let caret = Regex::new(r"^", true, true).unwrap();
        assert_eq!(
            caret.find_all(&['a', '\n']).len(),
            1,
            "not after a final line break"
        );
        assert!(
            !Regex::new(r"\bcafe\b", true, false)
                .unwrap()
                .is_match("Cafe\u{301}"),
            "a mark joins the word"
        );
        assert_eq!(
            matches(r"[\s:,.–-]*", false, ": –x"),
            [": –", "", ""],
            "empty matches advance"
        );
        assert_eq!(matches(r"\b", false, "ab cd").len(), 4);
        assert_eq!(
            matches(r"\w+", false, "a²b ½c Ⅻ_d‿e"),
            ["a", "b", "c", "Ⅻ_d‿e"]
        );
        assert_eq!(
            matches(r"\s+", false, "a\u{B}b\u{85}c\u{A0}d\u{2028}e\u{200B}f\tg"),
            ["\u{B}", "\u{85}", "\u{A0}", "\u{2028}", "\t"]
        );
        assert_eq!(
            matches(r"[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{N}]", false, "aBǅʰ中1²Ⅻ").concat(),
            "Bǅʰ中1²Ⅻ"
        );
        assert_eq!(matches(r"[^\p{L}\n]*\p{L}", false, "12 #a"), ["12 #a"]);
        assert_eq!(matches(r"\d+", false, "12 ١٢ ½ ²"), ["12", "١٢"]);
        assert!(
            Regex::new(r"a{2,1}", false, false).is_err()
                && Regex::new(r"(a", false, false).is_err()
        );
        assert_eq!(escape("O'Neil.x"), r"O\'Neil\.x");
    }

    #[test]
    fn long_text_does_not_recurse_per_character() {
        let long = "a ".repeat(200_000);
        let r = Regex::new(r"(?:[^.!?]|[.!?](?=\S))+", false, false).unwrap();
        let t: Vec<char> = long.chars().collect();
        assert_eq!(r.find_all(&t).first(), Some(&(0..t.len())));
    }
}
