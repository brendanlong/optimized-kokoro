//! SSML to inline tags (`ssml.py`), used when a speech request sets `ssml: true`.
//!
//! `<break>` becomes `[pause:Ns]`, `<voice>` `[voice:name]`, `<prosody rate>` `[rate:x]`,
//! `<phoneme>` `[word](/ipa/)`, `<sub>` its alias, and every other element its text.

use crate::py::{self, float_repr};
use crate::pyre::{Pattern, compile, compile_i};
use crate::tags::clamp_rate;
use std::sync::LazyLock;

/// `settings.ssml_max_depth`: the deepest element nesting translated.
pub const MAX_DEPTH: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsmlError {
    /// `ET.ParseError` in the reference: malformed XML, a DTD, or nesting past [`MAX_DEPTH`].
    /// The message is the XML parser's own and differs from expat's.
    Malformed(String),
}

impl std::fmt::Display for SsmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(message) => write!(f, "Malformed SSML: {message}"),
        }
    }
}

impl std::error::Error for SsmlError {}

const BREAK_STRENGTH_S: &[(&str, f64)] =
    &[("none", 0.0), ("x-weak", 0.0), ("weak", 0.25), ("medium", 0.5), ("strong", 1.0), ("x-strong", 1.5)];

const PROSODY_RATE: &[(&str, f64)] =
    &[("x-slow", 0.5), ("slow", 0.75), ("medium", 1.0), ("default", 1.0), ("fast", 1.25), ("x-fast", 1.5)];

static TIME_ATTR: LazyLock<Pattern> = LazyLock::new(|| compile_i(r"^\s*(\d+(?:\.\d+)?)\s*(ms|s)\s*$"));
static PROLOG: LazyLock<Pattern> = LazyLock::new(|| compile(r"^(?s:\s|<\?[^>]*\?>|<!--.*?-->)*"));
static SPEAK: LazyLock<Pattern> = LazyLock::new(|| compile(r"^<speak[\s/>]"));
static WHITESPACE: LazyLock<Pattern> = LazyLock::new(|| compile(r"\s+"));

fn break_seconds(el: roxmltree::Node, max_pause_s: f64) -> f64 {
    if let Some(time) = el.attribute("time").filter(|t| !t.is_empty())
        && let Some(c) = TIME_ATTR.captures_at(time, 0)
    {
        let value = py::parse_float(&c[1]);
        let seconds = if c[2].to_lowercase() == "ms" { value / 1000.0 } else { value };
        return seconds.min(max_pause_s);
    }
    let strength = el.attribute("strength").unwrap_or("medium");
    BREAK_STRENGTH_S.iter().find(|(k, _)| *k == strength).map_or(0.5, |(_, v)| *v)
}

fn prosody_rate(value: &str) -> f64 {
    let value = py::strip(value).to_lowercase();
    if let Some((_, rate)) = PROSODY_RATE.iter().find(|(k, _)| *k == value) {
        return *rate;
    }
    let parsed = match value.strip_suffix('%') {
        Some(percent) => py::float(percent).map(|v| v / 100.0),
        None => py::float(&value),
    };
    parsed.map_or(1.0, clamp_rate)
}

fn text_of(el: roxmltree::Node) -> String {
    el.descendants().filter(|n| n.is_text()).filter_map(|n| n.text()).collect()
}

struct Renderer {
    parts: String,
    control_tags: bool,
    max_pause_s: f64,
}

impl Renderer {
    fn render(&mut self, el: roxmltree::Node, voice: &str, rate: f64, depth: usize) -> Result<(), SsmlError> {
        if depth > MAX_DEPTH {
            return Err(SsmlError::Malformed(format!("nesting deeper than {MAX_DEPTH} levels is not translated")));
        }
        let tag = el.tag_name().name().to_lowercase();
        match tag.as_str() {
            "break" => {
                let seconds = break_seconds(el, self.max_pause_s);
                if seconds > 0.0 {
                    self.parts.push_str(&format!(" [pause:{}s] ", float_repr(seconds)));
                }
                return Ok(());
            }
            "phoneme" => {
                if let Some(ph) = el.attribute("ph").filter(|ph| !ph.is_empty())
                    && el.attribute("alphabet").unwrap_or("ipa").to_lowercase() == "ipa"
                {
                    let word = text_of(el);
                    self.parts.push_str(&format!("[{}](/{}/)", py::strip(&word), py::strip(ph)));
                    return Ok(());
                }
            }
            "sub" => {
                match el.attribute("alias").filter(|a| !a.is_empty()) {
                    Some(alias) => self.parts.push_str(alias),
                    None => self.parts.push_str(&text_of(el)),
                }
                return Ok(());
            }
            "desc" => return Ok(()),
            _ => {}
        }
        let mut inner_voice = voice.to_owned();
        let mut inner_rate = rate;
        if let Some(name) = el.attribute("name").filter(|n| tag == "voice" && self.control_tags && !n.is_empty()) {
            inner_voice = py::strip(name).to_owned();
            self.parts.push_str(&format!(" [voice:{inner_voice}] "));
            if rate != 1.0 {
                self.parts.push_str(&format!(" [rate:{}] ", float_repr(rate)));
            }
        }
        if let Some(value) = el.attribute("rate").filter(|r| tag == "prosody" && self.control_tags && !r.is_empty()) {
            inner_rate = prosody_rate(value);
            if inner_rate != rate {
                self.parts.push_str(&format!(" [rate:{}] ", float_repr(inner_rate)));
            }
        }
        let block = tag == "p" || tag == "s";
        if block {
            self.parts.push(' ');
        }
        for child in el.children() {
            if child.is_element() {
                self.render(child, &inner_voice, inner_rate, depth + 1)?;
            } else if let Some(text) = child.text().filter(|_| child.is_text()) {
                self.parts.push_str(text);
            }
        }
        if block {
            self.parts.push(' ');
        }
        if inner_voice != voice {
            self.parts.push_str(&format!(" [voice:{voice}] "));
            if rate != 1.0 {
                self.parts.push_str(&format!(" [rate:{}] ", float_repr(rate)));
            }
        }
        if inner_rate != rate {
            self.parts.push_str(&format!(" [rate:{}] ", float_repr(rate)));
        }
        Ok(())
    }
}

/// `translate_ssml`: native-tag text if the input is SSML (a `<speak>` root after any XML
/// declaration and comments), otherwise the input unchanged. `<voice>` and `<prosody rate>` only
/// produce tags when `allow_voice_tags` is set and there is a default voice to return to.
pub fn translate(text: &str, default_voice: &str, allow_voice_tags: bool, max_pause_s: f64) -> Result<String, SsmlError> {
    let prolog = PROLOG.find(text).map_or(0, |m| m.end);
    let body = &text[prolog..];
    if body.chars().take(9).collect::<String>().to_uppercase() == "<!DOCTYPE" {
        return Err(SsmlError::Malformed("DTD is not supported".into()));
    }
    if !SPEAK.is_match(body) {
        return Ok(text.to_owned());
    }
    let document = roxmltree::Document::parse(text).map_err(|e| SsmlError::Malformed(e.to_string()))?;
    let mut renderer =
        Renderer { parts: String::new(), control_tags: allow_voice_tags && !default_voice.is_empty(), max_pause_s };
    renderer.render(document.root_element(), default_voice, 1.0, 0)?;
    Ok(py::strip(&WHITESPACE.sub_str(&renderer.parts, " ")).to_owned())
}
