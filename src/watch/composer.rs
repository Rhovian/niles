//! Styled pane text at the lead's composer boundary.

use crate::agents::ComposerState;

#[derive(Clone, Copy)]
pub(crate) enum Anchor {
    Claude(&'static str),
    Codex(&'static str),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Unknown,
    Default,
    Dim,
}

struct StyledLine {
    plain: String,
    chars: Vec<(char, Style)>,
}

impl StyledLine {
    fn parse(input: &str, initial: Style) -> Option<Self> {
        let mut plain = String::new();
        let mut chars = Vec::new();
        let mut style = initial;
        for token in Tokens(input) {
            match token? {
                Token::Text(ch) => {
                    plain.push(ch);
                    chars.push((ch, style));
                }
                Token::Sgr(params) => style = sgr_style(style, params)?,
                Token::Osc => {}
            }
        }
        Some(Self { plain, chars })
    }

    fn text_after(&self, prefix: &str) -> ComposerState {
        if !self.plain.starts_with(prefix) {
            return ComposerState::Unknown;
        }
        self.first_text(prefix.chars().count())
    }

    fn first_text(&self, skip: usize) -> ComposerState {
        match self
            .chars
            .iter()
            .skip(skip)
            .find(|(ch, _)| !ch.is_whitespace())
        {
            Some((_, Style::Default)) => ComposerState::Typed,
            Some((_, Style::Dim)) => ComposerState::Empty,
            Some((_, Style::Unknown)) => ComposerState::Unknown,
            None => ComposerState::Empty,
        }
    }
}

enum Token<'a> {
    Text(char),
    Sgr(&'a str),
    Osc,
}

/// One tokenizer for visible text and SGR. Other escape sequences make the screen unknown.
struct Tokens<'a>(&'a str);

impl<'a> Iterator for Tokens<'a> {
    type Item = Option<Token<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let ch = self.0.chars().next()?;
        if ch != '\x1b' {
            self.0 = &self.0[ch.len_utf8()..];
            return Some(Some(Token::Text(ch)));
        }
        if let Some(rest) = self.0.strip_prefix("\x1b]") {
            let Some(end) = rest.find("\x1b\\") else {
                self.0 = "";
                return Some(None);
            };
            self.0 = &rest[end + 2..];
            return Some(Some(Token::Osc));
        }
        let Some(rest) = self.0.strip_prefix("\x1b[") else {
            self.0 = "";
            return Some(None);
        };
        let Some(end) = rest.find('m') else {
            self.0 = "";
            return Some(None);
        };
        let params = &rest[..end];
        self.0 = &rest[end + 1..];
        if params.chars().all(|ch| ch.is_ascii_digit() || ch == ';') {
            Some(Some(Token::Sgr(params)))
        } else {
            Some(None)
        }
    }
}

fn sgr_style(mut style: Style, params: &str) -> Option<Style> {
    // Extended colours consume their own 2/0 parameters. Their presence is unknown styling,
    // regardless of the component values or any other attributes in the sequence.
    let codes: Vec<_> = params.split(';').collect();
    if codes.iter().any(|code| *code == "38" || *code == "48") {
        return Some(Style::Unknown);
    }
    for code in codes {
        style = match code {
            "" | "0" | "22" => Style::Default,
            "2" => Style::Dim,
            "39" if style != Style::Dim => Style::Default,
            "39" => Style::Dim,
            "1" | "7" | "49" => style,
            _ => Style::Unknown,
        };
    }
    Some(style)
}

fn screen_lines(capture: &str) -> Option<Vec<StyledLine>> {
    capture
        .lines()
        .map(|line| StyledLine::parse(line, Style::Unknown))
        .collect()
}

fn rule(line: &StyledLine) -> bool {
    line.plain.starts_with('─') && line.plain.chars().filter(|ch| *ch == '─').count() >= 20
}

pub(crate) fn recognize(capture: &str, anchor: Anchor) -> ComposerState {
    let Some(lines) = screen_lines(capture) else {
        return ComposerState::Unknown;
    };
    match anchor {
        Anchor::Claude(prompt) => claude(&lines, capture, prompt),
        Anchor::Codex(prompt) => codex(&lines, prompt),
    }
}

fn claude(lines: &[StyledLine], capture: &str, marker: &str) -> ComposerState {
    let Some((index, prompt)) = lines
        .iter()
        .enumerate()
        .rev()
        .find(|(_, line)| line.plain.starts_with(marker))
    else {
        return ComposerState::Unknown;
    };
    if index == 0 || !rule(&lines[index - 1]) {
        return ComposerState::Unknown;
    }
    let Some(bottom) = lines
        .iter()
        .enumerate()
        .skip(index + 1)
        .find(|(_, line)| rule(line))
        .map(|(i, _)| i)
    else {
        return ComposerState::Unknown;
    };
    if bottom + 1 >= lines.len()
        || !lines[bottom + 1].plain.trim_start().starts_with("⏵⏵")
        || lines[bottom + 2..]
            .iter()
            .any(|line| !line.plain.trim().is_empty())
        || lines[index + 1..bottom]
            .iter()
            .any(|line| !line.plain.starts_with("  "))
    {
        return ComposerState::Unknown;
    }
    let first = prompt.text_after(marker);
    if first != ComposerState::Empty {
        return first;
    }
    // A first blank row can precede a Shift+Enter continuation. The reset before the prompt
    // establishes the inherited default style for unstyled continuation rows.
    let initial = prompt.chars.first().map(|(_, style)| *style);
    let Some(initial) = initial else {
        return ComposerState::Unknown;
    };
    for row in capture.lines().skip(index + 1).take(bottom - index - 1) {
        let Some(line) = StyledLine::parse(row, initial) else {
            return ComposerState::Unknown;
        };
        match line.first_text(0) {
            ComposerState::Empty => continue,
            state @ (ComposerState::Typed | ComposerState::Unknown) => return state,
        }
    }
    ComposerState::Empty
}

fn codex(lines: &[StyledLine], marker: &str) -> ComposerState {
    let Some((index, prompt)) = lines
        .iter()
        .enumerate()
        .rev()
        .find(|(_, line)| line.plain.starts_with(marker))
    else {
        return ComposerState::Unknown;
    };
    // The composer has a blank row above it, a blank row below it, then exactly two footer rows.
    // Popup choices and transcript user rows do not have this surrounding structure.
    if index == 0
        || !lines[index - 1].plain.trim().is_empty()
        || index + 3 >= lines.len()
        || !lines[index + 1].plain.trim().is_empty()
        || !lines[index + 2].plain.starts_with("  ")
        || !lines[index + 2].plain.contains(" · ")
        || (!lines[index + 3].plain.contains("← for agents")
            && !lines[index + 3].plain.contains("warning"))
        || lines[index + 4..]
            .iter()
            .any(|line| !line.plain.trim().is_empty())
    {
        return ComposerState::Unknown;
    }
    prompt.text_after(marker)
}
