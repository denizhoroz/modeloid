//! Input rules checked by forms in the browser and again by the core.

/// Dataset names become folder names, so they follow folder-name rules.
pub fn validate_dataset_name(name: &str) -> Result<(), String> {
    if name.trim() != name {
        return Err("The name can't start or end with spaces.".into());
    }
    if name.is_empty() {
        return Err("Enter a name.".into());
    }
    if name.chars().count() > 100 {
        return Err("Keep the name under 100 characters.".into());
    }
    if let Some(c) = name.chars().find(|c| r#"<>:"/\|?*"#.contains(*c) || c.is_control()) {
        return Err(format!("The name can't contain '{c}', it's used as a folder name."));
    }
    if name.ends_with('.') {
        return Err("The name can't end with a dot.".into());
    }
    Ok(())
}

/// Checks the shape of an API key or token before saving it.
pub fn validate_api_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Paste your API key.".into());
    }
    if key.chars().any(char::is_whitespace) {
        return Err("The key can't contain spaces.".into());
    }
    if !(8..=128).contains(&key.len()) {
        return Err("That doesn't look like an API key. Check you copied all of it.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_dataset_names() {
        assert!(validate_dataset_name("cats-v2").is_ok());
        assert!(validate_dataset_name("").is_err());
        assert!(validate_dataset_name(" cats").is_err());
        assert!(validate_dataset_name("a/b").is_err());
        assert!(validate_dataset_name("a:b").is_err());
        assert!(validate_dataset_name("..").is_err());
    }

    #[test]
    fn validates_keys() {
        assert!(validate_api_key("abcDEF1234567890").is_ok());
        assert!(validate_api_key("  abcDEF1234567890  ").is_ok());
        assert!(validate_api_key("").is_err());
        assert!(validate_api_key("abc def 1234").is_err());
        assert!(validate_api_key("short").is_err());
    }
}
