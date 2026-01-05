pub fn normalize_web_url(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if trimmed.contains("://")
        || trimmed.starts_with("about:")
        || trimmed.starts_with("file:")
        || trimmed.starts_with("data:")
    {
        return trimmed.to_string();
    }

    format!("https://{trimmed}")
}
