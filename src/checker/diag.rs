//! 诊断：类型检查的输出单元。

use crate::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// 类型错误：编辑器红线、`tyto check` exit 1（但永不阻塞运行）
    Error,
    /// 提示性警告（如同层重复声明）
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub span: Span,
    pub severity: Severity,
    pub message: String,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Diagnostic { span, severity: Severity::Error, message: message.into() }
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Diagnostic { span, severity: Severity::Warning, message: message.into() }
    }
}
