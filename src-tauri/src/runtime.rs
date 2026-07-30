use crate::protocol::{CadDiagnostics, CadParameter, CadParameterValue};

pub const DEFAULT_SAMPLE_SOURCE: &str = r#"width = 32; // @param min=8 max=80 step=1 label=Width
depth = 24; // @param min=8 max=80 step=1 label=Depth
height = 12; // @param min=4 max=60 step=1 label=Height

cube([width, depth, height]);
translate([24, 0, height]) cylinder(h=height * 2, r=6);
translate([-24, 0, 12]) sphere(r=8);
"#;

pub fn extract_open_scad_parameters(source: &str) -> Vec<CadParameter> {
    source
        .lines()
        .filter_map(|line| {
            let (assignment, comment) = line.split_once("// @param")?;
            let (name, value) = assignment.split_once('=')?;
            let literal = value.trim().trim_end_matches(';').trim();
            let mut parameter = CadParameter {
                name: name.trim().to_string(),
                value: parse_parameter_value(literal),
                parameter_type: parameter_type(literal).to_string(),
                min: None,
                max: None,
                step: None,
                label: None,
            };
            let tokens = split_param_tokens(comment.trim());
            let mut label_parts = Vec::new();
            let mut reading_label = false;
            for token in tokens {
                if let Some((key, value)) = token.split_once('=') {
                    reading_label = key == "label";
                    match key {
                        "min" => parameter.min = value.parse().ok(),
                        "max" => parameter.max = value.parse().ok(),
                        "step" => parameter.step = value.parse().ok(),
                        "label" => label_parts.push(value.to_string()),
                        _ => {}
                    }
                } else if reading_label {
                    label_parts.push(token.to_string());
                }
            }
            if !label_parts.is_empty() {
                parameter.label = Some(label_parts.join(" "));
            }
            Some(parameter)
        })
        .collect()
}

pub fn ok_diagnostics(elapsed_ms: u64) -> CadDiagnostics {
    CadDiagnostics {
        ok: true,
        elapsed_ms,
        items: Vec::new(),
    }
}

fn split_param_tokens(input: &str) -> Vec<&str> {
    input.split_whitespace().collect()
}

fn parse_parameter_value(literal: &str) -> CadParameterValue {
    if literal == "true" {
        return CadParameterValue::Boolean(true);
    }
    if literal == "false" {
        return CadParameterValue::Boolean(false);
    }
    if let Ok(number) = literal.parse::<f64>() {
        return CadParameterValue::Number(number);
    }
    CadParameterValue::String(literal.trim_matches('"').to_string())
}

fn parameter_type(literal: &str) -> &'static str {
    if literal == "true" || literal == "false" {
        "boolean"
    } else if literal.parse::<f64>().is_ok() {
        "number"
    } else {
        "string"
    }
}
