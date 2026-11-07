fn strip_comments(s: &str) -> String {
    let mut depth = 0u32;
    s.chars()
        .filter(|c| match c {
            '(' => {
                depth += 1;
                false
            }
            ')' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ => depth == 0,
        })
        .collect()
}

fn authserv_id(cleaned: &str) -> Option<&str> {
    cleaned.split(';').next()?.split_whitespace().next()
}

fn method_pass(cleaned: &str, method: &str) -> bool {
    let expected = format!("{method}=pass");
    cleaned.split(';').skip(1).any(|clause| {
        clause
            .split_whitespace()
            .next()
            .is_some_and(|token| token.to_ascii_lowercase() == expected)
    })
}

pub(crate) fn trusted_pass(header: Option<&str>, authserv_id_expected: &str) -> bool {
    let Some(header) = header else {
        return false;
    };
    let cleaned = strip_comments(header);
    authserv_id(&cleaned) == Some(authserv_id_expected)
        && method_pass(&cleaned, "dkim")
        && method_pass(&cleaned, "dmarc")
}
