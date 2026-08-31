//! Bounded and secret-redacted diagnostics summary (ADR 0006, P1.2).
//!
//! Raw diagnostics and transport logs must never leak secrets (tokens,
//! keys, passwords, bearer credentials) into the domain or projection
//! layers, and must remain bounded to avoid memory amplification.

use std::fmt;

/// Maximum size of a diagnostics summary in bytes.
pub const MAX_DIAGNOSTICS_BYTES: usize = 4096;

/// A bounded, secret-redacted diagnostics summary.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct BoundedDiagnosticsSummary(String);

impl BoundedDiagnosticsSummary {
    /// Creates a bounded, redacted summary from a raw string.
    #[must_use]
    pub fn from_raw(raw: &str) -> Self {
        let redacted = redact_secrets(raw);
        let bounded = if redacted.len() > MAX_DIAGNOSTICS_BYTES {
            let mut cut = MAX_DIAGNOSTICS_BYTES;
            while !redacted.is_char_boundary(cut) && cut > 0 {
                cut -= 1;
            }
            let mut truncated = redacted[..cut].to_string();
            truncated.push_str("... [truncated]");
            truncated
        } else {
            redacted
        };
        Self(bounded)
    }

    /// Returns the summary as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the summary is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for BoundedDiagnosticsSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for BoundedDiagnosticsSummary {
    fn from(value: &str) -> Self {
        Self::from_raw(value)
    }
}

impl From<String> for BoundedDiagnosticsSummary {
    fn from(value: String) -> Self {
        Self::from_raw(&value)
    }
}

/// Redacts sensitive secret patterns from a string slice without external regex dependencies.
fn redact_secrets(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // 1. Authorization / bearer tokens: "bearer " or "Bearer "
        if starts_with_ignore_ascii_case(&chars[i..], "bearer ") {
            out.push_str("Bearer [REDACTED]");
            i += 7;
            while i < chars.len() && is_token_char(chars[i]) {
                i += 1;
            }
            continue;
        }

        // 2. Authorization header: "authorization: " / "Authorization: "
        if starts_with_ignore_ascii_case(&chars[i..], "authorization:") {
            out.push_str("Authorization: [REDACTED]");
            i += 14;
            while i < chars.len() && chars[i] != '\n' && chars[i] != '\r' {
                i += 1;
            }
            continue;
        }

        // 3. API key prefixes like "sk-" or "sk_" / "SK_" (OpenAI / canary fixture style)
        if (starts_with_ignore_ascii_case(&chars[i..], "sk-")
            || starts_with_ignore_ascii_case(&chars[i..], "sk_"))
            && i + 3 < chars.len()
        {
            out.push_str("sk-[REDACTED]");
            i += 3;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '-' || chars[i] == '_')
            {
                i += 1;
            }
            continue;
        }

        // 4. Key-value secrets: "password=", "token=", "secret=", "api_key=", "apikey="
        let kv_patterns = [
            ("password=", 9),
            ("token=", 6),
            ("secret=", 7),
            ("api_key=", 8),
            ("apikey=", 7),
        ];

        let mut matched_kv = false;
        for (pattern, plen) in kv_patterns {
            if starts_with_ignore_ascii_case(&chars[i..], pattern) {
                out.push_str(pattern);
                out.push_str("[REDACTED]");
                i += plen;
                while i < chars.len() && !is_secret_delimiter(chars[i]) {
                    i += 1;
                }
                matched_kv = true;
                break;
            }
        }
        if matched_kv {
            continue;
        }

        out.push(chars[i]);
        i += 1;
    }

    out
}

fn starts_with_ignore_ascii_case(slice: &[char], prefix: &str) -> bool {
    if slice.len() < prefix.len() {
        return false;
    }
    for (a, b) in slice.iter().zip(prefix.chars()) {
        if !a.eq_ignore_ascii_case(&b) {
            return false;
        }
    }
    true
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || c == '.'
        || c == '-'
        || c == '_'
        || c == '~'
        || c == '+'
        || c == '/'
        || c == '='
}

fn is_secret_delimiter(c: char) -> bool {
    c.is_whitespace()
        || c == '&'
        || c == ';'
        || c == ','
        || c == '"'
        || c == '\''
        || c == '}'
        || c == ']'
        || c == '\n'
        || c == '\r'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_bearer_token() {
        let raw =
            "Error during request: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.xyz in headers";
        let summary = BoundedDiagnosticsSummary::from_raw(raw);
        assert!(!summary.as_str().contains("eyJhbGci"));
        assert!(summary.as_str().contains("Bearer [REDACTED] in headers"));
    }

    #[test]
    fn redacts_sk_tokens_and_kv_pairs() {
        let raw = "Failed with token=secret123&sk-abcdef123456789 and password=mysecretpass";
        let summary = BoundedDiagnosticsSummary::from_raw(raw);
        assert!(!summary.as_str().contains("secret123"));
        assert!(!summary.as_str().contains("abcdef123456789"));
        assert!(!summary.as_str().contains("mysecretpass"));
        assert!(
            summary
                .as_str()
                .contains("token=[REDACTED]&sk-[REDACTED] and password=[REDACTED]")
        );
    }

    #[test]
    fn caps_long_diagnostics() {
        let long_str = "A".repeat(5000);
        let summary = BoundedDiagnosticsSummary::from_raw(&long_str);
        assert!(summary.len() <= MAX_DIAGNOSTICS_BYTES + 32);
        assert!(summary.as_str().ends_with("... [truncated]"));
    }
}
