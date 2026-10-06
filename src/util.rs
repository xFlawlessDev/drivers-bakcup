//! Shared helpers: CSV escaping, date formatting, folder-name sanitizing, and
//! Microsoft-driver filtering.

/// Escape a value for inclusion in a CSV field.
///
/// Wraps the value in double quotes and doubles any embedded quotes when it
/// contains a comma, quote, or newline.
pub fn escape_csv(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Format a WMI driver date (`YYYYMMDD...`) as `YYYY-MM-DD`.
///
/// Returns the original string when it is not a recognizable date, or
/// `"Unknown"` when absent.
pub fn format_driver_date(date: Option<&str>) -> String {
    match date {
        Some(date_str)
            if date_str.len() >= 8 && date_str[..8].chars().all(|c| c.is_ascii_digit()) =>
        {
            let year = &date_str[0..4];
            let month = &date_str[4..6];
            let day = &date_str[6..8];
            match (month.parse::<u32>(), day.parse::<u32>()) {
                (Ok(m), Ok(d)) if (1..=12).contains(&m) && (1..=31).contains(&d) => {
                    format!("{}-{}-{}", year, month, day)
                }
                _ => date_str.to_string(),
            }
        }
        Some(date_str) => date_str.to_string(),
        None => "Unknown".to_string(),
    }
}

/// Sanitize a folder name, keeping only characters safe for Windows paths.
pub fn sanitize_folder_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric()
                || c == ' '
                || c == '.'
                || c == '-'
                || c == '_'
                || c == '('
                || c == ')'
            {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Sanitize a device-class name for use as a folder. Class names are simpler
/// than product names, so fewer characters are allowed.
pub fn sanitize_class_folder_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// True when a provider name indicates a Microsoft-supplied driver.
pub fn is_microsoft_provider(provider: Option<&str>) -> bool {
    provider
        .map(|p| p.to_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

/// Strip a trailing `.inf` extension (case-insensitive) from a file name.
pub fn strip_inf_extension(name: &str) -> &str {
    if name.len() >= 4 {
        let (stem, ext) = name.split_at(name.len() - 4);
        if ext.eq_ignore_ascii_case(".inf") {
            return stem;
        }
    }
    name
}

/// Return an error when a path contains sequences that are unsafe to pass to
/// `pnputil` or to build on disk.
pub fn is_unsafe_path(path: &str) -> bool {
    path.contains("..") || path.contains('%')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_csv_plain() {
        assert_eq!(escape_csv("abc"), "abc");
    }

    #[test]
    fn escape_csv_with_comma() {
        assert_eq!(escape_csv("a,b"), "\"a,b\"");
    }

    #[test]
    fn escape_csv_with_quotes() {
        assert_eq!(escape_csv("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn format_driver_date_valid() {
        assert_eq!(format_driver_date(Some("20230615")), "2023-06-15");
        assert_eq!(
            format_driver_date(Some("20230615000000.000000+000")),
            "2023-06-15"
        );
    }

    #[test]
    fn format_driver_date_invalid_month_day() {
        assert_eq!(format_driver_date(Some("20231399")), "20231399");
    }

    #[test]
    fn format_driver_date_unknown() {
        assert_eq!(format_driver_date(None), "Unknown");
        assert_eq!(format_driver_date(Some("not-a-date")), "not-a-date");
    }

    #[test]
    fn sanitize_names() {
        assert_eq!(
            sanitize_folder_name("NVIDIA GeForce RTX 3080"),
            "NVIDIA GeForce RTX 3080"
        );
        assert_eq!(sanitize_folder_name("a/b:c"), "a_b_c");
        assert_eq!(sanitize_class_folder_name("HIDClass"), "HIDClass");
        assert_eq!(sanitize_class_folder_name("Audio/Video"), "Audio_Video");
    }

    #[test]
    fn microsoft_provider_detection() {
        assert!(is_microsoft_provider(Some("Microsoft")));
        assert!(is_microsoft_provider(Some("Microsoft Corporation")));
        assert!(!is_microsoft_provider(Some("Intel Corporation")));
        assert!(!is_microsoft_provider(None));
    }

    #[test]
    fn strip_inf() {
        assert_eq!(strip_inf_extension("intcdaud.inf"), "intcdaud");
        assert_eq!(strip_inf_extension("IntcDAud.INF"), "IntcDAud");
        assert_eq!(strip_inf_extension("noext"), "noext");
    }

    #[test]
    fn unsafe_paths() {
        assert!(is_unsafe_path("C:\\a\\..\\b"));
        assert!(is_unsafe_path("C:\\a%20b"));
        assert!(!is_unsafe_path("C:\\Drivers\\Display"));
    }
}
