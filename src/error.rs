use colored::Colorize;

pub type RtResult<T> = Result<T, RtError>;

/// 源码位置（1-based）；derive 出的字典序 = 源码位置先后
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

impl Span {
    pub fn new(line: usize, col: usize) -> Self {
        Span { line, col }
    }
}

#[derive(Debug, Clone)]
pub enum RtError {
    Lex { span: Span, message: String },
    Parse { span: Span, message: String },
    Runtime { span: Option<Span>, message: String },
}

impl RtError {
    pub fn lex(span: Span, message: impl Into<String>) -> Self {
        RtError::Lex { span, message: message.into() }
    }

    pub fn parse(span: Span, message: impl Into<String>) -> Self {
        RtError::Parse { span, message: message.into() }
    }

    pub fn runtime(span: Option<Span>, message: impl Into<String>) -> Self {
        RtError::Runtime { span, message: message.into() }
    }

    /// 为缺少位置信息的 runtime 错误补上 span
    pub fn with_span(mut self, span: Span) -> Self {
        if let RtError::Runtime { span: s, .. } = &mut self {
            if s.is_none() {
                *s = Some(span);
            }
        }
        self
    }

    /// 生成人类可读的错误报告；提供源码时附带行摘录与指向箭头
    pub fn report(&self, source: Option<&str>) -> String {
        let (kind, span, message) = match self {
            RtError::Lex { span, message } => ("lex error", Some(*span), message),
            RtError::Parse { span, message } => ("parse error", Some(*span), message),
            RtError::Runtime { span, message } => ("runtime error", *span, message),
        };

        let mut out = format!("{} {}", kind.red().bold(), message);
        if let Some(span) = span {
            out.push_str(&format!(
                " {} ",
                format!("(line {}, col {})", span.line, span.col).dimmed()
            ));
            if let Some(src) = source {
                out.push('\n');
                out.push_str(&render_excerpt(src, span));
            }
        }
        out
    }
}

/// 渲染出错行的摘录，并在指定列下方放置箭头
fn render_excerpt(source: &str, span: Span) -> String {
    let line = source.lines().nth(span.line.saturating_sub(1));
    let Some(line) = line else { return String::new() };

    let line_no = span.line.to_string();
    let gutter = " ".repeat(line_no.len());
    let caret_pad = " ".repeat(span.col.saturating_sub(1));

    format!(
        "{gutter} |\n{line_no} | {line}\n{gutter} | {caret_pad}^",
        gutter = gutter.dimmed(),
        line_no = line_no.dimmed(),
        line = line,
        caret_pad = caret_pad,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_includes_position_and_excerpt() {
        let src = "x = 1\ny = z";
        let err = RtError::runtime(Some(Span::new(2, 5)), "undefined variable `z`");
        let text = err.report(Some(src));
        assert!(text.contains("runtime error"));
        assert!(text.contains("undefined variable `z`"));
        assert!(text.contains("line 2, col 5"));
        assert!(text.contains("y = z"));
        assert!(text.contains("^"));
    }

    #[test]
    fn report_without_source_still_has_position() {
        let err = RtError::parse(Span::new(3, 12), "expected '{'");
        let text = err.report(None);
        assert!(text.contains("parse error"));
        assert!(text.contains("expected '{'"));
        assert!(text.contains("line 3, col 12"));
    }
}
