use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn render_strict_template(
    template_name: &str,
    template: &str,
    allowed_placeholders: &[&str],
    provided_values: Vec<(&str, String)>,
) -> Result<String, String> {
    let mut allowed = BTreeSet::new();
    for placeholder in allowed_placeholders {
        validate_placeholder_name(template_name, placeholder)?;
        if !allowed.insert(*placeholder) {
            return Err(format!(
                "Prompt template {template_name} declares duplicate placeholder {placeholder}."
            ));
        }
    }

    let mut values = BTreeMap::new();
    for (placeholder, value) in provided_values {
        validate_placeholder_name(template_name, placeholder)?;
        if !allowed.contains(placeholder) {
            return Err(format!(
                "Prompt renderer for {template_name} received unknown placeholder {placeholder}."
            ));
        }
        if values.insert(placeholder, value).is_some() {
            return Err(format!(
                "Prompt renderer for {template_name} received duplicate placeholder {placeholder}."
            ));
        }
    }

    let missing_values = allowed
        .iter()
        .filter(|placeholder| !values.contains_key(**placeholder))
        .copied()
        .collect::<Vec<_>>();
    if !missing_values.is_empty() {
        return Err(format!(
            "Prompt renderer for {template_name} is missing placeholder values: {}.",
            missing_values.join(", ")
        ));
    }

    let occurrences = placeholder_occurrences(template_name, template)?;
    for placeholder in occurrences.keys() {
        if !allowed.contains(placeholder.as_str()) {
            return Err(format!(
                "Prompt template {template_name} contains unknown placeholder {placeholder}."
            ));
        }
    }
    for placeholder in &allowed {
        match occurrences.get(*placeholder).copied() {
            None => {
                return Err(format!(
                    "Prompt template {template_name} is missing declared placeholder {placeholder}."
                ))
            }
            Some(1) => {}
            Some(count) => {
                return Err(format!(
                    "Prompt template {template_name} contains placeholder {placeholder} {count} times; expected exactly once."
                ))
            }
        }
    }

    let mut rendered = String::with_capacity(template.len());
    let mut cursor = 0;
    while let Some(relative_start) = template[cursor..].find("{{") {
        let start = cursor + relative_start;
        let name_start = start + 2;
        let relative_end = template[name_start..].find("}}").ok_or_else(|| {
            format!("Prompt template {template_name} has an unclosed placeholder.")
        })?;
        let end = name_start + relative_end;
        let placeholder = &template[name_start..end];
        rendered.push_str(&template[cursor..start]);
        rendered.push_str(values.get(placeholder).ok_or_else(|| {
            format!("Prompt renderer for {template_name} has no value for {placeholder}.")
        })?);
        cursor = end + 2;
    }
    rendered.push_str(&template[cursor..]);

    Ok(rendered)
}

fn placeholder_occurrences(
    template_name: &str,
    template: &str,
) -> Result<BTreeMap<String, usize>, String> {
    let mut occurrences = BTreeMap::new();
    let mut cursor = 0;
    while cursor < template.len() {
        let next_open = template[cursor..].find("{{").map(|offset| cursor + offset);
        let next_close = template[cursor..].find("}}").map(|offset| cursor + offset);
        if next_close.is_some_and(|close| next_open.is_none_or(|open| close < open)) {
            return Err(format!(
                "Prompt template {template_name} contains an unmatched closing placeholder delimiter."
            ));
        }
        let Some(start) = next_open else {
            break;
        };
        let name_start = start + 2;
        let relative_end = template[name_start..].find("}}").ok_or_else(|| {
            format!("Prompt template {template_name} has an unclosed placeholder.")
        })?;
        let end = name_start + relative_end;
        let placeholder = &template[name_start..end];
        validate_placeholder_name(template_name, placeholder)?;
        *occurrences.entry(placeholder.to_string()).or_insert(0) += 1;
        cursor = end + 2;
    }
    Ok(occurrences)
}

fn validate_placeholder_name(template_name: &str, placeholder: &str) -> Result<(), String> {
    if placeholder.is_empty()
        || !placeholder
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(format!(
            "Prompt template {template_name} has invalid placeholder name {placeholder:?}."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::render_strict_template;

    #[test]
    fn strict_renderer_rejects_missing_unknown_and_duplicate_values() {
        let missing = render_strict_template("test", "{{A}}", &["A"], vec![]).unwrap_err();
        assert!(missing.contains("missing placeholder values: A"));

        let unknown = render_strict_template(
            "test",
            "{{A}}",
            &["A"],
            vec![("A", "a".into()), ("B", "b".into())],
        )
        .unwrap_err();
        assert!(unknown.contains("unknown placeholder B"));

        let duplicate = render_strict_template(
            "test",
            "{{A}}",
            &["A"],
            vec![("A", "a".into()), ("A", "again".into())],
        )
        .unwrap_err();
        assert!(duplicate.contains("duplicate placeholder A"));
    }

    #[test]
    fn strict_renderer_rejects_template_contract_drift() {
        let unknown =
            render_strict_template("test", "{{A}} {{B}}", &["A"], vec![("A", "a".into())])
                .unwrap_err();
        assert!(unknown.contains("unknown placeholder B"));

        let absent =
            render_strict_template("test", "literal", &["A"], vec![("A", "a".into())]).unwrap_err();
        assert!(absent.contains("missing declared placeholder A"));

        let duplicate =
            render_strict_template("test", "{{A}} {{A}}", &["A"], vec![("A", "a".into())])
                .unwrap_err();
        assert!(duplicate.contains("2 times; expected exactly once"));
    }

    #[test]
    fn inserted_values_are_not_reinterpreted_as_placeholders() {
        let rendered = render_strict_template(
            "test",
            "value={{A}}",
            &["A"],
            vec![("A", "literal {{USER_TEXT}}".into())],
        )
        .unwrap();
        assert_eq!(rendered, "value=literal {{USER_TEXT}}");
    }
}
