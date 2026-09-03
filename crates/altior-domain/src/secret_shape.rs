//! Secret-shaped content detector (ADR 0017, AGENTS.md "Memory").
//!
//! A pure, deterministic filter that identifies credential-shaped and secret-shaped
//! patterns in candidate memory content and provenance excerpts before durable write
//! or sync.
//!
//! Fails closed on ambiguity: false positives are acceptable to guarantee that
//! credentials and private keys never enter the journal or database.

/// Returns `true` if the candidate string contains any secret-shaped pattern.
#[must_use]
pub fn is_secret_shaped(candidate: &str) -> bool {
    let candidate_trimmed = candidate.trim();
    if candidate_trimmed.is_empty() {
        return false;
    }

    // 1. PEM private key blocks (e.g., "-----BEGIN PRIVATE KEY-----", "-----BEGIN RSA PRIVATE KEY-----")
    if detect_pem_block(candidate) {
        return true;
    }

    // 2. AWS Access Key IDs (AKIA followed by 16 uppercase alphanumeric characters)
    if detect_aws_access_key(candidate) {
        return true;
    }

    // 3. API keys starting with sk- (e.g., OpenAI sk-..., Anthropic sk-ant-...)
    if detect_sk_api_key(candidate) {
        return true;
    }

    // 4. GitHub tokens (ghp_, gho_, ghu_, ghs_, ghr_)
    if detect_github_token(candidate) {
        return true;
    }

    // 5. Slack tokens (xoxb-, xoxp-, xoxa-, xoxr-, xoxs-)
    if detect_slack_token(candidate) {
        return true;
    }

    // 6. JWT token shape (three base64url segments, typically starting with eyJ)
    if detect_jwt_shape(candidate) {
        return true;
    }

    // 7. Key-value credential assignments (e.g. password=..., api_key: ..., secret := ...)
    if detect_credential_assignment(candidate) {
        return true;
    }

    // 8. Long hex runs (>= 32 consecutive hex chars)
    if detect_long_hex_run(candidate) {
        return true;
    }

    // 9. Long base64 runs adjacent to key identifiers or ending in padding
    if detect_long_base64_run(candidate, 40) {
        return true;
    }

    false
}

/// Detects PEM private key headers.
fn detect_pem_block(s: &str) -> bool {
    let upper = s.to_ascii_uppercase();
    if let Some(pos) = upper.find("BEGIN ") {
        let rest = &upper[pos + 6..];
        if let Some(priv_pos) = rest.find("PRIVATE KEY") {
            let between = &rest[..priv_pos];
            // Header must be reasonably short (e.g., "RSA ", "EC ", "OPENSSH ", or empty)
            if between.len() <= 32 {
                return true;
            }
        }
    }
    false
}

/// Detects AWS access key IDs (`AKIA` + 16 chars from `[A-Z0-9]`).
fn detect_aws_access_key(s: &str) -> bool {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes.len() - i >= 20
            && &bytes[i..i + 4] == b"AKIA"
            && bytes[i..i + 20]
                .iter()
                .all(|&b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            // Boundary check: not immediately preceded or followed by alphanumeric
            let prev_ok = i == 0 || !is_ident_char(bytes[i - 1]);
            let next_ok = i + 20 >= bytes.len() || !is_ident_char(bytes[i + 20]);
            if prev_ok && next_ok {
                return true;
            }
        }
    }
    false
}

/// Detects `sk-` prefixed API keys (`OpenAI` / Anthropic style, min 20 chars body).
fn detect_sk_api_key(s: &str) -> bool {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes.len() - i >= 23 && &bytes[i..i + 3] == b"sk-" {
            let prev_ok = i == 0 || !is_ident_char(bytes[i - 1]);
            if prev_ok {
                // Check body of at least 20 chars from alphanumeric, hyphen, underscore
                let body_len = bytes[i + 3..]
                    .iter()
                    .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                    .count();
                if body_len >= 20 {
                    return true;
                }
            }
        }
    }
    false
}

/// Detects GitHub tokens (`ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_` + 20..=64 chars).
fn detect_github_token(s: &str) -> bool {
    let prefixes = [b"ghp_", b"gho_", b"ghu_", b"ghs_", b"ghr_"];
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        for prefix in &prefixes {
            if bytes.len() - i >= prefix.len() + 20 && &bytes[i..i + prefix.len()] == *prefix {
                let prev_ok = i == 0 || !is_ident_char(bytes[i - 1]);
                if prev_ok {
                    let body_len = bytes[i + prefix.len()..]
                        .iter()
                        .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'_')
                        .count();
                    if body_len >= 20 {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Detects Slack tokens (`xoxb-`, `xoxp-`, `xoxa-`, `xoxr-`, `xoxs-` + 10+ chars).
fn detect_slack_token(s: &str) -> bool {
    let prefixes = [b"xoxb-", b"xoxp-", b"xoxa-", b"xoxr-", b"xoxs-"];
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        for prefix in &prefixes {
            if bytes.len() - i >= prefix.len() + 10 && &bytes[i..i + prefix.len()] == *prefix {
                let prev_ok = i == 0 || !is_ident_char(bytes[i - 1]);
                if prev_ok {
                    let body_len = bytes[i + prefix.len()..]
                        .iter()
                        .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'-')
                        .count();
                    if body_len >= 10 {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Detects JWT tokens: `eyJ... . eyJ... . ...` (three base64url parts, at least first starts with `eyJ`).
fn detect_jwt_shape(s: &str) -> bool {
    for word in s.split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '`') {
        let trimmed = word.trim_matches(|c: char| matches!(c, ',' | ';' | '(' | ')' | '<' | '>'));
        if !trimmed.starts_with("eyJ") {
            continue;
        }
        let parts: Vec<&str> = trimmed.split('.').collect();
        if parts.len() == 3
            && parts.iter().all(|p| {
                !p.is_empty()
                    && p.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
            && parts[0].len() >= 8
            && parts[1].len() >= 8
            && parts[2].len() >= 8
        {
            return true;
        }
    }
    false
}

/// Detects key-value credential assignments.
/// Matches keyword (`password`, `passwd`, `secret`, `token`, `api_key`, `apikey`, `access_token`, `auth_token`, `private_key`)
/// followed by separator (`:`, `=`, `:=`), optional quotes, and value of length >= 20 or containing high entropy.
fn detect_credential_assignment(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    let keywords = [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "access_token",
        "auth_token",
        "private_key",
    ];

    for kw in keywords {
        let mut offset = 0;
        while let Some(pos) = lower[offset..].find(kw) {
            let kw_start = offset + pos;
            let kw_end = kw_start + kw.len();
            offset = kw_end;

            // Ensure word boundary before keyword
            if kw_start > 0 && is_ident_char(lower.as_bytes()[kw_start - 1]) {
                continue;
            }

            // Look ahead for separator
            let remainder = lower[kw_end..].trim_start();
            let sep_len = if remainder.starts_with(":=") {
                2
            } else if remainder.starts_with('=') || remainder.starts_with(':') {
                1
            } else {
                continue;
            };

            let val_part = remainder[sep_len..].trim_start();
            let val_clean = val_part.trim_start_matches(['"', '\'']);
            let val_token = val_clean
                .split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',' || c == ';')
                .next()
                .unwrap_or("");

            // If value token is >= 20 characters and contains alphanumeric chars
            if val_token.len() >= 20 && val_token.chars().any(|c| c.is_ascii_alphanumeric()) {
                return true;
            }
        }
    }
    false
}

/// Detects long uninterrupted hex runs (>= `min_len` hex digits).
fn detect_long_hex_run(s: &str) -> bool {
    detect_long_hex_run_len(s, 32)
}

fn detect_long_hex_run_len(s: &str, min_len: usize) -> bool {
    let mut current_len = 0;
    for &b in s.as_bytes() {
        if b.is_ascii_hexdigit() {
            current_len += 1;
            if current_len >= min_len {
                return true;
            }
        } else {
            current_len = 0;
        }
    }
    false
}

/// Detects long base64 runs (>= `min_len` chars) adjacent to credential indicators or standalone.
fn detect_long_base64_run(s: &str, min_len: usize) -> bool {
    for word in s.split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '`') {
        let trimmed = word.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '=' | '(' | ')'));
        if trimmed.len() >= min_len
            && trimmed
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c == '-' || c == '_')
            // Require mix of cases/digits or standard base64 padding to distinguish from plain prose
            && (trimmed.contains('+')
                || trimmed.contains('/')
                || trimmed.ends_with('=')
                || (trimmed.chars().any(|c| c.is_ascii_uppercase())
                    && trimmed.chars().any(|c| c.is_ascii_lowercase())
                    && trimmed.chars().any(|c| c.is_ascii_digit())
                    && trimmed.len() >= 48))
        {
            return true;
        }
    }
    false
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_pem_private_key_blocks() {
        assert!(is_secret_shaped(
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEogIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----"
        ));
        assert!(is_secret_shaped(
            "-----BEGIN PRIVATE KEY-----\nkey_content\n-----END PRIVATE KEY-----"
        ));
        assert!(is_secret_shaped(
            "-----BEGIN EC PRIVATE KEY-----\nkey\n-----END EC PRIVATE KEY-----"
        ));
        assert!(is_secret_shaped(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nkey\n-----END OPENSSH PRIVATE KEY-----"
        ));
    }

    #[test]
    fn detects_aws_access_keys() {
        assert!(is_secret_shaped("My key is AKIAIOSFODNN7EXAMPLE for S3"));
        assert!(is_secret_shaped("AKIA1234567890ABCDEF"));
    }

    #[test]
    fn detects_sk_api_keys() {
        assert!(is_secret_shaped("sk-1234567890abcdef1234567890abcdef"));
        assert!(is_secret_shaped(
            "sk-ant-api03-abcdef12345678901234567890_abc-123"
        ));
        assert!(is_secret_shaped(
            "export OPENAI_API_KEY=sk-proj-1234567890abcdef1234567890"
        ));
    }

    #[test]
    fn detects_github_tokens() {
        assert!(is_secret_shaped("ghp_123456789012345678901234567890123456"));
        assert!(is_secret_shaped("gho_123456789012345678901234567890123456"));
        assert!(is_secret_shaped("ghs_abcdef1234567890abcdef12345678901234"));
    }

    #[test]
    fn detects_slack_tokens() {
        assert!(is_secret_shaped(
            "xoxb-1234567890-123456789012-abcdef123456"
        ));
        assert!(is_secret_shaped(
            "xoxp-1234567890-123456789012-abcdef123456"
        ));
    }

    #[test]
    fn detects_jwt_shapes() {
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        assert!(is_secret_shaped(jwt));
        assert!(is_secret_shaped(&format!("Authorization: Bearer {jwt}")));
    }

    #[test]
    fn detects_credential_assignments() {
        assert!(is_secret_shaped(
            "password = \"supersecretlongpassword123456\""
        ));
        assert!(is_secret_shaped(
            "api_key: abcdef0123456789abcdef0123456789"
        ));
        assert!(is_secret_shaped("secret:=A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6"));
        assert!(is_secret_shaped(
            "token: 4f3c2b1a0987654321fedcba0123456789abcdef"
        ));
    }

    #[test]
    fn detects_long_hex_runs() {
        let hex32 = "4f3c2b1a0987654321fedcba01234567";
        assert!(is_secret_shaped(hex32));
        let hex64 = "4f3c2b1a0987654321fedcba0123456789abcdef4f3c2b1a0987654321fedcba";
        assert!(is_secret_shaped(hex64));
    }

    #[test]
    fn detects_base64_strings() {
        let b64 = "dGhpcyBpcyBhIHZlcnkgbG9uZyBiYXNlNjQgc3RyaW5nIHdpdGggcGFkZGluZz09";
        assert!(is_secret_shaped(b64));
    }

    #[test]
    fn accepts_benign_prose_and_code() {
        assert!(!is_secret_shaped("The user prefers tabs over spaces."));
        assert!(!is_secret_shaped("Favorite color is blue."));
        assert!(!is_secret_shaped(
            "Project root is D:\\Projects\\GitProjects\\Altior"
        ));
        assert!(!is_secret_shaped(
            "I am implementing work item P2.1 for Altior."
        ));
        assert!(!is_secret_shaped(
            "C++ standard library std::vector<int> capacity"
        ));
        assert!(!is_secret_shaped("My birthday is January 15, 1990."));
        assert!(!is_secret_shaped("Short token: 1234"));
        assert!(!is_secret_shaped(""));
        assert!(!is_secret_shaped("   "));
    }
}
