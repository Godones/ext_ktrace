use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::{
    fmt::{self, Write},
    marker::PhantomData,
};

use crate::runtime::{DebugOps, DebugSite, flag_mask_for};

/// Public error type for control-plane operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    msg: String,
}

impl Error {
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        Self { msg: msg.into() }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

/// A file-like handle that models Linux dynamic debug's `control` file.
#[derive(Debug)]
pub struct ControlFile<K: DebugOps + 'static> {
    sites: Vec<&'static DebugSite<K>>,
    _ops: PhantomData<K>,
}

impl<K: DebugOps + 'static> ControlFile<K> {
    /// Canonical procfs path used by Linux dynamic debug.
    pub const PROC_PATH: &'static str = "/proc/dynamic_debug/control";
    /// Debugfs alias commonly used by Linux tracing tooling.
    pub const DEBUGFS_PATH: &'static str = "/sys/kernel/debug/dynamic_debug/control";

    pub(crate) fn new(sites: Vec<&'static DebugSite<K>>) -> Self {
        Self {
            sites,
            _ops: PhantomData,
        }
    }

    /// Returns the common procfs path for this control file.
    pub const fn procfs_path(&self) -> &'static str {
        Self::PROC_PATH
    }

    /// Returns the common debugfs alias path for the same control file.
    pub const fn debugfs_path(&self) -> &'static str {
        Self::DEBUGFS_PATH
    }

    /// Returns the number of registered callsites tracked by this control file.
    pub fn site_count(&self) -> usize {
        self.sites.len()
    }

    /// Reads the current dynamic debug catalog.
    pub fn read(&self) -> Result<String, Error> {
        let mut out = "# filename:lineno [module]function flags format\n".to_string();
        for site in self.sites.iter().copied() {
            write_site_line(&mut out, site);
        }
        Ok(out)
    }

    /// Applies one or more dynamic debug commands.
    pub fn write(&mut self, query: &str) -> Result<usize, Error> {
        let commands = parse_commands(query)?;

        let mut total_matches = 0usize;
        for command in commands {
            total_matches += apply_command(&command, &self.sites);
        }
        Ok(total_matches)
    }
}

fn apply_command<K: DebugOps>(command: &Command, sites: &[&'static DebugSite<K>]) -> usize {
    let mut matches = 0usize;
    for site in sites.iter().copied() {
        if !command.matches(site) {
            continue;
        }
        site.set_flags(command.apply(site.flags()));
        matches += 1;
    }
    matches
}

fn write_site_line<K: DebugOps>(out: &mut String, site: &'static DebugSite<K>) {
    let function = site.function();
    if function.is_empty() {
        let _ = writeln!(
            out,
            "{}:{} [{}] {} \"{}\"",
            site.file(),
            site.line(),
            site.module(),
            site.flags_string(),
            site.format()
        );
    } else {
        let _ = writeln!(
            out,
            "{}:{} [{}]{} {} \"{}\"",
            site.file(),
            site.line(),
            site.module(),
            function,
            site.flags_string(),
            site.format()
        );
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Command {
    selectors: Vec<Selector>,
    operation: FlagOp,
}

impl Command {
    pub(crate) fn matches<K: DebugOps>(&self, site: &DebugSite<K>) -> bool {
        self.selectors.iter().all(|selector| selector.matches(site))
    }

    pub(crate) fn apply(&self, current: u8) -> u8 {
        self.operation.apply(current)
    }
}

#[derive(Debug, Clone)]
enum Selector {
    File(String),
    Func(String),
    Module(String),
    Line(LineRange),
    Format(String),
}

impl Selector {
    fn matches<K: DebugOps>(&self, site: &DebugSite<K>) -> bool {
        match self {
            Selector::File(pattern) => matches_file(pattern, site.file()),
            Selector::Func(pattern) => matches_func(pattern, site.function()),
            Selector::Module(pattern) => matches_module(pattern, site.module()),
            Selector::Line(range) => range.contains(site.line()),
            Selector::Format(pattern) => matches_format(pattern, site.format()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct LineRange {
    start: u32,
    end: u32,
}

impl LineRange {
    fn contains(&self, line: u32) -> bool {
        self.start <= line && line <= self.end
    }
}

#[derive(Debug, Clone, Copy)]
enum FlagOpKind {
    Add,
    Remove,
    Replace,
}

#[derive(Debug, Clone, Copy)]
struct FlagOp {
    kind: FlagOpKind,
    mask: u8,
}

impl FlagOp {
    fn apply(self, current: u8) -> u8 {
        match self.kind {
            FlagOpKind::Add => current | self.mask,
            FlagOpKind::Remove => current & !self.mask,
            FlagOpKind::Replace => self.mask,
        }
    }
}

pub(crate) fn parse_commands(input: &str) -> Result<Vec<Command>, Error> {
    let token_lines = tokenize(input)?;
    if token_lines.is_empty() {
        return Err(Error::new("empty dynamic debug command"));
    }

    let mut commands = Vec::with_capacity(token_lines.len());
    for tokens in token_lines {
        commands.push(parse_command(tokens)?);
    }
    Ok(commands)
}

fn parse_command(tokens: Vec<String>) -> Result<Command, Error> {
    let mut selectors = Vec::new();
    let mut operation = None;
    let mut index = 0usize;

    while index < tokens.len() {
        let token = &tokens[index];
        if is_flag_token(token) {
            if operation.is_some() {
                return Err(Error::new("multiple flag operations in one command"));
            }
            operation = Some(parse_flag_op(token)?);
            index += 1;
            continue;
        }

        match token.as_str() {
            "file" => {
                let value = expect_value(&tokens, index, "file")?;
                selectors.push(Selector::File(value.clone()));
                index += 2;
            }
            "func" => {
                let value = expect_value(&tokens, index, "func")?;
                selectors.push(Selector::Func(value.clone()));
                index += 2;
            }
            "module" => {
                let value = expect_value(&tokens, index, "module")?;
                selectors.push(Selector::Module(value.clone()));
                index += 2;
            }
            "line" => {
                let value = expect_value(&tokens, index, "line")?;
                selectors.push(Selector::Line(parse_line_range(value)?));
                index += 2;
            }
            "format" => {
                let value = expect_value(&tokens, index, "format")?;
                selectors.push(Selector::Format(value.clone()));
                index += 2;
            }
            unknown => {
                return Err(Error::new(format!(
                    "unknown dynamic debug token `{unknown}`"
                )));
            }
        }
    }

    let operation = operation.ok_or_else(|| Error::new("missing flag operation such as `+p`"))?;
    Ok(Command {
        selectors,
        operation,
    })
}

fn expect_value<'a>(
    tokens: &'a [String],
    index: usize,
    keyword: &str,
) -> Result<&'a String, Error> {
    tokens
        .get(index + 1)
        .ok_or_else(|| Error::new(format!("missing value after `{keyword}`")))
}

fn parse_line_range(text: &str) -> Result<LineRange, Error> {
    if let Some((start, end)) = text.split_once('-') {
        let start = parse_u32(start, "line range start")?;
        let end = parse_u32(end, "line range end")?;
        if start > end {
            return Err(Error::new(format!(
                "invalid line range `{text}`: start is greater than end"
            )));
        }
        return Ok(LineRange { start, end });
    }

    let line = parse_u32(text, "line")?;
    Ok(LineRange {
        start: line,
        end: line,
    })
}

fn parse_u32(text: &str, what: &str) -> Result<u32, Error> {
    text.parse::<u32>()
        .map_err(|_| Error::new(format!("invalid {what} `{text}`")))
}

fn is_flag_token(token: &str) -> bool {
    token
        .as_bytes()
        .first()
        .is_some_and(|head| matches!(head, b'+' | b'-' | b'='))
}

fn parse_flag_op(token: &str) -> Result<FlagOp, Error> {
    let mut chars = token.chars();
    let head = chars
        .next()
        .ok_or_else(|| Error::new("empty flag operation"))?;
    let rest = chars.as_str();
    if head == '=' && rest == "_" {
        return Ok(FlagOp {
            kind: FlagOpKind::Replace,
            mask: 0,
        });
    }
    if rest.is_empty() {
        return Err(Error::new(format!(
            "flag operation `{token}` does not specify any flags"
        )));
    }

    let mut mask = 0u8;
    for chr in rest.chars() {
        if chr == '_' {
            return Err(Error::new(format!(
                "underscore is only valid in the exact form `=_`, got `{token}`"
            )));
        }
        let flag = flag_mask_for(chr).ok_or_else(|| {
            Error::new(format!("unknown dynamic debug flag `{chr}` in `{token}`"))
        })?;
        mask |= flag;
    }

    let kind = match head {
        '+' => FlagOpKind::Add,
        '-' => FlagOpKind::Remove,
        '=' => FlagOpKind::Replace,
        _ => unreachable!(),
    };
    Ok(FlagOp { kind, mask })
}

fn tokenize(input: &str) -> Result<Vec<Vec<String>>, Error> {
    let mut commands = Vec::new();
    let mut current = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escape = false;

    for chr in input.chars() {
        if escape {
            token.push(chr);
            escape = false;
            continue;
        }

        match quote {
            Some(quoted) => {
                if chr == '\\' {
                    escape = true;
                } else if chr == quoted {
                    quote = None;
                } else {
                    token.push(chr);
                }
            }
            None => match chr {
                '\\' => escape = true,
                '"' | '\'' => quote = Some(chr),
                ';' | '\n' => {
                    finish_token(&mut current, &mut token);
                    if !current.is_empty() {
                        commands.push(current);
                        current = Vec::new();
                    }
                }
                c if c.is_whitespace() => finish_token(&mut current, &mut token),
                _ => token.push(chr),
            },
        }
    }

    if quote.is_some() {
        return Err(Error::new(
            "unterminated quoted string in dynamic debug command",
        ));
    }
    if escape {
        token.push('\\');
    }

    finish_token(&mut current, &mut token);
    if !current.is_empty() {
        commands.push(current);
    }
    Ok(commands)
}

fn finish_token(current: &mut Vec<String>, token: &mut String) {
    if token.is_empty() {
        return;
    }
    current.push(core::mem::take(token));
}

fn matches_file(pattern: &str, value: &str) -> bool {
    if has_glob(pattern) {
        return glob_match(pattern, value);
    }
    value == pattern || value.ends_with(pattern)
}

fn matches_module(pattern: &str, value: &str) -> bool {
    if has_glob(pattern) {
        return glob_match(pattern, value);
    }
    if value == pattern {
        return true;
    }
    value
        .strip_suffix(pattern)
        .is_some_and(|prefix| prefix.ends_with("::"))
}

fn matches_func(pattern: &str, value: &str) -> bool {
    if has_glob(pattern) {
        return glob_match(pattern, value)
            || value
                .rsplit("::")
                .next()
                .is_some_and(|tail| glob_match(pattern, tail));
    }
    value == pattern
        || value
            .rsplit("::")
            .next()
            .is_some_and(|tail| tail == pattern)
}

fn matches_format(pattern: &str, value: &str) -> bool {
    if has_glob(pattern) {
        return glob_match(pattern, value);
    }
    value.contains(pattern)
}

fn has_glob(pattern: &str) -> bool {
    pattern
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'*' | b'?'))
}

fn glob_match(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();

    let mut pat_idx = 0usize;
    let mut val_idx = 0usize;
    let mut star_idx = None;
    let mut star_match = 0usize;

    while val_idx < value.len() {
        if pat_idx < pattern.len()
            && (pattern[pat_idx] == b'?' || pattern[pat_idx] == value[val_idx])
        {
            pat_idx += 1;
            val_idx += 1;
        } else if pat_idx < pattern.len() && pattern[pat_idx] == b'*' {
            star_idx = Some(pat_idx);
            pat_idx += 1;
            star_match = val_idx;
        } else if let Some(star) = star_idx {
            pat_idx = star + 1;
            star_match += 1;
            val_idx = star_match;
        } else {
            return false;
        }
    }

    while pat_idx < pattern.len() && pattern[pat_idx] == b'*' {
        pat_idx += 1;
    }
    pat_idx == pattern.len()
}

#[cfg(test)]
pub(crate) fn parse_commands_for_tests(input: &str) -> Result<Vec<Command>, Error> {
    parse_commands(input)
}

#[cfg(test)]
pub(crate) fn flag_mask_for_tests(command: &Command) -> u8 {
    command.operation.mask
}

#[cfg(test)]
pub(crate) fn flag_op_is_add(command: &Command) -> bool {
    matches!(command.operation.kind, FlagOpKind::Add)
}

#[cfg(test)]
pub(crate) fn flag_op_is_replace(command: &Command) -> bool {
    matches!(command.operation.kind, FlagOpKind::Replace)
}

#[cfg(test)]
pub(crate) fn expected_print_mask() -> u8 {
    flag_mask_for('p').unwrap()
}
